use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use serenity::all::*;
use serenity::async_trait;
use serenity::prelude::*;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use anyhow::{Result, anyhow};

use crate::store::BotConfig;
use crate::ai::{self, ChatMessage};
use crate::automod::{self, AutoModState};

#[derive(Clone)]
pub struct BotRuntimeState {
    pub config: Arc<Mutex<BotConfig>>,
    pub app: AppHandle,
    pub automod: Arc<AutoModState>,
}

pub struct RunningBot {
    pub config: Arc<Mutex<BotConfig>>,
    pub shard_manager: Arc<ShardManager>,
    pub shutdown_tx: Option<oneshot::Sender<()>>,
    pub http: Arc<Http>,
    pub cache: Arc<Cache>,
    pub automod: Arc<AutoModState>,
}

#[derive(Default)]
pub struct BotRegistry {
    pub running: Mutex<HashMap<String, RunningBot>>,
}

impl BotRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().contains_key(id)
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        let maybe = self.running.lock().remove(id);
        if let Some(rb) = maybe {
            rb.shard_manager.shutdown_all().await;
            if let Some(tx) = rb.shutdown_tx { let _ = tx.send(()); }
        }
        Ok(())
    }

    pub fn http(&self, id: &str) -> Option<Arc<Http>> {
        self.running.lock().get(id).map(|r| r.http.clone())
    }

    pub fn cache(&self, id: &str) -> Option<Arc<Cache>> {
        self.running.lock().get(id).map(|r| r.cache.clone())
    }
}

struct Handler {
    state: BotRuntimeState,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        let id = self.state.config.lock().id.clone();
        let _ = self.state.app.emit("bot-log", serde_json::json!({
            "bot_id": id,
            "level": "info",
            "msg": format!("Connected as {} ({} guilds)", ready.user.name, ready.guilds.len()),
        }));
        let _ = self.state.app.emit("bot-status", serde_json::json!({
            "bot_id": id, "online": true, "username": ready.user.name,
        }));
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot { return; }

        let cfg = {
            let g = self.state.config.lock();
            g.clone()
        };

        // Run automod checks
        if let Ok(Some(violation)) = self.state.automod.check_message(&ctx, &cfg.automod, &msg).await {
            if let Err(e) = automod::execute_action(&ctx, &violation, &msg, &cfg.automod).await {
                tracing::warn!("automod action failed: {e}");
            }
            return;
        }

        let me = match ctx.cache.current_user().id == msg.author.id { _ => ctx.cache.current_user().id };
        // Only respond when mentioned.
        let mentioned = msg.mentions.iter().any(|u| u.id == me);
        if !mentioned { return; }

        if !cfg.ai_enabled { return; }

        let bot_id = cfg.id.clone();

        let _ = self.state.app.emit("bot-log", serde_json::json!({
            "bot_id": bot_id, "level": "info",
            "msg": format!("@mention in #{} from {}", msg.channel_id, msg.author.name),
        }));

        // Pull last N messages.
        let history = match msg.channel_id.messages(&ctx.http, GetMessages::new().before(msg.id).limit(cfg.history_size as u8)).await {
            Ok(m) => m,
            Err(e) => { tracing::warn!("history fetch fail: {e}"); vec![] }
        };

        let mut convo: Vec<ChatMessage> = Vec::new();
        let system = format!(
            "You are a Discord bot persona. Follow this persona/character strictly:\n\n{}\n\n\
             You were just mentioned. Use the available tools to respond. \
             Prefer `send_message` to reply. Only use moderation tools if the persona/context clearly justifies it. \
             Current channel id: {}. Current guild id: {}. The triggering message id is: {}. The triggering user id is: {}.",
            cfg.persona,
            msg.channel_id,
            msg.guild_id.map(|g| g.to_string()).unwrap_or_default(),
            msg.id,
            msg.author.id,
        );
        convo.push(ChatMessage { role: "system".into(), content: system });

        // History oldest-first
        for m in history.iter().rev() {
            let role = if m.author.id == me { "assistant" } else { "user" };
            convo.push(ChatMessage {
                role: role.into(),
                content: format!("{} (id={}): {}", m.author.name, m.author.id, m.content),
            });
        }
        convo.push(ChatMessage {
            role: "user".into(),
            content: format!("{} (id={}): {}", msg.author.name, msg.author.id, msg.content),
        });

        let resp = match ai::complete(&cfg.xai_api_key, &cfg.model, &convo).await {
            Ok(r) => r,
            Err(e) => {
                let _ = self.state.app.emit("bot-log", serde_json::json!({
                    "bot_id": bot_id, "level": "error", "msg": format!("AI error: {e}"),
                }));
                return;
            }
        };

