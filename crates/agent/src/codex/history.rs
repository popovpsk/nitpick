use serde_json::{Value, json};

use crate::{Entry, EntryRole};

pub const PAGE_SIZE: u32 = 100;
pub const MAX_PAGES: usize = 20;

pub fn turns_params(thread_id: &str, cursor: Option<&str>) -> Value {
    json!({
        "threadId": thread_id,
        "cursor": cursor,
        "limit": PAGE_SIZE,
        "sortDirection": "desc",
        "itemsView": "full",
    })
}

pub fn page(result: &Value) -> (Vec<Value>, Option<String>) {
    let turns = result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let next = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    (turns, next)
}

pub fn entries(turns: &[Value]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for turn in turns {
        let mut answer = String::new();
        let items = turn.get("items").and_then(Value::as_array);
        for item in items.into_iter().flatten() {
            match item.get("type").and_then(Value::as_str) {
                Some("userMessage" | "user_message") => {
                    flush(&mut answer, &mut entries);
                    if let Some(text) = user_text(item) {
                        entries.push(Entry {
                            role: EntryRole::User,
                            text,
                        });
                    }
                }
                Some("agentMessage" | "agent_message") => {
                    let text = item.get("text").and_then(Value::as_str).unwrap_or("");
                    if !text.is_empty() {
                        if !answer.is_empty() {
                            answer.push_str("\n\n");
                        }
                        answer.push_str(text);
                    }
                }
                _ => {}
            }
        }
        flush(&mut answer, &mut entries);
    }
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

fn user_text(item: &Value) -> Option<String> {
    let texts: Vec<&str> = item
        .get("content")?
        .as_array()?
        .iter()
        .filter(|input| input.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|input| input.get("text").and_then(Value::as_str))
        .collect();
    let text = texts.join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}
