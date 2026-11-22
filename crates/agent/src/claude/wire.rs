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
    #[serde(default, deserialize_with = "lenient")]
    pub subtype: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub parent_tool_use_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub event: Option<StreamEvent>,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<Message>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub errors: Option<Vec<Value>>,
    #[serde(default, deserialize_with = "lenient")]
    pub terminal_reason: Option<String>,
    #[serde(default)]
    pub api_error_status: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub rate_limit_info: Option<RateLimitInfo>,
    #[serde(default, deserialize_with = "lenient")]
    pub attempt: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    pub max_retries: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    pub request_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub request: Option<ControlRequest>,
}

impl Frame {
    pub fn is_sidechain(&self) -> bool {
        self.parent_tool_use_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub delta: Option<Delta>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Delta {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub thinking: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Message {
    #[serde(default, deserialize_with = "blocks")]
    pub content: Vec<Block>,
}

fn blocks<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Block>, D::Error> {
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
pub struct Block {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub thinking: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub tool_use_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RateLimitInfo {
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(rename = "isUsingOverage", default, deserialize_with = "lenient")]
    pub is_using_overage: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ControlRequest {
    #[serde(default, deserialize_with = "lenient")]
    pub subtype: Option<String>,
}

pub fn user_message(text: &str) -> String {
    json!({
        "type": "user",
        "message": {"role": "user", "content": text},
        "parent_tool_use_id": null,
    })
    .to_string()
}

pub fn initialize_request(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "initialize"},
    })
    .to_string()
}

pub fn interrupt_request(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "interrupt"},
    })
    .to_string()
}

pub fn deny_tool(request_id: &str) -> String {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": "deny",
                "message": "Nitpick runs the assistant read-only.",
            },
        },
    })
    .to_string()
}

pub fn unsupported_request(request_id: &str) -> String {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": "Nitpick does not handle this request.",
        },
    })
    .to_string()
}
