use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use ::agent::brief::{self, Brief, Thread};
use ::agent::{Event, Failure, FailureKind, Outcome, prompt};
use app_db::AgentKind;
use ui_resources::i18n::Strings;

use crate::agent_profiles::LoginHint;

pub(super) const MAX_MESSAGE_CHARS: usize = 500;
pub(super) const MAX_BATCH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OverviewState {
    None,
    Current,
    Outdated { stored: String },
}

pub(super) fn overview_state(latest: Option<&str>, head: &str) -> OverviewState {
    match latest {
        None => OverviewState::None,
        Some(stored) if stored == head => OverviewState::Current,
        Some(stored) => OverviewState::Outdated {
            stored: stored.to_owned(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadAction {
    Nothing,
    Overview,
    Update,
}

pub(super) fn load_action(state: &OverviewState, auto: bool, local: bool) -> LoadAction {
    match state {
        OverviewState::None if auto => LoadAction::Overview,
        OverviewState::Outdated { .. } if auto && !local => LoadAction::Update,
        _ => LoadAction::Nothing,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Request {
    Overview,
    Update { old_head: String },
    Chat { prompt: String },
}

impl Request {
    pub(super) fn kind(&self) -> AgentKind {
        match self {
            Request::Overview | Request::Update { .. } => AgentKind::Overview,
            Request::Chat { .. } => AgentKind::Chat,
        }
    }

    pub(super) fn for_overview_button(state: &OverviewState) -> Request {
        match state {
            OverviewState::Outdated { stored } => Request::Update {
                old_head: stored.clone(),
            },
            _ => Request::Overview,
        }
    }
}

pub(super) fn compose(
    request: &Request,
    brief: &Brief,
    local: bool,
    head: &str,
    brief_included: bool,
    seen: &HashMap<String, u64>,
) -> String {
    match request {
        Request::Overview => prompt::with_brief(brief, &prompt::overview_task()),
        Request::Update { old_head } => {
            let task = if local {
                prompt::local_update_task()
            } else {
                prompt::update_task(old_head, head)
            };
            prompt::with_brief(brief, &task)
        }
        Request::Chat { prompt: text } => compose_chat(brief, text, brief_included, seen),
    }
}

pub(super) fn compose_chat(
    brief: &Brief,
    text: &str,
    brief_included: bool,
    seen: &HashMap<String, u64>,
) -> String {
    if brief_included {
        return prompt::with_brief(brief, text);
    }
    let changed = unseen_threads(brief, seen);
    prompt::with_new_comments(&changed, text)
}

pub(super) fn unseen_threads(brief: &Brief, seen: &HashMap<String, u64>) -> Vec<Thread> {
    brief::changed_since(&brief.threads, seen)
        .into_iter()
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Problem {
    NotLoggedIn,
    UsageLimit,
    ContextTooLong,
    ModelNotFound,
    Other(String),
}

pub(super) fn classify(failure: &Failure) -> Problem {
    match failure.kind {
        FailureKind::NotLoggedIn => Problem::NotLoggedIn,
        FailureKind::UsageLimit => Problem::UsageLimit,
        FailureKind::ContextTooLong => Problem::ContextTooLong,
        FailureKind::ModelNotFound => Problem::ModelNotFound,
        FailureKind::SessionLost | FailureKind::Other => Problem::Other(failure.message.clone()),
    }
}

pub(super) fn problem_text(problem: &Problem, login: Option<LoginHint>, s: &Strings) -> String {
    match problem {
        Problem::NotLoggedIn => match login {
            Some(LoginHint::Command(command)) => s.assistant_login_command(command),
            Some(LoginHint::EnvVar(name)) => s.assistant_login_env(name),
            None => s.assistant_login_generic.to_string(),
        },
        Problem::UsageLimit => s.assistant_usage_limit.to_string(),
        Problem::ContextTooLong => s.assistant_context_too_long.to_string(),
        Problem::ModelNotFound => s.assistant_model_not_found.to_string(),
        Problem::Other(message) => {
            let detail = limit_text(message);
            if detail.is_empty() {
                s.assistant_failed_plain.to_string()
            } else {
                s.assistant_failed(&detail)
            }
        }
    }
}

pub(super) fn exit_text(reason: Option<&str>, s: &Strings) -> String {
    let detail = reason.map(limit_text).unwrap_or_default();
    if detail.is_empty() {
        s.assistant_exited_plain.to_string()
    } else {
        s.assistant_exited(&detail)
    }
}

pub(super) fn limit_text(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_MESSAGE_CHARS) {
        None => text.to_owned(),
        Some((cut, _)) => format!("{}\u{2026}", text[..cut].trim_end()),
    }
}

pub(super) fn coalesce(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        match (out.last_mut(), event) {
            (Some(Event::Text(last)), Event::Text(next)) => last.push_str(&next),
            (Some(Event::Thinking(last)), Event::Thinking(next)) => last.push_str(&next),
            (_, event) => out.push(event),
        }
    }
    out
}

pub(super) fn is_frame_paced(event: &Event) -> bool {
    matches!(event, Event::Text(_) | Event::Thinking(_))
}

pub(super) type Write = Pin<Box<dyn Future<Output = ()> + Send>>;

pub(super) fn answered(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Completed | Outcome::Interrupted => true,
        Outcome::Cancelled | Outcome::Failed(_) => false,
    }
}

pub(super) fn auto_attempted_after(
    current: Option<&str>,
    request: &Request,
    turn_head: &str,
    outcome: &Outcome,
) -> Option<String> {
    let stopped_overview =
        matches!(outcome, Outcome::Cancelled) && request.kind() == AgentKind::Overview;
    if stopped_overview {
        Some(turn_head.to_owned())
    } else {
        current.map(str::to_owned)
    }
}

pub(super) fn brief_delivered(was: bool, carried: bool, outcome: &Outcome) -> bool {
    was || (carried && answered(outcome))
}

pub(super) fn resumed_delivered(resumed: bool, answered_before: bool) -> bool {
    resumed && answered_before
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Launching {
    pub profile: String,
    pub command: String,
    pub model: Option<String>,
}

impl Launching {
    pub(super) fn new(profile: &str, command: &str, model: Option<&str>) -> Self {
        Self {
            profile: profile.trim().to_owned(),
            command: command.trim().to_owned(),
            model: model
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned),
        }
    }
}

pub(super) fn settings_notice(used: bool, has_messages: bool, last_is_notice: bool) -> bool {
    used && has_messages && !last_is_notice
}

pub(super) struct Guard<T> {
    value: Option<T>,
    dispose: fn(T),
}

impl<T> Guard<T> {
    pub(super) fn new(value: T, dispose: fn(T)) -> Self {
        Self {
            value: Some(value),
            dispose,
        }
    }

    pub(super) fn take(&mut self) -> Option<T> {
        self.value.take()
    }
}

impl<T> Drop for Guard<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            (self.dispose)(value);
        }
    }
}
