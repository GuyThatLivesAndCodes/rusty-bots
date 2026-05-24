use crate::store::{AutoModConfig, AutoModRule, AutoModAction};
use serenity::all::*;
use std::collections::HashMap;
use parking_lot::Mutex;
use chrono::{DateTime, Utc};
use anyhow::{Result, anyhow};
use regex::Regex;
use std::sync::OnceLock;

pub struct AutoModState {
    pub user_message_history: Mutex<HashMap<UserId, Vec<MessageTimestamp>>>,
}

#[derive(Clone)]
struct MessageTimestamp {
    time: DateTime<Utc>,
    mention_count: u32,
}

impl AutoModState {
    pub fn new() -> Self {
        Self {
            user_message_history: Mutex::new(HashMap::new()),
        }
    }

    pub async fn check_message(
        &self,
        ctx: &Context,
        config: &AutoModConfig,
        msg: &Message,
    ) -> Result<Option<AutoModViolation>> {
        if !config.enabled {
            return Ok(None);
        }

        // Skip whitelisted users
        if config.whitelist_users.contains(&msg.author.id.to_string()) {
            return Ok(None);
        }

        // Skip ignored channels
        if config.ignored_channels.contains(&msg.channel_id.to_string()) {
            return Ok(None);
        }

        // Check if user has ignored roles
        if let Some(guild_id) = msg.guild_id {
            let member = guild_id.member(&ctx.http, msg.author.id).await.ok();
            if let Some(m) = member {
                for role_id in &m.roles {
                    if config.ignored_roles.contains(&role_id.to_string()) {
                        return Ok(None);
                    }
                }
            }
        }

        let content_lower = msg.content.to_lowercase();

        // Check each rule
        for rule in &config.rules {
            if let Some(violation) = self.check_rule(ctx, rule, msg, &content_lower).await? {
                return Ok(Some(violation));
            }
        }

        Ok(None)
    }

    async fn check_rule(
        &self,
        ctx: &Context,
        rule: &AutoModRule,
        msg: &Message,
        content_lower: &str,
    ) -> Result<Option<AutoModViolation>> {
        match rule {
            AutoModRule::SpamDetection {
                message_threshold,
                time_window_secs,
                action,
            } => {
                let now = Utc::now();
                let mut history = self.user_message_history.lock();
                let timestamps = history.entry(msg.author.id).or_insert_with(Vec::new);

                // Remove old messages outside the time window
                timestamps.retain(|ts| {
                    (now - ts.time).num_seconds() < *time_window_secs as i64
                });

                timestamps.push(MessageTimestamp {
                    time: now,
                    mention_count: msg.mentions.len() as u32,
                });

                if timestamps.len() >= *message_threshold as usize {
                    return Ok(Some(AutoModViolation {
                        rule_type: "Spam Detection".to_string(),
                        action: action.clone(),
                        message_id: msg.id,
                        user_id: msg.author.id,
                    }));
                }
            }
            AutoModRule::BadWords { words, action } => {
                for word in words {
                    if content_lower.contains(&word.to_lowercase()) {
                        return Ok(Some(AutoModViolation {
                            rule_type: "Bad Words".to_string(),
                            action: action.clone(),
                            message_id: msg.id,
                            user_id: msg.author.id,
                        }));
                    }
                }
            }
            AutoModRule::Caps {
                threshold_percent,
                action,
            } => {
                if msg.content.len() > 5 {
                    let uppercase_count = msg.content.chars().filter(|c| c.is_uppercase()).count();
                    let letter_count = msg.content.chars().filter(|c| c.is_alphabetic()).count();
                    if letter_count > 0 {
                        let caps_percent = (uppercase_count as f32 / letter_count as f32) * 100.0;
                        if caps_percent >= *threshold_percent {
                            return Ok(Some(AutoModViolation {
                                rule_type: "Excessive Caps".to_string(),
                                action: action.clone(),
                                message_id: msg.id,
                                user_id: msg.author.id,
                            }));
                        }
                    }
                }
            }
            AutoModRule::MentionSpam {
                mention_threshold,
                action,
            } => {
                if msg.mentions.len() >= *mention_threshold as usize {
                    return Ok(Some(AutoModViolation {
                        rule_type: "Mention Spam".to_string(),
                        action: action.clone(),
                        message_id: msg.id,
                        user_id: msg.author.id,
                    }));
                }
            }
            AutoModRule::LinkFilter {
                allowed_domains,
                action,
            } => {
                if let Some(violation) = self.check_links(msg, allowed_domains, action)? {
                    return Ok(Some(violation));
                }
            }
            AutoModRule::InviteFilter { action } => {
                let invite_patterns = [
                    "discord.gg/",
                    "discordapp.com/invite/",
                ];
                if invite_patterns.iter().any(|p| content_lower.contains(p)) {
                    return Ok(Some(AutoModViolation {
                        rule_type: "Invite Link".to_string(),
                        action: action.clone(),
                        message_id: msg.id,
                        user_id: msg.author.id,
                    }));
                }
            }
        }
        Ok(None)
    }

