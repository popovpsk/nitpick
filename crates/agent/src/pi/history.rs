use serde_json::{Value, json};

use crate::{Entry, EntryRole};

pub const HISTORY_ID: &str = "nitpick-history";

pub fn get_messages(id: &str) -> String {
    json!({"id": id, "type": "get_messages"}).to_string()
}

pub fn entries(data: Option<&Value>) -> Vec<Entry> {
    let messages = data
        .and_then(|data| data.get("messages"))
        .and_then(Value::as_array);
    let mut entries = Vec::new();
    let mut answer = String::new();
    for message in messages.into_iter().flatten() {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                flush(&mut answer, &mut entries);
                if let Some(text) = content_text(message.get("content"), "\n\n") {
                    entries.push(Entry {
                        role: EntryRole::User,
                        text,
                    });
                }
            }
            Some("assistant") => {
                if message.get("stopReason").and_then(Value::as_str) == Some("error") {
                    continue;
                }
                if let Some(text) = content_text(message.get("content"), "") {
                    if !answer.is_empty() {
                        answer.push_str("\n\n");
                    }
                    answer.push_str(&text);
                }
            }
            _ => {}
        }
    }
    flush(&mut answer, &mut entries);
    entries
}

fn flush(answer: &mut String, entries: &mut Vec<Entry>) {
    let text = std::mem::take(answer);
    if !text.trim().is_empty() {
        entries.push(Entry {
            role: EntryRole::Assistant,
            text,
        });
    }
}

fn content_text(content: Option<&Value>, separator: &str) -> Option<String> {
    let text = match content? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(separator),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}
