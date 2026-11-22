use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

#[derive(Debug, Default, Deserialize)]
pub struct Frame {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub command: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub success: Option<bool>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub method: Option<String>,
    #[serde(
        rename = "assistantMessageEvent",
        default,
        deserialize_with = "lenient"
    )]
    pub update: Option<Update>,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<Message>,
    #[serde(rename = "toolCallId", default, deserialize_with = "lenient")]
    pub tool_call_id: Option<String>,
    #[serde(rename = "toolName", default, deserialize_with = "lenient")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(rename = "isError", default, deserialize_with = "lenient")]
    pub is_error: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub attempt: Option<u64>,
    #[serde(rename = "maxAttempts", default, deserialize_with = "lenient")]
    pub max_attempts: Option<u64>,
    #[serde(rename = "errorMessage", default, deserialize_with = "lenient")]
    pub error_message: Option<String>,
    #[serde(rename = "finalError", default, deserialize_with = "lenient")]
    pub final_error: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub aborted: Option<bool>,
}

impl Frame {
    pub fn id_str(&self) -> Option<&str> {
        self.id.as_ref().and_then(Value::as_str)
    }

    pub fn error_text(&self) -> Option<String> {
        match self.error.as_ref()? {
            Value::String(text) if !text.is_empty() => Some(text.clone()),
            Value::Null | Value::String(_) => None,
            other => Some(other.to_string()),
        }
    }

    pub fn disposition(&self) -> Option<&str> {
        self.data.as_ref()?.get("disposition")?.as_str()
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Update {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub delta: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Message {
    #[serde(default, deserialize_with = "lenient")]
    pub role: Option<String>,
    #[serde(rename = "stopReason", default, deserialize_with = "lenient")]
    pub stop_reason: Option<String>,
    #[serde(rename = "errorMessage", default, deserialize_with = "lenient")]
    pub error_message: Option<String>,
    #[serde(default, deserialize_with = "parts")]
    pub content: Vec<Part>,
}

fn parts<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Part>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(Value::Array(items)) = value else {
        return Ok(Vec::new());
    };
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

#[derive(Debug, Default, Deserialize)]
pub struct Part {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub thinking: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct State {
    #[serde(rename = "messageCount", default, deserialize_with = "lenient")]
    pub message_count: Option<u64>,
    #[serde(rename = "sessionId", default, deserialize_with = "lenient")]
    pub session_id: Option<String>,
}

impl State {
    pub fn from_data(data: Option<&Value>) -> Self {
        data.and_then(|data| serde_json::from_value(data.clone()).ok())
            .unwrap_or_default()
    }
}

pub fn prompt(id: &str, message: &str) -> String {
    json!({"id": id, "type": "prompt", "message": message}).to_string()
}

pub fn get_state(id: &str) -> String {
    json!({"id": id, "type": "get_state"}).to_string()
}

pub fn clear_queue(id: &str) -> String {
    json!({"id": id, "type": "clear_queue"}).to_string()
}

pub fn abort(id: &str) -> String {
    json!({"id": id, "type": "abort"}).to_string()
}

pub fn cancel_dialog(id: &Value) -> String {
    json!({"type": "extension_ui_response", "id": id, "cancelled": true}).to_string()
}
