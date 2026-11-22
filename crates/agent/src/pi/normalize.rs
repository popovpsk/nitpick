use std::collections::HashSet;

use serde_json::Value;

use super::classify;
use super::wire::{self, Frame, State};
use crate::{Event, Failure, Outcome};

pub const STATE_ID: &str = "nitpick-state";

const TITLE_CHARS: usize = 120;
const GENERIC_ERROR: &str = "The assistant stopped with an error.";
const SILENT_METHODS: &[&str] = &[
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
];

#[derive(Debug, Default)]
pub struct Output {
    pub events: Vec<Event>,
    pub reply: Option<String>,
    pub state: Option<Result<State, Failure>>,
    pub finished: bool,
}

#[derive(Debug, Default)]
struct Turn {
    prompt_id: String,
    open_tools: Vec<String>,
    seen_tools: HashSet<String>,
    streamed_text: bool,
    streamed_thinking: bool,
    error: Option<String>,
    aborted: bool,
}

#[derive(Debug, Default)]
pub struct Normalizer {
    turn: Option<Turn>,
}

impl Normalizer {
    pub fn begin_turn(&mut self, prompt_id: &str) {
        self.turn = Some(Turn {
            prompt_id: prompt_id.to_owned(),
            ..Turn::default()
        });
    }

    pub fn active(&self) -> bool {
        self.turn.is_some()
    }

    pub fn end(&mut self, outcome: Outcome) -> Output {
        let mut out = Output::default();
        self.finish(outcome, &mut out);
        out
    }

    pub fn feed(&mut self, line: &str) -> Output {
        let mut out = Output::default();
        let frame: Frame = match serde_json::from_str(line) {
            Ok(frame) => frame,
            Err(error) => {
                log::debug!(
                    "skipping an unreadable assistant line ({error}): {}",
                    preview(line)
                );
                return out;
            }
        };
        match frame.kind.as_deref() {
            Some("extension_ui_request") => out.reply = dialog_reply(&frame),
            Some("response") => self.response(&frame, &mut out),
            Some("message_start") => {
                if let Some(turn) = self.turn.as_mut() {
                    turn.streamed_text = false;
                    turn.streamed_thinking = false;
                }
            }
            Some("message_update") => self.message_update(&frame, &mut out),
            Some("message_end") => self.message_end(&frame, &mut out),
            Some("tool_execution_start") => self.tool_started(&frame, &mut out),
            Some("tool_execution_end") => self.tool_finished(&frame, &mut out),
            Some("auto_retry_start") => self.retry_started(&frame, &mut out),
            Some("auto_retry_end") => self.retry_ended(&frame),
            Some("agent_settled") => self.settled(&frame, &mut out),
            Some("extension_error") => log::debug!(
                "a Pi extension failed: {}",
                frame.error_text().unwrap_or_default()
            ),
            _ => {}
        }
        out
    }

    fn response(&mut self, frame: &Frame, out: &mut Output) {
        match frame.command.as_deref() {
            Some("get_state") if frame.id_str() == Some(STATE_ID) => {
                out.state = Some(if frame.success == Some(false) {
                    Err(failure(frame.error_text()))
                } else {
                    Ok(State::from_data(frame.data.as_ref()))
                });
            }
            Some("prompt") => {
                let Some(turn) = self.turn.as_ref() else {
                    return;
                };
                if frame.id_str() != Some(turn.prompt_id.as_str()) {
                    return;
                }
                if frame.success == Some(false) {
                    self.finish(Outcome::Failed(failure(frame.error_text())), out);
                } else if frame.disposition() == Some("handled") {
                    self.finish(Outcome::Completed, out);
                }
            }
            command => {
                if frame.success == Some(false) {
                    log::debug!(
                        "Pi refused {}: {}",
                        command.unwrap_or("a command"),
                        frame.error_text().unwrap_or_default()
                    );
                }
            }
        }
    }

    fn message_update(&mut self, frame: &Frame, out: &mut Output) {
        let (Some(turn), Some(update)) = (self.turn.as_mut(), frame.update.as_ref()) else {
            return;
        };
        let Some(delta) = update.delta.as_deref().filter(|delta| !delta.is_empty()) else {
            return;
        };
        match update.kind.as_deref() {
            Some("text_delta") => {
                turn.streamed_text = true;
                out.events.push(Event::Text(delta.to_owned()));
            }
            Some("thinking_delta") => {
                turn.streamed_thinking = true;
                out.events.push(Event::Thinking(delta.to_owned()));
            }
            _ => {}
        }
    }

