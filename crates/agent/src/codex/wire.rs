use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use crate::Launch;

pub const APPROVAL_POLICY: &str = "never";
pub const SANDBOX: &str = "read-only";

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

#[derive(Debug, Default, Deserialize)]
pub struct Params {
    #[serde(rename = "threadId", default, deserialize_with = "lenient")]
    pub thread_id: Option<String>,
    #[serde(rename = "turnId", default, deserialize_with = "lenient")]
    pub turn_id: Option<String>,
    #[serde(rename = "itemId", default, deserialize_with = "lenient")]
    pub item_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub delta: Option<String>,
    #[serde(rename = "textDelta", default, deserialize_with = "lenient")]
    pub text_delta: Option<String>,
    #[serde(rename = "summaryIndex", default, deserialize_with = "lenient")]
    pub summary_index: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    pub item: Option<Item>,
    #[serde(default, deserialize_with = "lenient")]
    pub turn: Option<Turn>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<String>,
    #[serde(rename = "additionalDetails", default, deserialize_with = "lenient")]
    pub additional_details: Option<String>,
    #[serde(rename = "codexErrorInfo", default)]
    pub codex_error_info: Option<Value>,
    #[serde(rename = "willRetry", default, deserialize_with = "lenient")]
    pub will_retry: Option<bool>,
}

impl Params {
    pub fn parse(params: &Value) -> Self {
        Self::deserialize(params).unwrap_or_default()
    }

    pub fn text(&self) -> Option<&str> {
        self.delta
            .as_deref()
            .or(self.text_delta.as_deref())
            .filter(|text| !text.is_empty())
    }

    pub fn error_info(&self) -> ErrorInfo {
        match &self.error {
            Some(Value::Object(_)) => self
                .error
                .as_ref()
                .and_then(|value| ErrorInfo::deserialize(value).ok())
                .unwrap_or_default(),
            Some(Value::String(text)) => ErrorInfo {
                message: Some(text.clone()),
                ..ErrorInfo::default()
            },
            _ => ErrorInfo {
                message: self.message.clone(),
                additional_details: self.additional_details.clone(),
                codex_error_info: self.codex_error_info.clone(),
            },
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ErrorInfo {
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<String>,
    #[serde(rename = "additionalDetails", default, deserialize_with = "lenient")]
    pub additional_details: Option<String>,
    #[serde(rename = "codexErrorInfo", default)]
    pub codex_error_info: Option<Value>,
}

impl ErrorInfo {
    pub fn from_rpc(message: &str, data: Option<&Value>) -> Self {
        let code = data.and_then(|data| data.get("codexErrorInfo").cloned());
        Self {
            message: Some(message.to_owned()),
            additional_details: None,
            codex_error_info: code,
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.message
            .as_deref()
            .or(self.additional_details.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Turn {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Item {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub command: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(
        rename = "exitCode",
        alias = "exit_code",
        default,
        deserialize_with = "lenient"
    )]
    pub exit_code: Option<i64>,
    #[serde(
        rename = "commandActions",
        alias = "command_actions",
        default,
        deserialize_with = "lenient"
    )]
    pub command_actions: Option<Vec<Value>>,
}

impl Item {
    pub fn is(&self, camel: &str, snake: &str) -> bool {
        self.kind
            .as_deref()
            .is_some_and(|kind| kind == camel || kind == snake)
    }

    pub fn action_commands(&self) -> Vec<&str> {
        let mut commands: Vec<&str> = Vec::new();
        for action in self.command_actions.iter().flatten() {
            let Some(command) = action
                .get("command")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|command| !command.is_empty())
            else {
                continue;
            };
            if commands.last() != Some(&command) {
                commands.push(command);
            }
        }
        commands
    }
}

pub fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "nitpick",
            "title": "Nitpick",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {"experimentalApi": true},
    })
}

fn thread_settings(launch: &Launch) -> Map<String, Value> {
    let cwd = std::path::absolute(&launch.cwd).unwrap_or_else(|_| launch.cwd.clone());
    let mut settings = Map::new();
    settings.insert("cwd".into(), json!(cwd.to_string_lossy()));
    settings.insert("approvalPolicy".into(), json!(APPROVAL_POLICY));
    settings.insert("sandbox".into(), json!(SANDBOX));
    if let Some(model) = launch.model.as_deref().filter(|model| !model.is_empty()) {
        settings.insert("model".into(), json!(model));
    }
    if !launch.instructions.is_empty() {
        settings.insert("developerInstructions".into(), json!(launch.instructions));
    }
    settings
}

pub fn thread_start_params(launch: &Launch) -> Value {
    Value::Object(thread_settings(launch))
}

pub fn thread_resume_params(launch: &Launch, thread_id: &str) -> Value {
    let mut params = thread_settings(launch);
    params.insert("threadId".into(), json!(thread_id));
    params.insert("excludeTurns".into(), json!(true));
    Value::Object(params)
}

pub fn unarchive_params(thread_id: &str) -> Value {
    json!({"threadId": thread_id})
}

pub fn turn_start_params(thread_id: &str, text: &str) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": text}],
        "approvalPolicy": APPROVAL_POLICY,
        "sandboxPolicy": {"type": "readOnly"},
        "summary": "auto",
    })
}

pub fn interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    json!({"threadId": thread_id, "turnId": turn_id})
}

pub fn server_reply(method: &str) -> Option<Value> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({"decision": "decline"}))
        }
        "item/permissions/requestApproval" => Some(json!({"permissions": {}, "scope": "turn"})),
        "item/tool/requestUserInput" => Some(json!({"answers": {}})),
        "mcpServer/elicitation/request" => Some(json!({"action": "decline"})),
        _ => None,
    }
}

pub fn thread_id(result: &Value) -> Option<&str> {
    result
        .get("thread")?
        .get("id")?
        .as_str()
        .filter(|id| !id.is_empty())
}

pub fn turn_id(result: &Value) -> Option<&str> {
    result
        .get("turn")?
        .get("id")?
        .as_str()
        .filter(|id| !id.is_empty())
}

pub fn user_agent(result: &Value) -> Option<&str> {
    result.get("userAgent")?.as_str()
}

pub fn cli_version(result: &Value) -> Option<&str> {
    result.get("thread")?.get("cliVersion")?.as_str()
}
