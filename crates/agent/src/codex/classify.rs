use std::fmt;

use serde_json::Value;

use crate::FailureKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

pub const MINIMUM_VERSION: Version = Version(0, 146, 0);

pub fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim().trim_start_matches('v');
    let numeric = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()?;
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Version(major, minor, patch))
}

pub fn version_from_user_agent(user_agent: &str) -> Option<Version> {
    let (_, rest) = user_agent.split_once('/')?;
    parse_version(rest.split_whitespace().next()?)
}

pub fn check_version(version: Option<Version>) -> Result<(), String> {
    match version {
        Some(found) if found < MINIMUM_VERSION => Err(format!(
            "Codex {found} is too old: Nitpick needs {MINIMUM_VERSION} or newer. Update Codex and try again."
        )),
        _ => Ok(()),
    }
}

fn normalized(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn code_parts(code: Option<&Value>) -> (String, Option<u64>) {
    match code {
        Some(Value::String(name)) => (normalized(name), None),
        Some(Value::Object(fields)) => match fields.iter().next() {
            Some((name, detail)) => (
                normalized(name),
                detail.get("httpStatusCode").and_then(Value::as_u64),
            ),
            None => (String::new(), None),
        },
        _ => (String::new(), None),
    }
}

pub fn classify(code: Option<&Value>, message: &str) -> FailureKind {
    let (variant, status) = code_parts(code);
    let text = message.to_lowercase();
    let says = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if variant == "unauthorized"
        || status == Some(401)
        || says(&[
            "unauthorized",
            "not logged in",
            "codex login",
            "invalid api key",
            "incorrect api key",
        ])
    {
        FailureKind::NotLoggedIn
    } else if matches!(variant.as_str(), "usagelimitexceeded" | "ratelimitexceeded")
        || status == Some(429)
        || says(&["usage limit"])
    {
        FailureKind::UsageLimit
    } else if variant == "contextwindowexceeded" || says(&["context_length_exceeded"]) {
        FailureKind::ContextTooLong
    } else if says(&["model_not_found"]) || (says(&["model"]) && says(&["does not exist"])) {
        FailureKind::ModelNotFound
    } else {
        FailureKind::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeError {
    Lost,
    Archived,
    Other,
}

pub fn resume_error(message: &str) -> ResumeError {
    let text = message.to_lowercase();
    let says = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if says(&["is archived"]) {
        ResumeError::Archived
    } else if says(&[
        "no rollout found",
        "thread not found",
        "session not found",
        "conversation not found",
        "invalid session id",
        "invalid thread id",
    ]) {
        ResumeError::Lost
    } else if says(&["failed to read thread", "rollout at"]) {
        if says(&[
            "os error",
            "permission denied",
            "locked",
            "busy",
            "too many open files",
            "timed out",
        ]) {
            ResumeError::Other
        } else if says(&[
            "is empty",
            "valid utf-8",
            "failed to parse",
            "parse error",
            "expected value",
            "eof while parsing",
            "invalid json",
        ]) {
            ResumeError::Lost
        } else {
            ResumeError::Other
        }
    } else {
        ResumeError::Other
    }
}