    fn message_end(&mut self, frame: &Frame, out: &mut Output) {
        let (Some(turn), Some(message)) = (self.turn.as_mut(), frame.message.as_ref()) else {
            return;
        };
        let stop = message.stop_reason.as_deref();
        match stop {
            Some("error") => {
                turn.error = Some(
                    message
                        .error_message
                        .clone()
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| GENERIC_ERROR.to_owned()),
                );
                turn.aborted = false;
            }
            Some("aborted") => {
                turn.error = None;
                turn.aborted = true;
            }
            Some(_) => {
                turn.error = None;
                turn.aborted = false;
            }
            None => {}
        }
        if message.role.as_deref() != Some("assistant") || matches!(stop, Some("error" | "aborted"))
        {
            return;
        }
        for part in &message.content {
            match part.kind.as_deref() {
                Some("text") if !turn.streamed_text => {
                    if let Some(text) = part.text.as_deref().filter(|text| !text.is_empty()) {
                        out.events.push(Event::Text(text.to_owned()));
                    }
                }
                Some("thinking") if !turn.streamed_thinking => {
                    if let Some(text) = part.thinking.as_deref().filter(|text| !text.is_empty()) {
                        out.events.push(Event::Thinking(text.to_owned()));
                    }
                }
                _ => {}
            }
        }
        turn.streamed_text = false;
        turn.streamed_thinking = false;
    }

    fn tool_started(&mut self, frame: &Frame, out: &mut Output) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        let Some(id) = frame.tool_call_id.as_deref().filter(|id| !id.is_empty()) else {
            return;
        };
        if !turn.seen_tools.insert(id.to_owned()) {
            return;
        }
        turn.open_tools.push(id.to_owned());
        let name = frame.tool_name.as_deref().unwrap_or("tool");
        out.events.push(Event::ToolStarted {
            id: id.to_owned(),
            title: tool_title(name, frame.args.as_ref()),
        });
    }

    fn tool_finished(&mut self, frame: &Frame, out: &mut Output) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        let Some(id) = frame.tool_call_id.as_deref() else {
            return;
        };
        if let Some(position) = turn.open_tools.iter().position(|open| open == id) {
            turn.open_tools.remove(position);
            out.events.push(Event::ToolFinished {
                id: id.to_owned(),
                failed: frame.is_error.unwrap_or(false),
            });
        }
    }

    fn retry_started(&mut self, frame: &Frame, out: &mut Output) {
        if self.turn.is_none() {
            return;
        }
        let attempt = frame.attempt.unwrap_or(0);
        let mut text = match frame.max_attempts {
            Some(max) => format!("Retrying the request (attempt {attempt} of {max})"),
            None => format!("Retrying the request (attempt {attempt})"),
        };
        if let Some(reason) = frame
            .error_message
            .as_deref()
            .map(one_line)
            .filter(|reason| !reason.is_empty())
        {
            text.push_str(": ");
            text.push_str(&reason);
        }
        out.events.push(Event::Notice(text));
    }

    fn retry_ended(&mut self, frame: &Frame) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        if frame.success == Some(false)
            && turn.error.is_none()
            && let Some(error) = frame.final_error.as_deref().filter(|text| !text.is_empty())
        {
            turn.error = Some(error.to_owned());
        }
    }

    fn settled(&mut self, frame: &Frame, out: &mut Output) {
        let Some(turn) = self.turn.as_ref() else {
            return;
        };
        let outcome = if frame.aborted == Some(true) || turn.aborted {
            Outcome::Interrupted
        } else if let Some(message) = &turn.error {
            Outcome::Failed(Failure {
                kind: classify::kind(message),
                message: message.clone(),
            })
        } else {
            Outcome::Completed
        };
        self.finish(outcome, out);
    }

    fn finish(&mut self, outcome: Outcome, out: &mut Output) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        for id in turn.open_tools {
            out.events.push(Event::ToolFinished { id, failed: true });
        }
        out.events.push(Event::TurnFinished(outcome));
        out.finished = true;
    }
}

fn failure(error: Option<String>) -> Failure {
    let message = error.unwrap_or_else(|| GENERIC_ERROR.to_owned());
    Failure {
        kind: classify::kind(&message),
        message,
    }
}

fn dialog_reply(frame: &Frame) -> Option<String> {
    let id = frame.id.as_ref().filter(|id| !id.is_null())?;
    if frame
        .method
        .as_deref()
        .is_some_and(|method| SILENT_METHODS.contains(&method))
    {
        return None;
    }
    Some(wire::cancel_dialog(id))
}

pub fn tool_title(name: &str, args: Option<&Value>) -> String {
    let field = |keys: &[&str]| -> Option<String> {
        let args = args?;
        keys.iter()
            .find_map(|key| args.get(*key).and_then(Value::as_str))
            .map(str::to_owned)
    };
    let detail = match name {
        "bash" => field(&["command"]),
        "grep" | "find" => field(&["pattern", "path"]),
        _ => field(&["path", "pattern"]),
    };
    match detail
        .map(|text| one_line(&text))
        .filter(|text| !text.is_empty())
    {
        Some(text) => text,
        None => name.to_owned(),
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

fn preview(line: &str) -> String {
    line.chars().take(200).collect()
}
