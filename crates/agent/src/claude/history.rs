use std::io::Read as _;
use std::path::Path;

use serde_json::Value;

use crate::{Entry, EntryRole};

pub const MAX_FILE: u64 = 64 * 1024 * 1024;

const INTERRUPTED: &str = "[Request interrupted by user";
const SYNTHETIC_MODEL: &str = "<synthetic>";

pub fn read(path: &Path) -> Vec<Entry> {
    let mut bytes = Vec::new();
    let read =
        std::fs::File::open(path).and_then(|file| file.take(MAX_FILE + 1).read_to_end(&mut bytes));
    match read {
        Ok(_) if bytes.len() as u64 > MAX_FILE => {
            log::debug!("the assistant's session file is over {MAX_FILE} bytes; no history");
            Vec::new()
        }
        Ok(_) => parse(&String::from_utf8_lossy(&bytes)),
        Err(error) => {
            log::debug!("reading the assistant's session file failed: {error}");
            Vec::new()
        }
    }
}

#[derive(Default)]
struct Answer {
    text: String,
    message: Option<String>,
}

impl Answer {
    fn push(&mut self, message: Option<&str>, text: &str) {
        if text.is_empty() {
            return;
        }
        let same = message.is_some() && self.message.as_deref() == message;
        if !same && !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        self.text.push_str(text);
        self.message = message.map(str::to_owned);
    }

    fn flush(&mut self, entries: &mut Vec<Entry>) {
        let answer = std::mem::take(self);
        if !answer.text.trim().is_empty() {
            entries.push(Entry {
                role: EntryRole::Assistant,
                text: answer.text,
            });
        }
    }
}

pub fn parse(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut answer = Answer::default();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if flag(&record, "isSidechain") || flag(&record, "isMeta") {
            continue;
        }
        let Some(message) = record.get("message") else {
            continue;
        };
        match record.get("type").and_then(Value::as_str) {
            Some("user") => {
                if flag(&record, "isCompactSummary") {
                    continue;
                }
                let Some(prompt) = user_text(message) else {
                    continue;
                };
                answer.flush(&mut entries);
                entries.push(Entry {
                    role: EntryRole::User,
                    text: prompt,
                });
            }
            Some("assistant") => {
                let failed = flag(&record, "isApiErrorMessage")
                    || record.get("error").is_some_and(Value::is_string)
                    || message.get("model").and_then(Value::as_str) == Some(SYNTHETIC_MODEL);
                if failed {
                    continue;
                }
                let id = message.get("id").and_then(Value::as_str);
                for block in blocks(message) {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        answer.push(id, block.get("text").and_then(Value::as_str).unwrap_or(""));
                    }
                }
            }
            _ => {}
        }
    }
    answer.flush(&mut entries);
    entries
}

fn flag(record: &Value, name: &str) -> bool {
    record.get(name).and_then(Value::as_bool) == Some(true)
}

fn blocks(message: &Value) -> &[Value] {
    message
        .get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn user_text(message: &Value) -> Option<String> {
    let text = match message.get("content")? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect();
            if texts.is_empty() {
                return None;
            }
            texts.join("\n\n")
        }
        _ => return None,
    };
    (!text.trim().is_empty() && !text.starts_with(INTERRUPTED)).then_some(text)
}
