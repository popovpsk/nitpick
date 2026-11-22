use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use crate::Launch;

pub const PROTOCOL_VERSION: i64 = 1;

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub load_session: bool,
    pub resume: bool,
    pub system_prompt: bool,
}

fn declared(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Object(_) | Value::Bool(true)))
}

impl Capabilities {
    pub fn from_initialize(result: &Value) -> Self {
        let agent = result.get("agentCapabilities");
        let sessions = agent.and_then(|agent| agent.get("sessionCapabilities"));
        Self {
            load_session: agent
                .and_then(|agent| agent.get("loadSession"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            resume: declared(sessions.and_then(|sessions| sessions.get("resume"))),
            system_prompt: declared(sessions.and_then(|sessions| sessions.get("systemPrompt"))),
        }
    }
}

pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "clientCapabilities": {
            "fs": {"readTextFile": false, "writeTextFile": false},
            "terminal": false,
        },
        "clientInfo": {
            "name": "nitpick",
            "title": "Nitpick",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn session_settings(launch: &Launch, system_prompt: bool) -> Map<String, Value> {
    let cwd = std::path::absolute(&launch.cwd).unwrap_or_else(|_| launch.cwd.clone());
    let mut params = Map::new();
    params.insert("cwd".into(), json!(cwd.to_string_lossy()));
    params.insert("mcpServers".into(), json!([]));
    if system_prompt && !launch.instructions.trim().is_empty() {
        params.insert(
            "systemPrompt".into(),
            json!({"mode": "append", "text": launch.instructions}),
        );
    }
    params
}

pub fn new_session_params(launch: &Launch, system_prompt: bool) -> Value {
    Value::Object(session_settings(launch, system_prompt))
}

pub fn resume_params(launch: &Launch, session: &str, system_prompt: bool) -> Value {
    let mut params = session_settings(launch, system_prompt);
    params.insert("sessionId".into(), json!(session));
    Value::Object(params)
}

pub fn prompt_params(session: &str, instructions: Option<&str>, text: &str) -> Value {
    let mut blocks = Vec::new();
    if let Some(instructions) = instructions {
        blocks.push(json!({"type": "text", "text": instructions}));
    }
    blocks.push(json!({"type": "text", "text": text}));
    json!({"sessionId": session, "prompt": blocks})
}

pub fn cancel_params(session: &str) -> Value {
    json!({"sessionId": session})
}

pub fn set_option_params(session: &str, option: &str, value: &Value) -> Value {
    json!({"sessionId": session, "configId": option, "value": value})
}

pub fn set_model_params(session: &str, model: &str) -> Value {
    json!({"sessionId": session, "modelId": model})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelState {
    pub current: Option<String>,
}

pub fn model_state(result: &Value) -> Option<ModelState> {
    let models = result.get("models")?;
    models.get("availableModels")?.as_array()?;
    Some(ModelState {
        current: models
            .get("currentModelId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub fn session_id(result: &Value) -> Option<&str> {
    result
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

pub fn stop_reason(result: &Value) -> Option<&str> {
    result.get("stopReason").and_then(Value::as_str)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigOption {
    pub id: String,
    pub current: Option<Value>,
    pub values: Vec<Value>,
}

impl ConfigOption {
    pub fn offers(&self, value: &str) -> bool {
        self.values
            .iter()
            .any(|offered| offered.as_str() == Some(value))
    }

    pub fn is_current(&self, value: &str) -> bool {
        self.current.as_ref().and_then(Value::as_str) == Some(value)
    }
}

pub fn config_option(result: &Value, category: &str) -> Option<ConfigOption> {
    let options = result.get("configOptions")?.as_array()?;
    let option = options
        .iter()
        .find(|option| option.get("category").and_then(Value::as_str) == Some(category))?;
    let id = option.get("id").and_then(Value::as_str)?.to_owned();
    let mut values = Vec::new();
    collect_values(option.get("options"), &mut values);
    Some(ConfigOption {
        id,
        current: option.get("currentValue").cloned(),
        values,
    })
}

fn collect_values(options: Option<&Value>, out: &mut Vec<Value>) {
    for entry in options.and_then(Value::as_array).into_iter().flatten() {
        if let Some(value) = entry.get("value") {
            out.push(value.clone());
        } else {
            collect_values(entry.get("options"), out);
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Update {
    #[serde(rename = "sessionUpdate", default, deserialize_with = "lenient")]
    pub update: Option<String>,
    #[serde(rename = "messageId", default, deserialize_with = "lenient")]
    pub message_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub content: Option<Content>,
    #[serde(rename = "toolCallId", default, deserialize_with = "lenient")]
    pub tool_call_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Content {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
}

impl Update {
    pub fn parse(update: &Value) -> Self {
        Self::deserialize(update).unwrap_or_default()
    }

    pub fn text(&self) -> Option<&str> {
        let content = self.content.as_ref()?;
        if content.kind.as_deref().is_some_and(|kind| kind != "text") {
            return None;
        }
        content.text.as_deref().filter(|text| !text.is_empty())
    }
}