        // Execute tool calls
        if let Some(calls) = resp.tool_calls {
            for call in calls {
                let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::json!({}));
                let result = execute_tool(&ctx, &msg, &call.function.name, &args).await;
                let _ = self.state.app.emit("bot-log", serde_json::json!({
                    "bot_id": bot_id, "level": "info",
                    "msg": format!("tool {} -> {:?}", call.function.name, result),
                }));
            }
        } else if let Some(text) = resp.content {
            if !text.trim().is_empty() {
                let _ = msg.channel_id.say(&ctx.http, text).await;
            }
        }
    }
}

async fn execute_tool(ctx: &Context, msg: &Message, name: &str, args: &serde_json::Value) -> Result<String> {
    let guild_id = msg.guild_id.ok_or_else(|| anyhow!("not in a guild"))?;
    match name {
        "send_message" => {
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if !content.is_empty() {
                msg.channel_id.say(&ctx.http, content).await?;
            }
            Ok("sent".into())
        }
        "react" => {
            let e = args.get("emoji").and_then(|v| v.as_str()).unwrap_or("👍");
            let r = ReactionType::Unicode(e.to_string());
            msg.react(&ctx.http, r).await?;
            Ok("reacted".into())
        }
        "kick_user" => {
            let uid: u64 = args.get("user_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad user_id"))?;
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            guild_id.kick_with_reason(&ctx.http, UserId::new(uid), reason).await?;
            Ok("kicked".into())
        }
        "ban_user" => {
            let uid: u64 = args.get("user_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad user_id"))?;
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            guild_id.ban_with_reason(&ctx.http, UserId::new(uid), 0, reason).await?;
            Ok("banned".into())
        }
        "timeout_user" => {
            let uid: u64 = args.get("user_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad user_id"))?;
            let secs = args.get("seconds").and_then(|v| v.as_i64()).unwrap_or(60);
            let until = chrono::Utc::now() + chrono::Duration::seconds(secs);
            let ts = Timestamp::from_unix_timestamp(until.timestamp()).map_err(|e| anyhow!("{e}"))?;
            guild_id.edit_member(&ctx.http, UserId::new(uid), EditMember::new().disable_communication_until_datetime(ts)).await?;
            Ok("timed out".into())
        }
        "add_role" => {
            let uid: u64 = args.get("user_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad user_id"))?;
            let rid: u64 = args.get("role_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad role_id"))?;
            ctx.http.add_member_role(guild_id, UserId::new(uid), RoleId::new(rid), None).await?;
            Ok("role added".into())
        }
        "remove_role" => {
            let uid: u64 = args.get("user_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad user_id"))?;
            let rid: u64 = args.get("role_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad role_id"))?;
            ctx.http.remove_member_role(guild_id, UserId::new(uid), RoleId::new(rid), None).await?;
            Ok("role removed".into())
        }
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

pub async fn start_bot(registry: Arc<BotRegistry>, app: AppHandle, cfg: BotConfig) -> Result<()> {
    if registry.is_running(&cfg.id) {
        return Err(anyhow!("bot already running"));
    }
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MEMBERS;

    let config_arc = Arc::new(Mutex::new(cfg.clone()));
    let automod_state = Arc::new(AutoModState::new());
    let runtime_state = BotRuntimeState {
        config: config_arc.clone(),
        app: app.clone(),
        automod: automod_state.clone(),
    };

    let mut client = Client::builder(&cfg.token, intents)
        .event_handler(Handler { state: runtime_state })
        .await?;

    let shard_manager = client.shard_manager.clone();
    let http = client.http.clone();
    let cache = client.cache.clone();
    let (tx, _rx) = oneshot::channel::<()>();

    registry.running.lock().insert(cfg.id.clone(), RunningBot {
        config: config_arc,
        shard_manager,
        shutdown_tx: Some(tx),
        http,
        cache,
        automod: automod_state,
    });

    let bot_id = cfg.id.clone();
    let registry_cl = registry.clone();
    let app_cl = app.clone();
    tokio::spawn(async move {
        if let Err(e) = client.start().await {
            let _ = app_cl.emit("bot-log", serde_json::json!({
                "bot_id": bot_id, "level": "error", "msg": format!("client error: {e}"),
            }));
        }
        registry_cl.running.lock().remove(&bot_id);
        let _ = app_cl.emit("bot-status", serde_json::json!({
            "bot_id": bot_id, "online": false,
        }));
    });

    Ok(())
}
