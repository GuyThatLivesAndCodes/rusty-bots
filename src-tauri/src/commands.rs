use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serenity::all::*;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::store::{BotConfig, StoreHandle, AutoModConfig, AutoModRule, AutoModAction};
use crate::bot::{BotRegistry, start_bot};

pub struct AppState {
    pub store: Arc<StoreHandle>,
    pub registry: Arc<BotRegistry>,
}

#[derive(Serialize)]
pub struct BotView {
    #[serde(flatten)]
    pub config: BotConfig,
    pub running: bool,
}

#[tauri::command]
pub fn list_bots(state: State<'_, AppState>) -> Vec<BotView> {
    state.store.list().into_iter().map(|c| {
        let running = state.registry.is_running(&c.id);
        BotView { config: c, running }
    }).collect()
}

#[derive(Deserialize)]
pub struct UpsertBotInput {
    pub id: Option<String>,
    pub name: String,
    pub token: String,
    pub xai_api_key: Option<String>,
    pub model: Option<String>,
    pub persona: Option<String>,
    pub ai_enabled: Option<bool>,
    pub history_size: Option<usize>,
    pub automod: Option<AutoModConfig>,
}

#[tauri::command]
pub fn save_bot(state: State<'_, AppState>, input: UpsertBotInput) -> Result<BotConfig, String> {
    let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing = state.store.get(&id);
    let cfg = BotConfig {
        id,
        name: input.name,
        token: input.token,
        xai_api_key: input.xai_api_key.unwrap_or_else(|| existing.as_ref().map(|e| e.xai_api_key.clone()).unwrap_or_default()),
        model: input.model.unwrap_or_else(|| existing.as_ref().map(|e| e.model.clone()).unwrap_or_else(|| "grok-4.3".to_string())),
        persona: input.persona.unwrap_or_else(|| existing.as_ref().map(|e| e.persona.clone()).unwrap_or_default()),
        ai_enabled: input.ai_enabled.unwrap_or_else(|| existing.as_ref().map(|e| e.ai_enabled).unwrap_or(true)),
        history_size: input.history_size.unwrap_or_else(|| existing.as_ref().map(|e| e.history_size).unwrap_or(10)),
        automod: input.automod.unwrap_or_else(|| existing.as_ref().map(|e| e.automod.clone()).unwrap_or_default()),
    };
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    // Sync config to running bot if it's online
    if let Some(rb) = state.registry.running.lock().get(&cfg.id) {
        *rb.config.lock() = cfg.clone();
    }

    Ok(cfg)
}

