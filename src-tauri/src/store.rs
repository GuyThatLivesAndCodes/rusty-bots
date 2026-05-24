use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use parking_lot::Mutex;
use anyhow::Result;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub id: String,
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub xai_api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub persona: String,
    #[serde(default = "default_true")]
    pub ai_enabled: bool,
    #[serde(default = "default_history")]
    pub history_size: usize,
    #[serde(default)]
    pub automod: AutoModConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoModConfig {
    pub enabled: bool,
    pub rules: Vec<AutoModRule>,
    pub ignored_channels: Vec<String>,
    pub ignored_roles: Vec<String>,
    pub whitelist_users: Vec<String>,
    pub log_channel: Option<String>,
    #[serde(default)]
    pub advanced_detection: AdvancedDetectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedDetectionConfig {
    pub enable_spaced_variant: bool,
    pub enable_special_char_variant: bool,
    pub enable_acronym_detection: bool,
    pub enable_cross_message_detection: bool,
    pub cross_message_window_secs: u64,
}

impl Default for AdvancedDetectionConfig {
    fn default() -> Self {
        Self {
            enable_spaced_variant: true,
            enable_special_char_variant: true,
            enable_acronym_detection: true,
            enable_cross_message_detection: true,
            cross_message_window_secs: 60,
        }
    }
}

impl Default for AutoModConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules: vec![],
            ignored_channels: vec![],
            ignored_roles: vec![],
            whitelist_users: vec![],
            log_channel: None,
            advanced_detection: AdvancedDetectionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AutoModRule {
    SpamDetection {
        #[serde(default = "default_spam_threshold")]
        message_threshold: u32,
        #[serde(default = "default_spam_window")]
        time_window_secs: u64,
        action: AutoModAction,
    },
    BadWords {
        words: Vec<String>,
        action: AutoModAction,
    },
    Caps {
        #[serde(default = "default_caps_threshold")]
        threshold_percent: f32,
        action: AutoModAction,
    },
    MentionSpam {
        #[serde(default = "default_mention_threshold")]
        mention_threshold: u32,
        action: AutoModAction,
    },
    LinkFilter {
        allowed_domains: Vec<String>,
        action: AutoModAction,
    },
    InviteFilter {
        action: AutoModAction,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AutoModAction {
    Delete,
    Warn { max_warnings: u32 },
    Timeout { duration_secs: u64 },
    Kick,
    Ban,
    SendMessage { message: String },
}

fn default_model() -> String { "grok-4.3".to_string() }
fn default_true() -> bool { true }
fn default_history() -> usize { 10 }
fn default_spam_threshold() -> u32 { 5 }
fn default_spam_window() -> u64 { 10 }
fn default_caps_threshold() -> f32 { 70.0 }
fn default_mention_threshold() -> u32 { 5 }

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub bots: Vec<BotConfig>,
}

pub struct StoreHandle {
    path: PathBuf,
    inner: Mutex<Store>,
}

impl StoreHandle {
    pub fn load(path: PathBuf) -> Result<Self> {
        let inner = if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            serde_json::from_str(&s).unwrap_or_default()
        } else {
            Store::default()
        };
        Ok(Self { path, inner: Mutex::new(inner) })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(p) = self.path.parent() { std::fs::create_dir_all(p)?; }
        let s = serde_json::to_string_pretty(&*self.inner.lock())?;
        std::fs::write(&self.path, s)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<BotConfig> {
        self.inner.lock().bots.clone()
    }

    pub fn get(&self, id: &str) -> Option<BotConfig> {
        self.inner.lock().bots.iter().find(|b| b.id == id).cloned()
    }

    pub fn upsert(&self, bot: BotConfig) -> Result<()> {
        {
            let mut g = self.inner.lock();
            if let Some(existing) = g.bots.iter_mut().find(|b| b.id == bot.id) {
                *existing = bot;
            } else {
                g.bots.push(bot);
            }
        }
        self.save()
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        {
            let mut g = self.inner.lock();
            g.bots.retain(|b| b.id != id);
        }
        self.save()
    }
}
