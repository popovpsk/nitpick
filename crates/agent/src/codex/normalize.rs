use std::collections::HashSet;

use serde_json::Value;

use super::classify::classify;
use super::wire::{ErrorInfo, Item, Params};
use crate::{Event, Failure, Outcome};

const TITLE_CHARS: usize = 120;
const GENERIC_FAILURE: &str = "Codex stopped with an error.";
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];

#[derive(Debug, Default)]
pub struct Output {
    pub events: Vec<Event>,
    pub finished: bool,
}

#[derive(Debug, Default)]
pub struct Normalizer {
    active: bool,
    thread: Option<String>,
    turn: Option<String>,
    streamed: HashSet<String>,
    separator_due: bool,
    last_summary: Option<(String, i64)>,
    thinking_emitted: bool,
    open_tools: Vec<String>,
    seen_tools: HashSet<String>,
    held_error: Option<ErrorInfo>,
}

impl Normalizer {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn turn_id(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    pub fn set_thread(&mut self, id: &str) {
        self.thread = Some(id.to_owned());
    }

    pub fn set_turn(&mut self, id: &str) {
        if self.turn.is_none() {
            self.turn = Some(id.to_owned());
        }
    }

    pub fn begin_turn(&mut self) {
        self.reset();
        self.active = true;
    }

    pub fn abort(&mut self, outcome: Outcome) -> Output {
        let mut out = Output::default();
        self.finish(outcome, &mut out);
        out
    }

    pub fn fail_with(&mut self, error: &ErrorInfo) -> Output {
        let failure = failure_from(Some(error), None);
        self.abort(Outcome::Failed(failure))
    }

    pub fn feed(&mut self, method: &str, params: &Value) -> Output {
        let mut out = Output::default();
        if !self.active {
            return out;
        }
        let params = Params::parse(params);
        if self.foreign(&params) {
            return out;
        }
        match method {
            "turn/started" => {
                if let Some(id) = params
                    .turn
                    .as_ref()
                    .and_then(|turn| turn.id.as_deref())
                    .or(params.turn_id.as_deref())
                {
                    self.set_turn(id);
                }
            }
            "item/agentMessage/delta" => {
                if let Some(text) = params.text() {
                    if let Some(id) = &params.item_id {
                        self.streamed.insert(id.clone());
                    }
                    self.emit_text(text, &mut out);
                }
            }
            "item/reasoning/summaryTextDelta" => self.thinking(&params, &mut out),
            "item/started" => {
                if let Some(item) = &params.item {
                    self.item_started(item, &mut out);
                }
            }
            "item/completed" => {
                if let Some(item) = &params.item {
                    self.item_completed(item, &mut out);
                }
            }
            "turn/completed" => self.turn_completed(&params, &mut out),
            "error" => self.error(&params, &mut out),
            _ => {}
        }
        out
    }

    fn foreign(&self, params: &Params) -> bool {
        if let (Some(ours), Some(theirs)) = (&self.thread, &params.thread_id)
            && ours != theirs
        {
            return true;
        }
        let theirs = params
            .turn
            .as_ref()
            .and_then(|turn| turn.id.as_ref())
            .or(params.turn_id.as_ref());
        matches!((&self.turn, theirs), (Some(ours), Some(theirs)) if ours != theirs)
    }

    fn emit_text(&mut self, text: &str, out: &mut Output) {
        let text = if self.separator_due {
            self.separator_due = false;
            format!("\n\n{text}")
        } else {
            text.to_owned()
        };
        out.events.push(Event::Text(text));
    }

    fn thinking(&mut self, params: &Params, out: &mut Output) {
        let Some(text) = params.text() else { return };
        let key = (
            params.item_id.clone().unwrap_or_default(),
            params.summary_index.unwrap_or(0),
        );
        let boundary = self.thinking_emitted && self.last_summary.as_ref() != Some(&key);
        self.last_summary = Some(key);
        self.thinking_emitted = true;
        let text = if boundary {
            format!("\n\n{text}")
        } else {
            text.to_owned()
        };
        out.events.push(Event::Thinking(text));
    }

    fn item_started(&mut self, item: &Item, out: &mut Output) {
        if !item.is("commandExecution", "command_execution") {
            return;
        }
        let Some(id) = item.id.as_deref().filter(|id| !id.is_empty()) else {
            return;
        };
        if !self.seen_tools.insert(id.to_owned()) {
            return;
        }
        self.open_tools.push(id.to_owned());
        out.events.push(Event::ToolStarted {
            id: id.to_owned(),
            title: tool_title(item),
        });
    }

    fn item_completed(&mut self, item: &Item, out: &mut Output) {
        if item.is("agentMessage", "agent_message") {
            self.message_completed(item, out);
        } else if item.is("commandExecution", "command_execution") {
            self.tool_completed(item, out);
        }
    }

    fn message_completed(&mut self, item: &Item, out: &mut Output) {
        let id = item.id.clone().unwrap_or_default();
        let text = item.text.as_deref().filter(|text| !text.is_empty());
        let streamed = self.streamed.contains(&id);
        if !streamed && let Some(text) = text {
            self.emit_text(text, out);
        }
        if streamed || text.is_some() {
            self.separator_due = true;
        }
    }

    fn tool_completed(&mut self, item: &Item, out: &mut Output) {
        let Some(id) = item.id.as_deref().filter(|id| !id.is_empty()) else {
            return;
        };
        let failed = matches!(item.status.as_deref(), Some("failed" | "declined"))
            || item.exit_code.is_some_and(|code| code != 0);
        if let Some(position) = self.open_tools.iter().position(|open| open == id) {
            self.open_tools.remove(position);
        } else if self.seen_tools.insert(id.to_owned()) {
            out.events.push(Event::ToolStarted {
                id: id.to_owned(),
                title: tool_title(item),
            });
        } else {
            return;
        }
        out.events.push(Event::ToolFinished {
            id: id.to_owned(),
            failed,
        });
    }

    fn turn_completed(&mut self, params: &Params, out: &mut Output) {
        let turn = params.turn.as_ref();
        let status = turn.and_then(|turn| turn.status.as_deref());
        let error = turn.and_then(|turn| turn.error.as_ref());
        let outcome = match status {
            Some("interrupted") => Outcome::Interrupted,
            Some("failed") => Outcome::Failed(failure_from(error, self.held_error.as_ref())),
            Some("completed") => Outcome::Completed,
            _ if error.is_some() || self.held_error.is_some() => {
                Outcome::Failed(failure_from(error, self.held_error.as_ref()))
            }
            _ => Outcome::Completed,
        };
        self.finish(outcome, out);
    }

    fn error(&mut self, params: &Params, out: &mut Output) {
        let info = params.error_info();
        if params.will_retry == Some(true) {
            let text = info.text().unwrap_or("Codex is retrying the request");
            out.events.push(Event::Notice(text.to_owned()));
        } else {
            self.held_error = Some(info);
        }
    }

    fn finish(&mut self, outcome: Outcome, out: &mut Output) {
        for id in self.open_tools.drain(..) {
            out.events.push(Event::ToolFinished { id, failed: true });
        }
        self.reset();
        out.events.push(Event::TurnFinished(outcome));
        out.finished = true;
    }

    fn reset(&mut self) {
        self.active = false;
        self.turn = None;
        self.streamed.clear();
        self.separator_due = false;
        self.last_summary = None;
        self.thinking_emitted = false;
        self.seen_tools.clear();
        self.held_error = None;
    }
}

pub fn failure_from(error: Option<&ErrorInfo>, held: Option<&ErrorInfo>) -> Failure {
    let code = error
        .and_then(|error| error.codex_error_info.as_ref())
        .or_else(|| held.and_then(|held| held.codex_error_info.as_ref()));
    let message = error
        .and_then(ErrorInfo::text)
        .or_else(|| held.and_then(ErrorInfo::text))
        .unwrap_or(GENERIC_FAILURE)
        .to_owned();
    Failure {
        kind: classify(code, &message),
        message,
    }
}

pub fn tool_title(item: &Item) -> String {
    let actions = item.action_commands().join(" && ");
    let command = if actions.is_empty() {
        shown_command(item.command.as_deref().unwrap_or_default())
    } else {
        actions
    };
    let title = one_line(&command);
    if title.is_empty() {
        "command".to_owned()
    } else {
        title
    }
}

pub fn shown_command(command: &str) -> String {
    let command = command.trim();
    let Some((program, rest)) = split_word(command) else {
        return String::new();
    };
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    if !SHELLS.contains(&name) {
        return command.to_owned();
    }
    let Some((flag, body)) = split_word(rest) else {
        return command.to_owned();
    };
    let is_command_flag = flag.len() >= 2
        && flag.starts_with('-')
        && flag.ends_with('c')
        && flag[1..].chars().all(|c| c.is_ascii_alphabetic());
    if !is_command_flag {
        return command.to_owned();
    }
    unquote(body.trim())
}

fn split_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    match text.find(char::is_whitespace) {
        Some(end) => Some((&text[..end], &text[end..])),
        None => Some((text, "")),
    }
}

fn unquote(body: &str) -> String {
    if body.len() >= 2 && body.starts_with('\'') && body.ends_with('\'') {
        body[1..body.len() - 1].replace("'\\''", "'")
    } else if body.len() >= 2 && body.starts_with('"') && body.ends_with('"') {
        body[1..body.len() - 1].replace("\\\"", "\"")
    } else {
        body.to_owned()
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