#[tauri::command]
pub async fn delete_bot(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.registry.stop(&id).await.map_err(|e| e.to_string())?;
    state.store.remove(&id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn start_bot_cmd(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    start_bot(state.registry.clone(), app, cfg).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_bot_cmd(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.registry.stop(&id).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct GuildView {
    pub id: String,
    pub name: String,
    pub member_count: u64,
}

#[tauri::command]
pub fn list_guilds(state: State<'_, AppState>, id: String) -> Result<Vec<GuildView>, String> {
    let cache = state.registry.cache(&id).ok_or_else(|| "bot not running".to_string())?;
    let guilds: Vec<GuildView> = cache.guilds().iter().filter_map(|gid| {
        cache.guild(*gid).map(|g| GuildView {
            id: gid.to_string(),
            name: g.name.clone(),
            member_count: g.member_count,
        })
    }).collect();
    Ok(guilds)
}

#[derive(Serialize)]
pub struct MemberView {
    pub user_id: String,
    pub username: String,
    pub nick: Option<String>,
    pub roles: Vec<RoleView>,
}

#[derive(Serialize, Clone)]
pub struct RoleView {
    pub id: String,
    pub name: String,
}

#[tauri::command]
pub async fn list_guild_members(state: State<'_, AppState>, id: String, guild_id: String) -> Result<Vec<MemberView>, String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let guild = GuildId::new(gid);
    let members = guild.members(&http, Some(100), None).await.map_err(|e| e.to_string())?;
    let roles_map = guild.roles(&http).await.map_err(|e| e.to_string())?;
    let role_view = |rid: &RoleId| -> RoleView {
        let n = roles_map.get(rid).map(|r| r.name.clone()).unwrap_or_else(|| rid.to_string());
        RoleView { id: rid.to_string(), name: n }
    };
    Ok(members.into_iter().map(|m| MemberView {
        user_id: m.user.id.to_string(),
        username: m.user.name.clone(),
        nick: m.nick.clone(),
        roles: m.roles.iter().map(role_view).collect(),
    }).collect())
}

#[tauri::command]
pub async fn list_guild_roles(state: State<'_, AppState>, id: String, guild_id: String) -> Result<Vec<RoleView>, String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let roles = GuildId::new(gid).roles(&http).await.map_err(|e| e.to_string())?;
    Ok(roles.into_iter().map(|(rid, r)| RoleView { id: rid.to_string(), name: r.name }).collect())
}

#[tauri::command]
pub async fn kick(state: State<'_, AppState>, id: String, guild_id: String, user_id: String, reason: Option<String>) -> Result<(), String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let uid: u64 = user_id.parse().map_err(|_| "bad user id".to_string())?;
    GuildId::new(gid).kick_with_reason(&http, UserId::new(uid), reason.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ban(state: State<'_, AppState>, id: String, guild_id: String, user_id: String, reason: Option<String>) -> Result<(), String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let uid: u64 = user_id.parse().map_err(|_| "bad user id".to_string())?;
    GuildId::new(gid).ban_with_reason(&http, UserId::new(uid), 0, reason.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn timeout(state: State<'_, AppState>, id: String, guild_id: String, user_id: String, seconds: i64) -> Result<(), String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let uid: u64 = user_id.parse().map_err(|_| "bad user id".to_string())?;
    let until = chrono::Utc::now() + chrono::Duration::seconds(seconds);
    let ts = Timestamp::from_unix_timestamp(until.timestamp()).map_err(|e| e.to_string())?;
    GuildId::new(gid).edit_member(&http, UserId::new(uid), EditMember::new().disable_communication_until_datetime(ts)).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_role(state: State<'_, AppState>, id: String, guild_id: String, user_id: String, role_id: String) -> Result<(), String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let uid: u64 = user_id.parse().map_err(|_| "bad user id".to_string())?;
    let rid: u64 = role_id.parse().map_err(|_| "bad role id".to_string())?;
    http.add_member_role(GuildId::new(gid), UserId::new(uid), RoleId::new(rid), None).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_role(state: State<'_, AppState>, id: String, guild_id: String, user_id: String, role_id: String) -> Result<(), String> {
    let http = state.registry.http(&id).ok_or_else(|| "bot not running".to_string())?;
    let gid: u64 = guild_id.parse().map_err(|_| "bad guild id".to_string())?;
    let uid: u64 = user_id.parse().map_err(|_| "bad user id".to_string())?;
    let rid: u64 = role_id.parse().map_err(|_| "bad role id".to_string())?;
    http.remove_member_role(GuildId::new(gid), UserId::new(uid), RoleId::new(rid), None).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_automod_config(state: State<'_, AppState>, id: String) -> Result<AutoModConfig, String> {
    let cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    Ok(cfg.automod)
}

#[tauri::command]
pub fn update_automod_config(state: State<'_, AppState>, id: String, config: AutoModConfig) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod = config;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    // Sync config to running bot if it's online
    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn toggle_automod(state: State<'_, AppState>, id: String, enabled: bool) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.enabled = enabled;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    // Sync config to running bot if it's online
    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn add_automod_rule(state: State<'_, AppState>, id: String, rule: AutoModRule) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.rules.push(rule);
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn remove_automod_rule(state: State<'_, AppState>, id: String, rule_index: usize) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    if rule_index < cfg.automod.rules.len() {
        cfg.automod.rules.remove(rule_index);
    }
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn set_automod_ignored_channels(state: State<'_, AppState>, id: String, channels: Vec<String>) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.ignored_channels = channels;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn set_automod_ignored_roles(state: State<'_, AppState>, id: String, roles: Vec<String>) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.ignored_roles = roles;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn set_automod_whitelist(state: State<'_, AppState>, id: String, users: Vec<String>) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.whitelist_users = users;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}

#[tauri::command]
pub fn set_automod_log_channel(state: State<'_, AppState>, id: String, channel: Option<String>) -> Result<(), String> {
    let mut cfg = state.store.get(&id).ok_or_else(|| "bot not found".to_string())?;
    cfg.automod.log_channel = channel;
    state.store.upsert(cfg.clone()).map_err(|e| e.to_string())?;

    if let Some(rb) = state.registry.running.lock().get(&id) {
        *rb.config.lock() = cfg;
    }

    Ok(())
}
