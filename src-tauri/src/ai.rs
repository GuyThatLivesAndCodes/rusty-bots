use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use anyhow::{Result, anyhow};

const XAI_URL: &str = "https://api.x.ai/v1/chat/completions";
const VOICES_URL: &str = "https://api.x.ai/v1/tts/voices";
const CUSTOM_VOICES_URL: &str = "https://api.x.ai/v1/custom-voices";

#[derive(Debug, Clone, Serialize)]
pub struct VoiceInfo {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// Pull every voice id from a flexible JSON response. xAI may wrap the list in
/// `voices`, `data`, `custom_voices`, `results`, or return a bare array.
fn collect_voices(v: &Value, kind: &str, out: &mut Vec<VoiceInfo>) {
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else {
        ["voices", "data", "custom_voices", "results", "items"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_array()).cloned())
            .unwrap_or_default()
    };
    for item in arr {
        if let Some(s) = item.as_str() {
            out.push(VoiceInfo { id: s.to_string(), label: s.to_string(), kind: kind.to_string() });
            continue;
        }
        let id = ["voice_id", "id", "name"]
            .iter()
            .find_map(|k| item.get(*k).and_then(|x| x.as_str()))
            .map(|s| s.to_string());
        let Some(id) = id else { continue; };
        let name = item.get("name").and_then(|x| x.as_str()).unwrap_or(&id);
        let mut details = Vec::new();
        for k in ["gender", "age", "tone", "language"] {
            if let Some(d) = item.get(k).and_then(|x| x.as_str()) { details.push(d.to_string()); }
        }
        let label = if details.is_empty() {
            format!("{name} ({id})")
        } else {
            format!("{name} — {} ({id})", details.join(", "))
        };
        out.push(VoiceInfo { id, label, kind: kind.to_string() });
    }
}

pub async fn list_voices(api_key: &str) -> Result<Vec<VoiceInfo>> {
    if api_key.is_empty() {
        return Err(anyhow!("xAI API key not set"));
    }
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut last_err = None;
    for (url, kind) in [(VOICES_URL, "built-in"), (CUSTOM_VOICES_URL, "custom")] {
        match client.get(url).bearer_auth(api_key).send().await {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        collect_voices(&v, kind, &mut out);
                    }
                } else {
                    last_err = Some(anyhow!("{} {}: {}", kind, status, text));
                }
            }
            Err(e) => last_err = Some(anyhow!("{kind}: {e}")),
        }
    }
    if out.is_empty() {
        if let Some(e) = last_err { return Err(e); }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(default)]
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

pub fn tool_schema() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "send_message",
                "description": "Reply to the channel with a message.",
                "parameters": {
                    "type": "object",
                    "properties": {"content": {"type": "string"}},
                    "required": ["content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "react",
                "description": "React to the most recent message with an emoji (unicode).",
                "parameters": {
                    "type": "object",
                    "properties": {"emoji": {"type": "string"}},
                    "required": ["emoji"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "kick_user",
                "description": "Kick a user from the guild by user_id.",
                "parameters": {
                    "type": "object",
                    "properties": {"user_id": {"type": "string"}, "reason": {"type": "string"}},
                    "required": ["user_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "ban_user",
                "description": "Ban a user from the guild by user_id.",
                "parameters": {
                    "type": "object",
                    "properties": {"user_id": {"type": "string"}, "reason": {"type": "string"}},
                    "required": ["user_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "timeout_user",
                "description": "Timeout (mute) a user for N seconds.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "user_id": {"type": "string"},
                        "seconds": {"type": "integer"}
                    },
                    "required": ["user_id", "seconds"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "add_role",
                "description": "Give a role to a user.",
                "parameters": {
                    "type": "object",
                    "properties": {"user_id": {"type": "string"}, "role_id": {"type": "string"}},
                    "required": ["user_id", "role_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "remove_role",
                "description": "Remove a role from a user.",
                "parameters": {
                    "type": "object",
                    "properties": {"user_id": {"type": "string"}, "role_id": {"type": "string"}},
                    "required": ["user_id", "role_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "join_voice",
                "description": "Join the voice channel that the mentioning user is currently in, to talk live.",
                "parameters": {"type": "object", "properties": {}}
            }
        },
        {
            "type": "function",
            "function": {
                "name": "leave_voice",
                "description": "Leave the current voice channel.",
                "parameters": {"type": "object", "properties": {}}
            }
        }
    ])
}

pub async fn complete(
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
) -> Result<AssistantMessage> {
    if api_key.is_empty() {
        return Err(anyhow!("xAI API key is not set for this bot"));
    }
    let client = reqwest::Client::new();
    let body = json!({
        "model": model,
        "messages": messages,
        "tools": tool_schema(),
        "tool_choice": "auto",
    });
    let resp = client.post(XAI_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow!("xAI {}: {}", status, text));
    }
    let v: Value = serde_json::from_str(&text)?;
    let msg = v["choices"][0]["message"].clone();
    let am: AssistantMessage = serde_json::from_value(msg)?;
    Ok(am)
}
