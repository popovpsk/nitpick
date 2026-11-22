use std::collections::HashMap;

use serde_json::Value;

use super::wire::Update;
use crate::{Event, Failure, FailureKind, Outcome};

const TITLE_CHARS: usize = 120;
const CANCEL_NOTE: &str = "the request was cancelled by the client.";
const REFUSAL: &str = "The assistant declined to answer.";

#[derive(Debug, Default)]
struct Tool {
    kind: Option<String>,
    title: String,
    started: bool,
    closed: bool,
}

#[derive(Debug, Default)]
struct Channel {
    last_id: Option<String>,
    emitted: bool,
}

impl Channel {
    fn boundary(&mut self, id: Option<&str>, forced: bool) -> bool {
        let differs = match (&self.last_id, id) {
            (Some(last), Some(id)) => last != id,
            _ => false,
        };
        let boundary = self.emitted && (differs || (forced && id.is_none()));
        if let Some(id) = id {
            self.last_id = Some(id.to_owned());
        }
        self.emitted = true;
        boundary
    }
}

#[derive(Debug, Default)]
pub struct Normalizer {
    active: bool,
    session: Option<String>,
    tools: HashMap<String, Tool>,
    text: Channel,
    thinking: Channel,
    tool_since_text: bool,
    cancelled: bool,
    held: String,
}

impl Normalizer {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn set_session(&mut self, id: &str) {
        self.session = Some(id.to_owned());
    }

    pub fn begin_turn(&mut self) {
        *self = Self {
            active: true,
            session: self.session.take(),
            ..Self::default()
        };
    }

    pub fn mark_cancelled(&mut self) {
        self.cancelled = true;
    }

    pub fn open_tools(&self) -> usize {
        self.tools.values().filter(|tool| !tool.closed).count()
    }

    pub fn kind_of(&self, id: &str) -> Option<&str> {
        self.tools.get(id).and_then(|tool| tool.kind.as_deref())
    }

    pub fn finish(&mut self, outcome: Outcome) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.active {
            return events;
        }
        let mut open: Vec<(&String, &Tool)> = self
            .tools
            .iter()
            .filter(|(_, tool)| tool.started && !tool.closed)
            .collect();
        open.sort_by(|a, b| a.0.cmp(b.0));
        for (id, _) in open {
            events.push(Event::ToolFinished {
                id: id.clone(),
                failed: true,
            });
        }
        events.push(Event::TurnFinished(outcome));
        self.active = false;
        self.tools.clear();
        self.held.clear();
        events
    }

    pub fn feed(&mut self, params: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.active {
            return events;
        }
        if let (Some(ours), Some(theirs)) = (
            &self.session,
            params.get("sessionId").and_then(Value::as_str),
        ) && ours != theirs
        {
            return events;
        }
        let Some(update) = params.get("update") else {
            return events;
        };
        let update = Update::parse(update);
        match update.update.as_deref() {
            Some("agent_message_chunk") => self.message(&update, &mut events),
            Some("agent_thought_chunk") => self.thought(&update, &mut events),
            Some("tool_call") => self.tool_call(&update, &mut events),
            Some("tool_call_update") => self.tool_update(&update, &mut events),
            _ => {}
        }
        events
    }

    fn message(&mut self, update: &Update, events: &mut Vec<Event>) {
        let Some(text) = update.text() else { return };
        let Some(text) = self.unfiltered(text) else {
            return;
        };
        let separator = self
            .text
            .boundary(update.message_id.as_deref(), self.tool_since_text);
        self.tool_since_text = false;
        events.push(Event::Text(if separator {
            format!("\n\n{text}")
        } else {
            text
        }));
    }

    fn unfiltered(&mut self, text: &str) -> Option<String> {
        if !self.cancelled {
            return Some(text.to_owned());
        }
        self.held.push_str(text);
        let seen = self
            .held
            .trim()
            .to_lowercase()
            .replace("canceled", "cancelled");
        if seen.is_empty() || (CANCEL_NOTE.starts_with(&seen) && seen != CANCEL_NOTE) {
            return None;
        }
        let held = std::mem::take(&mut self.held);
        if seen == CANCEL_NOTE {
            None
        } else {
            Some(held)
        }
    }

    fn thought(&mut self, update: &Update, events: &mut Vec<Event>) {
        let Some(text) = update.text() else { return };
        let separator = self.thinking.boundary(update.message_id.as_deref(), false);
        events.push(Event::Thinking(if separator {
            format!("\n\n{text}")
        } else {
            text.to_owned()
        }));
    }

    fn tool_call(&mut self, update: &Update, events: &mut Vec<Event>) {
        let Some(id) = update.tool_call_id.clone() else {
            return;
        };
        if self.tools.contains_key(&id) {
            self.tool_update(update, events);
            return;
        }
        self.tool_since_text = true;
        self.tools.insert(id.clone(), Tool::default());
        self.apply(&id, update, events);
    }

    fn tool_update(&mut self, update: &Update, events: &mut Vec<Event>) {
        let Some(id) = update.tool_call_id.clone() else {
            return;
        };
        if !self.tools.contains_key(&id) {
            self.tool_since_text = true;
            self.tools.insert(id.clone(), Tool::default());
            self.apply(&id, update, events);
            let tool = self.tools.get_mut(&id);
            if let Some(tool) = tool
                && !tool.started
                && !tool.closed
            {
                tool.started = true;
                events.push(Event::ToolStarted {
                    id,
                    title: display_title(tool),
                });
            }
            return;
        }
        self.apply(&id, update, events);
    }

    fn apply(&mut self, id: &str, update: &Update, events: &mut Vec<Event>) {
        let Some(tool) = self.tools.get_mut(id) else {
            return;
        };
        if tool.closed {
            return;
        }
        if let Some(title) = update
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
        {
            tool.title = title.to_owned();
        }
        if let Some(kind) = update.kind.as_deref().filter(|kind| !kind.is_empty()) {
            tool.kind = Some(kind.to_owned());
        }
        let status = update.status.as_deref();
        let running = matches!(status, Some("in_progress" | "completed" | "failed"));
        let finished = matches!(status, Some("completed" | "failed"));
        if running && !tool.started {
            tool.started = true;
            events.push(Event::ToolStarted {
                id: id.to_owned(),
                title: display_title(tool),
            });
        }
        if finished {
            tool.closed = true;
            events.push(Event::ToolFinished {
                id: id.to_owned(),
                failed: status == Some("failed"),
            });
        }
    }
}

fn display_title(tool: &Tool) -> String {
    let title = one_line(&tool.title);
    if !title.is_empty() {
        return title;
    }
    match tool.kind.as_deref() {
        Some(kind) if !kind.is_empty() => kind.to_owned(),
        _ => "tool".to_owned(),
    }
}

fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= TITLE_CHARS {
        return joined;
    }
    let mut short: String = joined.chars().take(TITLE_CHARS).collect();
    short.push('…');
    short
}

pub fn outcome_from_stop(reason: Option<&str>) -> Outcome {
    match reason {
        Some("cancelled") => Outcome::Interrupted,
        Some("refusal") => Outcome::Failed(Failure {
            kind: FailureKind::Other,
            message: REFUSAL.to_owned(),
        }),
        _ => Outcome::Completed,
    }
}