    fn check_links(
        &self,
        msg: &Message,
        allowed_domains: &[String],
        action: &AutoModAction,
    ) -> Result<Option<AutoModViolation>> {
        static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
        let url_pattern = URL_PATTERN.get_or_init(|| {
            Regex::new(r"https?://(?:www\.)?([a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+)").unwrap()
        });

        for cap in url_pattern.captures_iter(&msg.content) {
            if let Some(domain_match) = cap.get(1) {
                let domain = domain_match.as_str();
                if !allowed_domains.iter().any(|d| domain.contains(d)) {
                    return Ok(Some(AutoModViolation {
                        rule_type: "Disallowed Link".to_string(),
                        action: action.clone(),
                        message_id: msg.id,
                        user_id: msg.author.id,
                    }));
                }
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct AutoModViolation {
    pub rule_type: String,
    pub action: AutoModAction,
    pub message_id: MessageId,
    pub user_id: UserId,
}

pub async fn execute_action(
    ctx: &Context,
    violation: &AutoModViolation,
    msg: &Message,
    config: &AutoModConfig,
) -> Result<()> {
    let guild_id = msg.guild_id.ok_or_else(|| anyhow!("not in guild"))?;

    match &violation.action {
        AutoModAction::Delete => {
            msg.delete(&ctx.http).await?;
        }
        AutoModAction::Warn { .. } => {
            let _ = msg.reply(&ctx.http, "⚠️ Warning: Your message violated server rules.").await;
        }
        AutoModAction::Timeout { duration_secs } => {
            let until = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
            let ts = Timestamp::from_unix_timestamp(until.timestamp())?;
            guild_id
                .edit_member(&ctx.http, violation.user_id, EditMember::new().disable_communication_until_datetime(ts))
                .await?;
            let _ = msg.reply(&ctx.http, format!("⏱️ You've been timed out for {} seconds.", duration_secs)).await;
        }
        AutoModAction::Kick => {
            guild_id.kick_with_reason(&ctx.http, violation.user_id, &violation.rule_type).await?;
        }
        AutoModAction::Ban => {
            guild_id.ban_with_reason(&ctx.http, violation.user_id, 0, &violation.rule_type).await?;
        }
        AutoModAction::SendMessage { message } => {
            let _ = msg.reply(&ctx.http, message).await;
        }
    }

    if let Some(log_channel) = &config.log_channel {
        if let Ok(channel_id) = log_channel.parse::<u64>() {
            let _ = ChannelId::new(channel_id)
                .send_message(
                    &ctx.http,
                    CreateMessage::new().embed(
                        CreateEmbed::new()
                            .title("AutoMod Action")
                            .field("Rule", &violation.rule_type, true)
                            .field("User", format!("<@{}>", violation.user_id), true)
                            .field("Action", format!("{:?}", violation.action), true)
                            .color(0xffaa00)
                    ),
                )
                .await;
        }
    }

    Ok(())
}
