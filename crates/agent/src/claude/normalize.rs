use std::collections::HashSet;

use serde_json::Value;

use super::wire::{Block, Frame};
use crate::{Event, Failure, FailureKind, Outcome};

const TITLE_CHARS: usize = 120;

#[derive(Debug, Default)]
pub struct Output {
    pub events: Vec<Event>,
    pub reply: Option<String>,
    pub finished: bool,
}

#[derive(Debug, Default)]
pub struct Normalizer {
    session: Option<String>,
    open_tools: Vec<String>,
    seen_tools: HashSet<String>,
    streamed_text: bool,
    streamed_thinking: bool,
    assistant_error: Option<String>,
    assistant_error_text: Option<String>,
    limit_rejected: bool,
}

impl Normalizer {
    pub fn feed(&mut self, line: &str) -> Output {
        let mut out = Output::default();
        let frame: Frame = match serde_json::from_str(line) {
            Ok(frame) => frame,
            Err(error) => {
                log::debug!(
                    "skipping an unreadable assistant line ({error}): {}",
                    preview(line)
                );
                if looks_like_result(line) {
                    let failure = Failure {
                        kind: FailureKind::Other,
                        message: "The assistant's final status could not be read.".to_owned(),
                    };
                    self.finish(Outcome::Failed(failure), &mut out);
                }
                return out;
            }
        };
        match frame.kind.as_deref() {
            Some("system") => self.system(&frame, &mut out),
            Some("stream_event") if !frame.is_sidechain() => self.stream_event(&frame, &mut out),
            Some("assistant") if !frame.is_sidechain() => self.assistant(&frame, &mut out),
            Some("user") if !frame.is_sidechain() => self.user(&frame, &mut out),
            Some("result") => {
                let outcome = self.outcome(&frame);
                self.finish(outcome, &mut out);
            }
            Some("rate_limit_event") => self.rate_limit(&frame),
            Some("control_request") => Self::control_request(&frame, &mut out),
            _ => {}
        }
        out
    }

    pub fn abort(&mut self, failure: Failure) -> Output {
        let mut out = Output::default();
        self.finish(Outcome::Failed(failure), &mut out);
        out
    }

    fn system(&mut self, frame: &Frame, out: &mut Output) {
        match frame.subtype.as_deref() {
            Some("init") => {
                if let Some(id) = frame.session_id.as_deref().filter(|id| !id.is_empty())
                    && self.session.as_deref() != Some(id)
                {
                    self.session = Some(id.to_owned());
                    out.events.push(Event::Session { id: id.to_owned() });
                }
            }
            Some("api_retry") => {
                let attempt = frame.attempt.unwrap_or(0);
                let mut text = match frame.max_retries {
                    Some(max) => format!("Retrying the request (attempt {attempt} of {max})"),
                    None => format!("Retrying the request (attempt {attempt})"),
                };
                if let Some(reason) = frame.error.as_ref().and_then(Value::as_str) {
                    text.push_str(": ");
                    text.push_str(reason);
                }
                out.events.push(Event::Notice(text));
            }
            _ => {}
        }
    }

