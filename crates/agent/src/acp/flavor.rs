use serde_json::Value;

use crate::jsonrpc::RpcError;
use crate::{AcpFlavor, Failure, FailureKind};

use super::wire::Capabilities;

pub const OPENCODE_CONFIG: &str = "OPENCODE_CONFIG_CONTENT";
pub const OPENCODE_NO_PROJECT_CONFIG: &str = "OPENCODE_DISABLE_PROJECT_CONFIG";

const AUTH_CODE: i64 = -32000;
const NOT_FOUND_CODE: i64 = -32002;
const MISSING_CREDENTIAL: &str = "MISSING_CREDENTIAL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    V1,
    V2,
}

impl Dialect {
    pub fn from_version(output: Option<&str>) -> Dialect {
        match output.and_then(major_version) {
            Some(2) => Dialect::V2,
            _ => Dialect::V1,
        }
    }
}

fn major_version(output: &str) -> Option<u64> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    line.split_whitespace().find_map(|word| {
        let word = word.strip_prefix('v').unwrap_or(word);
        let (major, rest) = word.split_once('.')?;
        let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if minor.is_empty() {
            return None;
        }
        major.parse().ok()
    })
}

pub fn opencode_config(dialect: Dialect) -> String {
    match dialect {
        Dialect::V1 => r#"{"permission":{"edit":"deny"}}"#.to_owned(),
        Dialect::V2 => {
            r#"{"permissions":[{"action":"edit","resource":"*","effect":"deny"}]}"#.to_owned()
        }
    }
}

pub fn environment(flavor: AcpFlavor, dialect: Dialect) -> Vec<(String, String)> {
    match flavor {
        AcpFlavor::OpenCode => vec![
            (OPENCODE_CONFIG.to_owned(), opencode_config(dialect)),
            (OPENCODE_NO_PROJECT_CONFIG.to_owned(), "1".to_owned()),
        ],
        AcpFlavor::DeepSeek | AcpFlavor::Generic => Vec::new(),
    }
}

pub fn needs_version(flavor: AcpFlavor) -> bool {
    flavor == AcpFlavor::OpenCode
}

pub fn wants_plan_mode(flavor: AcpFlavor) -> bool {
    flavor == AcpFlavor::OpenCode
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeVia {
    Resume,
    Load,
}

impl ResumeVia {
    pub fn method(self) -> &'static str {
        match self {
            ResumeVia::Resume => "session/resume",
            ResumeVia::Load => "session/load",
        }
    }
}

pub fn resume_via(flavor: AcpFlavor, capabilities: &Capabilities) -> Option<ResumeVia> {
    if capabilities.resume {
        Some(ResumeVia::Resume)
    } else if capabilities.load_session && flavor != AcpFlavor::DeepSeek {
        Some(ResumeVia::Load)
    } else {
        None
    }
}

fn data_text(error: &RpcError) -> String {
    match &error.data {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

pub fn session_lost(flavor: AcpFlavor, error: &RpcError) -> bool {
    if error.code == NOT_FOUND_CODE
        || error
            .message
            .to_ascii_lowercase()
            .contains("session not found")
    {
        return true;
    }
    flavor == AcpFlavor::OpenCode
        && error
            .data
            .as_ref()
            .and_then(|data| data.get("service"))
            .and_then(Value::as_str)
            == Some("session")
}

pub fn not_logged_in(flavor: AcpFlavor, error: &RpcError) -> bool {
    let text = format!("{} {}", error.message, data_text(error));
    if text.to_ascii_lowercase().contains("auth_required") {
        return true;
    }
    match flavor {
        AcpFlavor::OpenCode => error.code == AUTH_CODE,
        AcpFlavor::DeepSeek => text.contains(MISSING_CREDENTIAL),
        AcpFlavor::Generic => false,
    }
}

pub fn failure(flavor: AcpFlavor, error: &RpcError, resuming: bool) -> Failure {
    let kind = if resuming && session_lost(flavor, error) {
        FailureKind::SessionLost
    } else if not_logged_in(flavor, error) {
        FailureKind::NotLoggedIn
    } else {
        FailureKind::Other
    };
    let message = if error.message.trim().is_empty() {
        format!("The agent reported an error ({}).", error.code)
    } else {
        error.message.clone()
    };
    Failure { kind, message }
}
