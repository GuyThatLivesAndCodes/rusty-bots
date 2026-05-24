use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use parking_lot::Mutex;
use anyhow::Result;

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
    #[serde(default = "default_true")]
    pub voice_enabled: bool,
    #[serde(default = "default_voice")]
    pub voice: String,
}

fn default_voice() -> String { "eve".to_string() }

fn default_model() -> String { "grok-4.3".to_string() }
fn default_true() -> bool { true }
fn default_history() -> usize { 10 }

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
