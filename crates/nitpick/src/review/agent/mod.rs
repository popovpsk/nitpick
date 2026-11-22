mod claims;
mod context;
mod links;
mod logic;
mod messages;
mod panel;
mod run;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ::agent::Session;
use ::agent::process::{Environment, Invocation};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, SharedString, Subscription, Task, Window,
};
use gpui_component::input::{InputEvent, TextareaState};
use gpui_component::message_scroller::MessageScrollerState;

use crate::agent_profiles::Profile;
use crate::localization::tr;
use crate::settings_store::{SettingsStore, clamp_agent_panel_width, settings, update_settings};

pub(super) use context::{ReviewContext, from_view};
pub(super) use links::{
    LinkPlan, PendingSelection, Step, StopStep, can_link_selection, plan_link, selection_span,
};
pub(super) use panel::frame;
pub(super) use run::init;

use logic::{OverviewState, Request};
use messages::{Kind, Message};

pub(super) const SLIDE: Duration = Duration::from_millis(200);
const WIDTH_SAVE_DELAY: Duration = Duration::from_millis(400);

static WIDTH_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    Starting,
    Running,
    Ready,
    Failed(SharedString),
}

impl Phase {
    pub(super) fn is_busy(&self) -> bool {
        matches!(self, Phase::Starting | Phase::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lookup {
    Checking,
    Found,
    Missing,
}

pub(super) enum AgentEvent {
    OpenSettings,
    Stop(Step),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Activity {
    Working,
    Thinking,
    Tool(SharedString),
    Notice(SharedString),
}

struct Turn {
    request: Request,
    retried: bool,
    carried_brief: bool,
    carried_instructions: bool,
    sent: HashMap<String, u64>,
    head: String,
    assistant: Option<u64>,
    tools: Vec<(String, SharedString)>,
    activity: Activity,
}

struct Live {
    session: Session,
    cwd: PathBuf,
    scratch: PathBuf,
    brief_delivered: bool,
    instructions: String,
    instructions_due: Option<String>,
    _pump: Task<()>,
}

struct Bound {
    claim: claims::Claim,
    brief_delivered: bool,
    instructions: String,
}

impl Drop for Live {
    fn drop(&mut self) {
        run::retire(self.session.clone(), std::mem::take(&mut self.scratch));
    }
}

pub(super) struct AgentPanel {
    open: bool,
    closing: bool,
    toggles: usize,
    width: f32,
    phase: Phase,
    unread: bool,
    profile: Profile,
    profile_name: SharedString,
    command: String,
    model: Option<String>,
    instructions: String,
    lookup: Lookup,
    missing_hint: SharedString,
    lookup_generation: u64,
    context: Option<Rc<ReviewContext>>,
    loaded: bool,
    load_generation: u64,
    messages: Vec<Message>,
    next_message: u64,
    overview: OverviewState,
    auto_attempted: Option<String>,
    stale: bool,
    scroll: Entity<MessageScrollerState>,
    rows: usize,
    input: Entity<TextareaState>,
    pending: Option<PendingSelection>,
    stop_cursor: Option<usize>,
    stops_owner: Option<u64>,
    stops_label: Option<SharedString>,
    live: Option<Live>,
    generation: u64,
    operation: u64,
    turn: Option<Turn>,
    starting: Option<(Kind, String)>,
    seen: HashMap<String, u64>,
    owner: u64,
    bound: Option<Bound>,
    resumable: bool,
    settings_used: bool,
    settings_notice: Option<u64>,
    _close: Task<()>,
    _lookup: Task<()>,
    _load: Task<()>,
    _dispatch: Task<()>,
    _settings: Subscription,
    _input: Subscription,
}

impl EventEmitter<AgentEvent> for AgentPanel {}

impl AgentPanel {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let agent = &settings(cx).agent;
        let profile = agent.active_profile();
        let command = agent.command(&profile);
        let model = agent.model(&profile);
        let instructions = agent.instructions.trim().to_string();
        let width = clamp_agent_panel_width(agent.panel_width).round();
        let placeholder = tr().assistant_input_placeholder.clone();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .placeholder(placeholder)
                .submit_on_enter(true)
        });
        let submit = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.submit_input(window, cx);
            }
        });
        let scroll = cx.new(|cx| MessageScrollerState::new(0, cx));
        Self {
            open: false,
            closing: false,
            toggles: 0,
            width,
            phase: Phase::Idle,
            unread: false,
            profile_name: profile.name.clone().into(),
            profile,
            missing_hint: tr().assistant_missing_hint(&command).into(),
            command,
            model,
            instructions,
            lookup: Lookup::Checking,
            lookup_generation: 0,
            context: None,
            loaded: false,
            load_generation: 0,
            messages: Vec::new(),
            next_message: 0,
            overview: OverviewState::None,
            auto_attempted: None,
            stale: false,
            scroll,
            rows: 0,
            input,
            pending: None,
            stop_cursor: None,
            stops_owner: None,
            stops_label: None,
            live: None,
            generation: 0,
            operation: 0,
            turn: None,
            starting: None,
            seen: HashMap::new(),
            owner: claims::next_owner(),
            bound: None,
            resumable: false,
            settings_used: false,
            settings_notice: None,
            _close: Task::ready(()),
            _lookup: Task::ready(()),
            _load: Task::ready(()),
            _dispatch: Task::ready(()),
            _settings: cx.observe_global::<SettingsStore>(|this, cx| this.apply_settings(cx)),
            _input: submit,
        }
    }

    pub(super) fn is_busy(&self) -> bool {
        self.phase.is_busy()
    }

    pub(super) fn has_unread(&self) -> bool {
        self.unread && !self.open
    }

    pub(super) fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
        } else {
            self.open(cx);
        }
    }

    pub(super) fn open(&mut self, cx: &mut Context<Self>) {
        if self.open || !settings(cx).agent.enabled {
            return;
        }
        self.open = true;
        self.closing = false;
        self.toggles += 1;
        self.unread = false;
        self._close = Task::ready(());
        self.start_lookup(cx);
        cx.notify();
    }

    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.closing = true;
        self.toggles += 1;
        self._close = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SLIDE).await;
            let _ = this.update(cx, |this, cx| {
                this.closing = false;
                cx.notify();
            });
        });
        cx.notify();
    }

    pub(super) fn resize(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = clamp_agent_panel_width(width).round();
        if (width - self.width).abs() < 0.5 {
            return;
        }
        self.width = width;
        let generation = WIDTH_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(WIDTH_SAVE_DELAY).await;
            if WIDTH_GENERATION.load(Ordering::Relaxed) == generation {
                cx.update(|cx| update_settings(cx, |s| s.agent.panel_width = width));
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn can_send(&self) -> bool {
        self.context.is_some()
            && self.loaded
            && !self.phase.is_busy()
            && self.lookup != Lookup::Missing
            && !self.command.trim().is_empty()
    }

    pub(super) fn can_request_overview(&self) -> bool {
        self.can_send()
    }

    pub(super) fn set_phase(&mut self, phase: Phase, cx: &mut Context<Self>) {
        self.unread = unread_after(self.unread, self.open, &phase);
        self.phase = phase;
        self.sync_rows(cx);
        cx.notify();
    }

    pub(super) fn start_lookup(&mut self, cx: &mut Context<Self>) {
        self.lookup_generation += 1;
        let generation = self.lookup_generation;
        let command = self.command.clone();
        self.lookup = Lookup::Checking;
        self._lookup = cx.spawn(async move |this, cx| {
            let found = net_runtime::unblock(move || locate(&command)).await;
            let _ = this.update(cx, |this, cx| {
                if this.lookup_generation != generation {
                    return;
                }
                this.lookup = found;
                if found == Lookup::Missing {
                    log::warn!("the assistant's command `{}` was not found", this.command);
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn launching(&self) -> logic::Launching {
        logic::Launching::new(
            &self.profile.id,
            &self.command,
            self.model.as_deref(),
            &self.instructions,
        )
    }

    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let before = self.launching();
        let agent = &settings(cx).agent;
        let profile = agent.active_profile();
        let command = agent.command(&profile);
        let model = agent.model(&profile);
        let instructions = agent.instructions.trim().to_string();
        let enabled = agent.enabled;
        self.profile_name = profile.name.clone().into();
        self.profile = profile;
        self.command = command;
        self.model = model;
        self.instructions = instructions;
        self.missing_hint = tr().assistant_missing_hint(&self.command).into();
        if !enabled {
            self.close(cx);
        }
        let after = self.launching();
        if before != after {
            let used = self.settings_used || self.live.is_some() || self.phase.is_busy();
            let last_is_notice = self.settings_notice.is_some()
                && self.messages.last().map(|message| message.id) == self.settings_notice;
            let notice = logic::settings_notice(used, !self.messages.is_empty(), last_is_notice);
            self.settings_used = false;
            self.end_process(cx);
            if before.profile != after.profile {
                self.bound = None;
                self.resumable = false;
                self.load_stored(!self.loaded, cx);
            }
            self.auto_attempted = None;
            if notice {
                let text = if before.profile != after.profile {
                    tr().assistant_notice_agent(&self.profile.name)
                } else {
                    tr().assistant_notice_settings.to_string()
                };
                self.add_notice(text, cx);
                self.settings_notice = self.messages.last().map(|message| message.id);
            }
            self.start_lookup(cx);
        }
        cx.notify();
    }
}

pub(super) fn unread_after(unread: bool, open: bool, phase: &Phase) -> bool {
    match phase {
        Phase::Ready => unread || !open,
        phase if phase.is_busy() => false,
        _ => unread,
    }
}

pub(super) fn locate(command: &str) -> Lookup {
    locate_in(command, Environment::current())
}

pub(super) fn locate_in(command: &str, env: Environment) -> Lookup {
    match Invocation::resolve_in(command, Vec::new(), &[], env, Path::new(".")) {
        Ok(invocation) if invocation.program.is_file() => Lookup::Found,
        _ => Lookup::Missing,
    }
}

#[cfg(test)]
mod tests;
