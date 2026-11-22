mod panel;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ::agent::process::{Environment, Invocation};
use gpui::{Context, EventEmitter, SharedString, Subscription, Task};

use crate::localization::tr;
use crate::settings_store::{SettingsStore, clamp_agent_panel_width, settings, update_settings};

pub(super) use panel::frame;

pub(super) const SLIDE: Duration = Duration::from_millis(200);
const WIDTH_SAVE_DELAY: Duration = Duration::from_millis(400);

static WIDTH_GENERATION: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
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
}

pub(super) struct AgentPanel {
    open: bool,
    closing: bool,
    toggles: usize,
    width: f32,
    phase: Phase,
    unread: bool,
    profile_id: String,
    profile_name: SharedString,
    command: String,
    lookup: Lookup,
    missing_hint: SharedString,
    lookup_generation: u64,
    _close: Task<()>,
    _lookup: Task<()>,
    _settings: Subscription,
}

impl EventEmitter<AgentEvent> for AgentPanel {}

impl AgentPanel {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let agent = &settings(cx).agent;
        let profile = agent.active_profile();
        let command = agent.command(&profile);
        let width = clamp_agent_panel_width(agent.panel_width).round();
        Self {
            open: false,
            closing: false,
            toggles: 0,
            width,
            phase: Phase::Idle,
            unread: false,
            profile_id: profile.id,
            profile_name: profile.name.into(),
            missing_hint: tr().assistant_missing_hint(&command).into(),
            command,
            lookup: Lookup::Checking,
            lookup_generation: 0,
            _close: Task::ready(()),
            _lookup: Task::ready(()),
            _settings: cx.observe_global::<SettingsStore>(|this, cx| this.apply_settings(cx)),
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

    pub(super) fn can_request_overview(&self) -> bool {
        false
    }

    pub(super) fn request_overview(&mut self, _cx: &mut Context<Self>) {}

    #[allow(dead_code)]
    pub(super) fn set_phase(&mut self, phase: Phase, cx: &mut Context<Self>) {
        self.unread = unread_after(self.unread, self.open, &phase);
        self.phase = phase;
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

    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let agent = &settings(cx).agent;
        let profile = agent.active_profile();
        let command = agent.command(&profile);
        let enabled = agent.enabled;
        let changed = profile.id != self.profile_id || command != self.command;
        self.profile_id = profile.id;
        self.profile_name = profile.name.into();
        self.command = command;
        self.missing_hint = tr().assistant_missing_hint(&self.command).into();
        if !enabled {
            self.close(cx);
        }
        if changed {
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
