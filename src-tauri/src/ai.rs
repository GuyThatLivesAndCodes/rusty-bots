use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use anyhow::{Result, anyhow};

const XAI_URL: &str = "https://api.x.ai/v1/chat/completions";

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
