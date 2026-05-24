use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use parking_lot::Mutex;
use serenity::all::*;
use serenity::async_trait;
use songbird::input::{Input, RawAdapter};
use songbird::input::core::io::MediaSource;
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler, CoreEvent};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use base64::Engine;
use anyhow::{Result, anyhow};
use tauri::{AppHandle, Emitter};

use crate::store::BotConfig;

const REALTIME_URL: &str = "wss://api.x.ai/v1/realtime?model=grok-voice-latest";

/// Active voice sessions keyed by guild id.
struct VoiceSession {
    tasks: Vec<JoinHandle<()>>,
    done: Arc<AtomicBool>,
}

fn sessions() -> &'static Mutex<HashMap<u64, VoiceSession>> {
    static S: OnceLock<Mutex<HashMap<u64, VoiceSession>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn is_in_voice(guild_id: u64) -> bool {
    sessions().lock().contains_key(&guild_id)
}

/// Streaming PCM source that songbird plays. Returns silence when no data is
/// buffered so the track stays alive for the whole call.
struct StreamSource {
    buf: Arc<Mutex<VecDeque<u8>>>,
    done: Arc<AtomicBool>,
}

impl Read for StreamSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.done.load(Ordering::Relaxed) {
            return Ok(0);
        }
        let mut b = self.buf.lock();
        if b.is_empty() {
            // Emit silence to keep the live track running.
            for x in out.iter_mut() { *x = 0; }
            return Ok(out.len());
        }
        let n = out.len().min(b.len());
        for slot in out.iter_mut().take(n) { *slot = b.pop_front().unwrap(); }
        Ok(n)
    }
}

impl Seek for StreamSource {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> { Ok(0) }
}

impl MediaSource for StreamSource {
    fn is_seekable(&self) -> bool { false }
    fn byte_len(&self) -> Option<u64> { None }
}

/// Receives decoded Discord audio and forwards mono PCM16 to xAI.
struct Receiver {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::VoiceTick(tick) = ctx {
            for (_ssrc, data) in tick.speaking.iter() {
                if let Some(pcm) = &data.decoded_voice {
                    // pcm is 48kHz stereo interleaved i16; downmix to mono.
                    let mut mono = Vec::with_capacity(pcm.len() / 2 * 2);
                    let mut i = 0;
                    while i + 1 < pcm.len() {
                        let l = pcm[i] as i32;
                        let r = pcm[i + 1] as i32;
                        let m = ((l + r) / 2) as i16;
                        mono.extend_from_slice(&m.to_le_bytes());
                        i += 2;
                    }
                    if !mono.is_empty() {
                        let _ = self.tx.send(mono);
                    }
                }
            }
        }
        None
    }
}