    fn stream_event(&mut self, frame: &Frame, out: &mut Output) {
        let Some(event) = &frame.event else { return };
        match event.kind.as_deref() {
            Some("message_start") => {
                self.streamed_text = false;
                self.streamed_thinking = false;
            }
            Some("content_block_delta") => {
                let Some(delta) = &event.delta else { return };
                match delta.kind.as_deref() {
                    Some("text_delta") => {
                        if let Some(text) = delta.text.as_deref().filter(|text| !text.is_empty()) {
                            self.streamed_text = true;
                            out.events.push(Event::Text(text.to_owned()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) =
                            delta.thinking.as_deref().filter(|text| !text.is_empty())
                        {
                            self.streamed_thinking = true;
                            out.events.push(Event::Thinking(text.to_owned()));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn assistant(&mut self, frame: &Frame, out: &mut Output) {
        let Some(message) = &frame.message else {
            return;
        };
        let error = frame.error.as_ref().and_then(Value::as_str);
        if let Some(code) = error {
            self.assistant_error = Some(code.to_owned());
        }
        for block in &message.content {
            match block.kind.as_deref() {
                Some("text") => {
                    let text = block.text.as_deref().unwrap_or_default();
                    if error.is_some() {
                        if !text.is_empty() {
                            self.assistant_error_text = Some(text.to_owned());
                        }
                    } else if !self.streamed_text && !text.is_empty() {
                        out.events.push(Event::Text(text.to_owned()));
                    }
                }
                Some("thinking") => {
                    let text = block.thinking.as_deref().unwrap_or_default();
                    if !self.streamed_thinking && !text.is_empty() {
                        out.events.push(Event::Thinking(text.to_owned()));
                    }
                }
                Some("tool_use") => self.tool_started(block, out),
                _ => {}
            }
        }
    }

    fn tool_started(&mut self, block: &Block, out: &mut Output) {
        let Some(id) = block.id.as_deref().filter(|id| !id.is_empty()) else {
            return;
        };
        if !self.seen_tools.insert(id.to_owned()) {
            return;
        }
        self.open_tools.push(id.to_owned());
        let name = block.name.as_deref().unwrap_or("tool");
        out.events.push(Event::ToolStarted {
            id: id.to_owned(),
            title: tool_title(name, block.input.as_ref()),
        });
    }

    fn user(&mut self, frame: &Frame, out: &mut Output) {
        let Some(message) = &frame.message else {
            return;
        };
        for block in &message.content {
            if block.kind.as_deref() != Some("tool_result") {
                continue;
            }
            let Some(id) = block.tool_use_id.as_deref() else {
                continue;
            };
            if let Some(position) = self.open_tools.iter().position(|open| open == id) {
                self.open_tools.remove(position);
                out.events.push(Event::ToolFinished {
                    id: id.to_owned(),
                    failed: block.is_error.unwrap_or(false),
                });
            }
        }
    }

    fn rate_limit(&mut self, frame: &Frame) {
        let Some(info) = &frame.rate_limit_info else {
            return;
        };
        if info.status.as_deref() == Some("rejected") && info.is_using_overage != Some(true) {
            self.limit_rejected = true;
        }
    }

    fn control_request(frame: &Frame, out: &mut Output) {
        let Some(request_id) = frame.request_id.as_deref() else {
            return;
        };
        let subtype = frame
            .request
            .as_ref()
            .and_then(|request| request.subtype.as_deref());
        out.reply = Some(match subtype {
            Some("can_use_tool") => super::wire::deny_tool(request_id),
            _ => super::wire::unsupported_request(request_id),
        });
    }

    fn finish(&mut self, outcome: Outcome, out: &mut Output) {
        for id in self.open_tools.drain(..) {
            out.events.push(Event::ToolFinished { id, failed: true });
        }
        self.seen_tools.clear();
        self.streamed_text = false;
        self.streamed_thinking = false;
        self.assistant_error = None;
        self.assistant_error_text = None;
        self.limit_rejected = false;
        out.events.push(Event::TurnFinished(outcome));
        out.finished = true;
    }

    fn outcome(&self, frame: &Frame) -> Outcome {
        let reason = frame.terminal_reason.as_deref();
        if matches!(reason, Some("aborted_streaming" | "aborted_tools")) {
            return Outcome::Interrupted;
        }
        let failed = frame.is_error == Some(true)
            || frame
                .subtype
                .as_deref()
                .is_some_and(|subtype| subtype != "success");
        if !failed {
            return Outcome::Completed;
        }
        let message = self.failure_message(frame);
        let kind = self.classify(frame, &message);
        Outcome::Failed(Failure { kind, message })
    }

    fn failure_message(&self, frame: &Frame) -> String {
        let errors: Vec<&str> = frame
            .errors
            .iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|text| !text.is_empty())
            .collect();
        if !errors.is_empty() {
            return errors.join("\n");
        }
        if let Some(text) = frame
            .result
            .as_ref()
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            return text.to_owned();
        }
        if let Some(text) = &self.assistant_error_text {
            return text.clone();
        }
        match frame.subtype.as_deref() {
            Some(subtype) if subtype != "success" => {
                format!("The assistant stopped with an error ({subtype}).")
            }
            _ => "The assistant stopped with an error.".to_owned(),
        }
    }

    fn classify(&self, frame: &Frame, message: &str) -> FailureKind {
        let code = self.assistant_error.as_deref().unwrap_or_default();
        let status = frame.api_error_status.as_ref().and_then(Value::as_u64);
        let reason = frame.terminal_reason.as_deref().unwrap_or_default();
        let text = message.to_lowercase();

        if text.contains("no conversation found with session id") {
            FailureKind::SessionLost
        } else if status == Some(401)
            || code == "authentication_failed"
            || text.contains("not logged in")
            || text.contains("please run /login")
            || text.contains("invalid api key")
        {
            FailureKind::NotLoggedIn
        } else if status == Some(429) || code == "rate_limit" || self.limit_rejected {
            FailureKind::UsageLimit
        } else if matches!(reason, "prompt_too_long" | "blocking_limit")
            || text.contains("prompt is too long")
        {
            FailureKind::ContextTooLong
        } else if code == "model_not_found" || text.contains("issue with the selected model") {
            FailureKind::ModelNotFound
        } else {
            FailureKind::Other
        }
    }
}

pub fn looks_like_result(line: &str) -> bool {
    line.contains("\"type\":\"result\"") || line.contains("\"type\": \"result\"")
}

pub fn tool_title(name: &str, input: Option<&Value>) -> String {
    let field = |keys: &[&str]| -> Option<String> {
        let input = input?;
        keys.iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .map(str::to_owned)
    };
    let detail = match name {
        "Bash" => field(&["command"]),
        "Read" => field(&["file_path", "path"]),
        "Grep" | "Glob" => field(&["pattern"]),
        _ => None,
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
