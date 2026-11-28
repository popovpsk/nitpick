use serde_json::Value;

use crate::shown::after_instructions;
use crate::{Entry, EntryRole, prompt};

struct Open {
    role: EntryRole,
    text: String,
    message: Option<String>,
}

#[derive(Default)]
pub struct Replay {
    session: String,
    instructions: String,
    entries: Vec<Entry>,
    open: Option<Open>,
}

impl Replay {
    pub fn new(session: &str, instructions: &str) -> Self {
        Self {
            session: session.to_owned(),
            instructions: instructions.to_owned(),
            ..Self::default()
        }
    }

    pub fn feed(&mut self, params: &Value) {
        if params.get("sessionId").and_then(Value::as_str) != Some(self.session.as_str()) {
            return;
        }
        let Some(update) = params.get("update") else {
            return;
        };
        let role = match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("user_message_chunk") => EntryRole::User,
            Some("agent_message_chunk") => EntryRole::Assistant,
            _ => return,
        };
        let content = update.get("content");
        if content
            .and_then(|content| content.get("type"))
            .and_then(Value::as_str)
            != Some("text")
        {
            return;
        }
        let Some(text) = content
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        else {
            return;
        };
        let message = update
            .get("messageId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.push(role, message, text);
    }

    fn push(&mut self, role: EntryRole, message: Option<String>, text: &str) {
        if let Some(open) = self.open.as_mut()
            && open.role == role
        {
            let same = message.is_none() || open.message == message;
            match role {
                EntryRole::Assistant => {
                    if !same && !open.text.is_empty() {
                        open.text.push_str("\n\n");
                    }
                    open.text.push_str(text);
                    open.message = message;
                    return;
                }
                EntryRole::User if same => {
                    open.text.push_str(text);
                    return;
                }
                EntryRole::User => {}
            }
        }
        self.close();
        self.open = Some(Open {
            role,
            text: text.to_owned(),
            message,
        });
    }

    fn close(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        let text = if open.role == EntryRole::User && prompt::is_instructions(&open.text) {
            after_instructions(&open.text, &self.instructions).to_owned()
        } else {
            open.text
        };
        if !text.trim().is_empty() {
            self.entries.push(Entry {
                role: open.role,
                text,
            });
        }
    }

    pub fn finish(mut self) -> Vec<Entry> {
        self.close();
        self.entries
    }
}