pub async fn join_voice(
    ctx: &Context,
    app: AppHandle,
    bot_id: String,
    cfg: BotConfig,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<()> {
    if cfg.xai_api_key.is_empty() {
        return Err(anyhow!("xAI API key required for voice"));
    }
    let manager = songbird::get(ctx).await
        .ok_or_else(|| anyhow!("songbird not initialized"))?;

    let call_lock = manager.join(guild_id, channel_id).await
        .map_err(|e| anyhow!("join failed: {e}"))?;

    let done = Arc::new(AtomicBool::new(false));
    let playback_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

    // audio_in: Discord -> xAI
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    {
        let mut call = call_lock.lock().await;
        call.add_global_event(CoreEvent::VoiceTick.into(), Receiver { tx: in_tx.clone() });

        // Start playback of the streamed xAI audio (mono, 48kHz).
        let source = StreamSource { buf: playback_buf.clone(), done: done.clone() };
        let input: Input = RawAdapter::new(source, 48_000, 1).into();
        call.play_input(input);
    }

    // Connect to xAI realtime WebSocket.
    let mut req = REALTIME_URL.into_client_request()
        .map_err(|e| anyhow!("bad ws request: {e}"))?;
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", cfg.xai_api_key).parse().unwrap(),
    );
    let (ws_stream, _) = tokio_tungstenite::connect_async(req).await
        .map_err(|e| anyhow!("xAI realtime connect failed: {e}"))?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Configure the session with the persona.
    let session_cfg = serde_json::json!({
        "type": "session.update",
        "session": {
            "voice": "eve",
            "instructions": cfg.persona,
            "turn_detection": {
                "type": "server_vad",
                "threshold": 0.85,
                "silence_duration_ms": 500,
                "prefix_padding_ms": 333
            },
            "audio": {
                "input":  { "format": { "type": "audio/pcm", "rate": 48000 } },
                "output": { "format": { "type": "audio/pcm", "rate": 48000 } }
            },
            "tools": []
        }
    });
    ws_write.send(WsMessage::Text(session_cfg.to_string())).await
        .map_err(|e| anyhow!("session.update failed: {e}"))?;

    let b64 = base64::engine::general_purpose::STANDARD;

    // Task: forward Discord audio to xAI.
    let done_w = done.clone();
    let writer = tokio::spawn(async move {
        while let Some(pcm) = in_rx.recv().await {
            if done_w.load(Ordering::Relaxed) { break; }
            let msg = serde_json::json!({
                "type": "input_audio_buffer.append",
                "audio": b64.encode(&pcm),
            });
            if ws_write.send(WsMessage::Text(msg.to_string())).await.is_err() {
                break;
            }
        }
        let _ = ws_write.close().await;
    });

    // Task: read xAI output audio and push to playback buffer.
    let done_r = done.clone();
    let app_r = app.clone();
    let bot_r = bot_id.clone();
    let reader = tokio::spawn(async move {
        let b64 = base64::engine::general_purpose::STANDARD;
        while let Some(msg) = ws_read.next().await {
            if done_r.load(Ordering::Relaxed) { break; }
            let txt = match msg {
                Ok(WsMessage::Text(t)) => t,
                Ok(WsMessage::Close(_)) | Err(_) => break,
                _ => continue,
            };
            let v: serde_json::Value = match serde_json::from_str(&txt) {
                Ok(v) => v, Err(_) => continue,
            };
            match v.get("type").and_then(|t| t.as_str()) {
                Some("response.output_audio.delta") => {
                    if let Some(a) = v.get("audio").and_then(|a| a.as_str()) {
                        if let Ok(bytes) = b64.decode(a) {
                            playback_buf.lock().extend(bytes);
                        }
                    }
                }
                Some("error") => {
                    let _ = app_r.emit("bot-log", serde_json::json!({
                        "bot_id": bot_r, "level": "error",
                        "msg": format!("voice error: {}", v),
                    }));
                }
                _ => {}
            }
        }
    });

    sessions().lock().insert(guild_id.get(), VoiceSession {
        tasks: vec![writer, reader],
        done,
    });

    let _ = app.emit("bot-log", serde_json::json!({
        "bot_id": bot_id, "level": "info",
        "msg": format!("joined voice channel {}", channel_id),
    }));
    Ok(())
}

pub async fn leave_voice(ctx: &Context, guild_id: GuildId) -> Result<()> {
    if let Some(session) = sessions().lock().remove(&guild_id.get()) {
        session.done.store(true, Ordering::Relaxed);
        for t in &session.tasks { t.abort(); }
    }
    if let Some(manager) = songbird::get(ctx).await {
        let _ = manager.remove(guild_id).await;
    }
    Ok(())
}

/// Count non-bot members currently in the given voice channel.
pub fn non_bot_members_in_channel(ctx: &Context, guild_id: GuildId, channel_id: ChannelId) -> usize {
    let Some(guild) = ctx.cache.guild(guild_id) else { return 0; };
    guild.voice_states.values()
        .filter(|vs| vs.channel_id == Some(channel_id))
        .filter_map(|vs| guild.members.get(&vs.user_id))
        .filter(|m| !m.user.bot)
        .count()
}
