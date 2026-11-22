use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Reject,
}

pub fn is_write_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "edit" | "delete" | "move"
    )
}

pub fn verdict(kinds: &[Option<&str>]) -> Verdict {
    if kinds.iter().flatten().any(|kind| is_write_kind(kind)) {
        Verdict::Reject
    } else {
        Verdict::Allow
    }
}

fn wanted(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Allow => "allow_once",
        Verdict::Reject => "reject_once",
    }
}

fn spelled(kind: &str) -> String {
    kind.trim().to_ascii_lowercase().replace('-', "_")
}

pub fn outcome(verdict: Verdict, options: &Value) -> Value {
    let wanted = wanted(verdict);
    let chosen = options
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| {
            option
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| spelled(kind) == wanted)
        })
        .find_map(|option| option.get("optionId").and_then(Value::as_str));
    match chosen {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

pub fn answer(params: &Value, recalled: Option<&str>) -> Value {
    let own = params
        .get("toolCall")
        .and_then(|call| call.get("kind"))
        .and_then(Value::as_str);
    let verdict = verdict(&[own, recalled]);
    let options = params.get("options").unwrap_or(&Value::Null);
    outcome(verdict, options)
}

pub fn tool_call_id(params: &Value) -> Option<&str> {
    params
        .get("toolCall")
        .and_then(|call| call.get("toolCallId"))
        .and_then(Value::as_str)
}
