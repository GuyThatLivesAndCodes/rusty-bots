use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

fn log(app: &AppHandle, bot_id: &str, level: &str, msg: impl Into<String>) {
    let msg = msg.into();
    eprintln!("[voice][{bot_id}][{level}] {msg}");
    tracing::info!(target: "voice", bot_id, level, "{msg}");
    let _ = app.emit("bot-log", serde_json::json!({
        "bot_id": bot_id, "level": level, "msg": msg,
    }));
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

/// Receives decoded Discord audio (mono 48kHz i16) and forwards PCM16 bytes to xAI.
struct Receiver {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    app: AppHandle,
    bot_id: String,
    heard: Arc<AtomicBool>,
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::VoiceTick(tick) = ctx {
            for (_ssrc, data) in tick.speaking.iter() {
                if let Some(pcm) = &data.decoded_voice {
                    if pcm.is_empty() { continue; }
                    if !self.heard.swap(true, Ordering::Relaxed) {
                        log(&self.app, &self.bot_id, "info", "voice: capturing audio from users");
                    }
                    let mut bytes = Vec::with_capacity(pcm.len() * 2);
                    for s in pcm { bytes.extend_from_slice(&s.to_le_bytes()); }
                    let _ = self.tx.send(bytes);
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
    log(&app, &bot_id, "info", format!("voice: join requested for channel {channel_id}"));
    if !cfg.voice_enabled {
        log(&app, &bot_id, "error", "voice: disabled for this bot");
        return Err(anyhow!("voice is disabled for this bot"));
    }
    if cfg.xai_api_key.is_empty() {
        log(&app, &bot_id, "error", "voice: xAI API key required");
        return Err(anyhow!("xAI API key required for voice"));
    }
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => {
            log(&app, &bot_id, "error", "voice: songbird not initialized");
            return Err(anyhow!("songbird not initialized"));
        }
    };

    // If a previous attempt left the bot half-connected, Discord thinks it is
    // still in the channel and every retry times out. Always clear any stale
    // connection on this guild before joining.
    let _ = manager.remove(guild_id).await;

    log(&app, &bot_id, "info", "voice: connecting to Discord voice (DAVE handshake)…");
    let join_fut = manager.join(guild_id, channel_id);
    let joined = tokio::time::timeout(std::time::Duration::from_secs(25), join_fut).await;
    let call_lock = match joined {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            // songbird hides the real cause behind "establishing connection
            // failed"; surface the inner ConnectionError, and clean up so the
            // bot does not get stuck in the channel.
            let detail = match &e {
                songbird::error::JoinError::Driver(inner) => format!("{inner:?}"),
                other => format!("{other}"),
            };
            let _ = manager.remove(guild_id).await;
            log(&app, &bot_id, "error", format!("voice join failed: {detail}"));
            return Err(anyhow!("join failed: {detail}"));
        }
        Err(_) => {
            let _ = manager.remove(guild_id).await;
            log(&app, &bot_id, "error", "voice: join timed out after 25s (no Discord voice connection)");
            return Err(anyhow!("join timed out"));
        }
    };

    log(&app, &bot_id, "info", format!("voice: joined channel {channel_id}, setting up audio bridge"));

    let done = Arc::new(AtomicBool::new(false));
    let playback_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

    // audio_in: Discord -> xAI
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    {
        let mut call = call_lock.lock().await;
        call.add_global_event(CoreEvent::VoiceTick.into(), Receiver {
            tx: in_tx.clone(),
            app: app.clone(),
            bot_id: bot_id.clone(),
            heard: Arc::new(AtomicBool::new(false)),
        });

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
    let (ws_stream, _) = match tokio_tungstenite::connect_async(req).await {
        Ok(s) => s,
        Err(e) => {
            log(&app, &bot_id, "error", format!("voice: xAI realtime connect failed: {e}"));
            let _ = manager.remove(guild_id).await;
            return Err(anyhow!("xAI realtime connect failed: {e}"));
        }
    };
    log(&app, &bot_id, "info", "voice: connected to xAI realtime API");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Configure the session with the persona.
    let session_cfg = serde_json::json!({
        "type": "session.update",
        "session": {
            "voice": cfg.voice,
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
    if let Err(e) = ws_write.send(WsMessage::Text(session_cfg.to_string())).await {
        log(&app, &bot_id, "error", format!("voice: session.update failed: {e}"));
        let _ = manager.remove(guild_id).await;
        return Err(anyhow!("session.update failed: {e}"));
    }
    log(&app, &bot_id, "info", format!("voice: session configured (voice='{}')", cfg.voice));

    // Task: forward Discord audio to xAI.
    let done_w = done.clone();
    let app_w = app.clone();
    let bot_w = bot_id.clone();
    let sent = Arc::new(AtomicU64::new(0));
    let sent_w = sent.clone();
    let writer = tokio::spawn(async move {
        let b64 = base64::engine::general_purpose::STANDARD;
        while let Some(pcm) = in_rx.recv().await {
            if done_w.load(Ordering::Relaxed) { break; }
            let msg = serde_json::json!({
                "type": "input_audio_buffer.append",
                "audio": b64.encode(&pcm),
            });
            if let Err(e) = ws_write.send(WsMessage::Text(msg.to_string())).await {
                log(&app_w, &bot_w, "error", format!("voice: failed sending audio to xAI: {e}"));
                break;
            }
            let n = sent_w.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 { log(&app_w, &bot_w, "info", "voice: streaming first audio to xAI"); }
            else if n % 250 == 0 { log(&app_w, &bot_w, "info", format!("voice: sent {n} audio frames to xAI")); }
        }
        let _ = ws_write.close().await;
    });

    // Task: read xAI events, push output audio to playback buffer.
    let done_r = done.clone();
    let app_r = app.clone();
    let bot_r = bot_id.clone();
    let reader = tokio::spawn(async move {
        let b64 = base64::engine::general_purpose::STANDARD;
        let mut got_audio = 0u64;
        while let Some(msg) = ws_read.next().await {
            if done_r.load(Ordering::Relaxed) { break; }
            let txt = match msg {
                Ok(WsMessage::Text(t)) => t,
                Ok(WsMessage::Close(c)) => {
                    log(&app_r, &bot_r, "error", format!("voice: xAI closed connection: {c:?}"));
                    break;
                }
                Ok(_) => continue,
                Err(e) => { log(&app_r, &bot_r, "error", format!("voice: xAI ws error: {e}")); break; }
            };
            let v: serde_json::Value = match serde_json::from_str(&txt) {
                Ok(v) => v, Err(_) => continue,
            };
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match ty {
                // Accept both documented and OpenAI-compatible audio event names.
                "response.output_audio.delta" | "response.audio.delta" => {
                    if let Some(a) = v.get("audio").or_else(|| v.get("delta")).and_then(|a| a.as_str()) {
                        if let Ok(bytes) = b64.decode(a) {
                            got_audio += 1;
                            if got_audio == 1 { log(&app_r, &bot_r, "info", "voice: receiving audio from xAI (speaking)"); }
                            playback_buf.lock().extend(bytes);
                        }
                    }
                }
                "session.created" | "session.updated" => {
                    log(&app_r, &bot_r, "info", format!("voice: {ty}"));
                }
                "response.done" | "response.completed" => {
                    got_audio = 0;
                }
                "error" => {
                    log(&app_r, &bot_r, "error", format!("voice: xAI error event: {v}"));
                }
                other if !other.is_empty() => {
                    // Surface anything unexpected so we can diagnose.
                    if !other.starts_with("response.output_audio.")
                        && !other.starts_with("input_audio_buffer.")
                        && other != "response.output_text.delta" {
                        log(&app_r, &bot_r, "info", format!("voice: event {other}"));
                    }
                }
                _ => {}
            }
        }
        log(&app_r, &bot_r, "info", "voice: xAI read loop ended");
    });

    sessions().lock().insert(guild_id.get(), VoiceSession {
        tasks: vec![writer, reader],
        done,
    });

    log(&app, &bot_id, "info", format!("voice: bridge live in channel {channel_id}"));
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
