use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use ::agent::{Error, Event, Launch, Outcome, Session, brief, prompt};
use app_db::AgentSession;
use gpui::{App, AppContext as _, AsyncApp, Context, SharedString, Task, WeakEntity, Window};

use super::claims::{self, Claim};
use super::context::{self, ReviewContext, Workdir};
use super::links;
use super::logic::{self, OverviewState, Request, Resume};
use super::messages::{Kind, Message, Role};
use super::{Activity, AgentPanel, Bound, Live, Lookup, Phase, Turn};
use crate::localization::{self, tr};
use crate::recent::now;
use crate::settings_store::{app_dir, settings};

const FLUSH: Duration = Duration::from_millis(16);
const SCRATCH_TTL: Duration = Duration::from_secs(24 * 60 * 60);

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Parts {
    session: Session,
    receiver: flume::Receiver<Event>,
    resume: Option<Resume>,
    instructions: String,
    scratch: PathBuf,
}

type Started = logic::Guard<Parts>;

fn dispose_parts(parts: Parts) {
    retire(parts.session, parts.scratch, None);
}

pub(in crate::review) fn init(cx: &mut App) {
    ::agent::process::prewarm();
    if let Some(db) = crate::db::db(cx) {
        let pruned = db.prune_agent(now());
        cx.background_spawn(async move {
            if let Err(err) = pruned.await {
                log::debug!("assistant history: {err}");
            }
        })
        .detach();
    }
    cx.background_spawn(async { sweep_scratch(&scratch_root()) })
        .detach();
}

fn scratch_root() -> PathBuf {
    app_dir(dirs::data_dir()).join("agent")
}

fn scratch_dir() -> PathBuf {
    let counter = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    scratch_root().join(format!("{:x}-{}-{counter}", now(), std::process::id()))
}

fn sweep_scratch(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > SCRATCH_TTL);
        if stale && let Err(err) = std::fs::remove_dir_all(entry.path()) {
            log::debug!("{}: {err}", entry.path().display());
        }
    }
}

pub(super) fn retire(session: Session, scratch: PathBuf, claim: Option<Claim>) {
    let done = session.shutdown();
    net_runtime::handle().spawn(async move {
        done.await;
        drop(session);
        drop(claim);
        let removed = net_runtime::unblock(move || std::fs::remove_dir_all(&scratch)).await;
        if let Err(err) = removed
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::debug!("assistant scratch folder: {err}");
        }
    });
}

async fn pump(
    this: WeakEntity<AgentPanel>,
    cx: &mut AsyncApp,
    receiver: flume::Receiver<Event>,
    generation: u64,
) {
    let mut closed = true;
    while let Ok(first) = receiver.recv_async().await {
        if logic::is_frame_paced(&first) {
            cx.background_executor().timer(FLUSH).await;
        }
        let mut batch = vec![first];
        while batch.len() < logic::MAX_BATCH
            && let Ok(next) = receiver.try_recv()
        {
            batch.push(next);
        }
        let batch = logic::coalesce(batch);
        let alive = this.update(cx, |this, cx| this.apply_events(generation, batch, cx));
        if !matches!(alive, Ok(true)) {
            closed = false;
            break;
        }
    }
    if closed {
        let _ = this.update(cx, |this, cx| {
            this.apply_events(generation, vec![Event::Exited(None)], cx)
        });
    }
}

impl AgentPanel {
    pub(in crate::review) fn set_review(
        &mut self,
        context: Rc<ReviewContext>,
        cx: &mut Context<Self>,
    ) {
        let same = self
            .context
            .as_ref()
            .is_some_and(|old| old.repo == context.repo && old.key == context.key);
        let head_changed = self
            .context
            .as_ref()
            .is_some_and(|old| old.head != context.head);
        if !same {
            self.end_process(cx);
            self.settings_used = false;
            self.settings_notice = None;
        }
        self.context = Some(context);
        if !same {
            self.reset_conversation(cx);
            cx.notify();
        } else if head_changed {
            self.evaluate(cx);
            cx.notify();
        }
    }

    fn retire_live(&mut self) {
        self.generation += 1;
        self.live = None;
    }

    pub(super) fn end_process(&mut self, cx: &mut Context<Self>) {
        self.operation += 1;
        self.starting = None;
        self._dispatch = Task::ready(());
        if let Some(turn) = self.turn.take() {
            self.finish_assistant(turn.assistant, cx);
        }
        self.retire_live();
        if self.phase != Phase::Idle {
            self.set_phase(Phase::Idle, cx);
        }
    }

    fn reset_conversation(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        self.rows = 0;
        self.scroll.update(cx, |scroll, cx| scroll.reset(0, cx));
        self.loaded = false;
        self.overview = OverviewState::None;
        self.auto_attempted = None;
        self.stale = false;
        self.seen.clear();
        self.bound = None;
        self.resumable = false;
        self.settings_notice = None;
        self.pending = None;
        self.load_stored(true, cx);
    }

    pub(super) fn load_stored(&mut self, evaluate: bool, cx: &mut Context<Self>) {
        self.load_generation += 1;
        let generation = self.load_generation;
        let Some(context) = self.context.clone() else {
            return;
        };
        let Some(db) = crate::db::db(cx) else {
            self.install_stored(generation, None, evaluate, cx);
            return;
        };
        let load = db.agent_session(&context.repo, &context.key, &self.profile.id);
        self._load = cx.spawn(async move |this, cx| {
            let stored = load.await.unwrap_or_else(|err| {
                log::warn!("loading the assistant's session: {err}");
                None
            });
            let _ = this.update(cx, |this, cx| {
                this.install_stored(generation, stored, evaluate, cx)
            });
        });
    }

    fn install_stored(
        &mut self,
        generation: u64,
        stored: Option<AgentSession>,
        evaluate: bool,
        cx: &mut Context<Self>,
    ) {
        if generation != self.load_generation {
            return;
        }
        self.resumable = self.bound.is_none()
            && stored
                .as_ref()
                .is_some_and(|stored| !claims::is_held(&stored.id));
        self.loaded = true;
        if evaluate {
            self.evaluate(cx);
        }
        cx.notify();
    }

    fn refresh_overview(&mut self) {
        let Some(context) = &self.context else {
            self.overview = OverviewState::None;
            return;
        };
        let latest = self
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant && message.kind == Kind::Overview)
            .map(|message| message.head.as_str());
        self.overview = logic::overview_state(latest, &context.head);
    }

    fn evaluate(&mut self, cx: &mut Context<Self>) {
        self.refresh_overview();
        let Some(context) = self.context.clone() else {
            return;
        };
        if !self.loaded {
            return;
        }
        if self.phase.is_busy() {
            self.stale = true;
            return;
        }
        if self.auto_attempted.as_deref() == Some(context.head.as_str()) {
            return;
        }
        let agent = &settings(cx).agent;
        let auto = agent.enabled && agent.auto_overview;
        let resumable = self.resumable && self.messages.is_empty();
        let request = match logic::load_action(&self.overview, auto, context.local, resumable) {
            logic::LoadAction::Nothing => return,
            logic::LoadAction::Overview => Request::Overview,
            logic::LoadAction::Update => Request::for_overview_button(&self.overview),
        };
        self.auto_attempted = Some(context.head.clone());
        if self.lookup != Lookup::Missing {
            self.dispatch(request, false, cx);
        }
    }

    pub(super) fn request_overview(&mut self, cx: &mut Context<Self>) {
        if !self.can_request_overview() {
            return;
        }
        let request = Request::for_overview_button(&self.overview);
        self.dispatch(request, false, cx);
    }

    pub(super) fn submit_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || !self.can_send() {
            return;
        }
        let question = links::compose_question(&text, self.pending.as_ref());
        self.pending = None;
        self.input.update(cx, |input, cx| input.clean(window, cx));
        self.send_chat(question.shown, question.prompt, cx);
    }

    pub(super) fn send_chat(&mut self, shown: String, prompt: String, cx: &mut Context<Self>) {
        if !self.can_send() || shown.trim().is_empty() {
            return;
        }
        let Some(head) = self.context.as_ref().map(|context| context.head.clone()) else {
            return;
        };
        self.push_message(Role::User, Kind::Chat, head, shown, cx);
        self.dispatch(Request::Chat { prompt }, false, cx);
    }

    pub(super) fn interrupt(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            Phase::Starting => {
                self.operation += 1;
                self._dispatch = Task::ready(());
                if let Some((kind, head)) = self.starting.take() {
                    self.auto_attempted = logic::auto_attempted_after(
                        self.auto_attempted.as_deref(),
                        kind,
                        &head,
                        &Outcome::Cancelled,
                    );
                }
                self.set_phase(Phase::Idle, cx);
                self.after_turn(cx);
            }
            Phase::Running => {
                if let Some(live) = &self.live {
                    live.session.interrupt();
                }
            }
            _ => {}
        }
    }

    pub(super) fn clear_conversation(&mut self, cx: &mut Context<Self>) {
        self.end_process(cx);
        self.load_generation += 1;
        self.bound = None;
        self.resumable = false;
        if let Some(context) = self.context.clone()
            && let Some(db) = crate::db::db(cx)
        {
            let profiles: Vec<String> = settings(cx)
                .agent
                .profiles()
                .iter()
                .map(|profile| profile.id.clone())
                .collect();
            let (repo, key) = (context.repo.clone(), context.key.clone());
            cx.background_spawn(async move {
                for profile in profiles {
                    let stored = db
                        .agent_session(&repo, &key, &profile)
                        .await
                        .unwrap_or_else(|err| {
                            log::warn!("loading the assistant's session: {err}");
                            None
                        });
                    let Some(id) = logic::clearable(stored.as_ref()) else {
                        continue;
                    };
                    let cleared = db.clear_agent_session(&repo, &key, &profile, Some(&id));
                    if let Err(err) = cleared.await {
                        log::warn!("clearing the assistant's sessions: {err}");
                    }
                }
            })
            .detach();
            self.auto_attempted = Some(context.head.clone());
        }
        self.messages.clear();
        self.rows = 0;
        self.scroll.update(cx, |scroll, cx| scroll.reset(0, cx));
        self.overview = OverviewState::None;
        self.seen.clear();
        self.settings_notice = None;
        self.stale = false;
        self.pending = None;
        cx.notify();
    }

    fn dispatch(&mut self, request: Request, retried: bool, cx: &mut Context<Self>) {
        let Some(context) = self.context.clone() else {
            return;
        };
        self.operation += 1;
        self.settings_used = true;
        let operation = self.operation;
        let chat = matches!(request, Request::Chat { .. });
        let live_delivered = self.live.as_ref().map(|live| live.brief_delivered);
        if logic::goes_out_at_once(chat, live_delivered) {
            self.send_now(request, retried, None, cx);
            return;
        }
        let kind = self.profile.kind;
        let command = self.command.clone();
        let model = self.model.clone();
        let profile_id = self.profile.id.clone();
        let language = context::language_name(localization::resolve(cx));
        let user_instructions = self.instructions.clone();
        let owner = self.owner;
        let own = self.bound.as_ref().map(|bound| Resume {
            id: bound.claim.id().to_owned(),
            brief_delivered: bound.brief_delivered,
            brief_head: bound.brief_head.clone(),
            instructions: bound.instructions.clone(),
        });
        let fixed_cwd = self.live.as_ref().map(|live| live.cwd.clone());
        let need_start = self.live.is_none();
        let scratch = scratch_dir();
        let db = crate::db::db(cx);
        self.starting = Some((request.kind(), context.head.clone()));
        self.set_phase(Phase::Starting, cx);
        self._dispatch = cx.spawn(async move |this, cx| {
            let plan = context.workdir.clone();
            let workdir = cx
                .background_spawn(
                    async move { context::resolve_workdir(&plan, fixed_cwd.as_deref()) },
                )
                .await;
            let started = if need_start {
                let resume = match own {
                    Some(own) => Some(own),
                    None => {
                        let stored = match &db {
                            Some(db) => db
                                .agent_session(&context.repo, &context.key, &profile_id)
                                .await
                                .unwrap_or_else(|err| {
                                    log::warn!("loading the assistant's session: {err}");
                                    None
                                }),
                            None => None,
                        };
                        match logic::adopt(stored, |id| Claim::acquire(id, owner)) {
                            Some((resume, claim)) => {
                                let adopted = resume.clone();
                                let kept = this.update(cx, |this, _| {
                                    let bound = Bound {
                                        claim,
                                        brief_delivered: adopted.brief_delivered,
                                        brief_head: adopted.brief_head,
                                        instructions: adopted.instructions,
                                    };
                                    logic::hand_over(
                                        &mut this.bound,
                                        this.operation,
                                        operation,
                                        bound,
                                    )
                                });
                                if matches!(kept, Ok(true)) {
                                    let _ = this.update(cx, |this, _| this.resumable = false);
                                    Some(resume)
                                } else {
                                    None
                                }
                            }
                            None => None,
                        }
                    }
                };
                let instructions =
                    prompt::instructions(language, workdir.disk_matches_head, &user_instructions);
                let launch = Launch {
                    kind,
                    command,
                    cwd: workdir.cwd.clone(),
                    model,
                    resume: resume.as_ref().map(|resume| resume.id.clone()),
                    instructions: instructions.clone(),
                    scratch: scratch.clone(),
                };
                let started = net_runtime::unblock(move || {
                    Session::start(launch).map(|(session, receiver)| {
                        Started::new(
                            Parts {
                                session,
                                receiver,
                                resume,
                                instructions,
                                scratch,
                            },
                            dispose_parts,
                        )
                    })
                })
                .await;
                Some(started)
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                this.dispatched(operation, request, retried, workdir, started, cx)
            });
        });
    }

    fn dispatched(
        &mut self,
        operation: u64,
        request: Request,
        retried: bool,
        workdir: Workdir,
        started: Option<Result<Started, Error>>,
        cx: &mut Context<Self>,
    ) {
        if operation != self.operation {
            return;
        }
        self.starting = None;
        if logic::must_start(self.live.is_some(), started.is_some()) {
            self.dispatch(request, retried, cx);
            return;
        }
        if let Some(started) = started {
            match started {
                Ok(mut started) => {
                    if let Some(parts) = started.take() {
                        self.install_live(parts, workdir.cwd.clone(), cx);
                    }
                }
                Err(err) => {
                    self.start_failed(err, cx);
                    return;
                }
            }
        }
        self.send_now(request, retried, Some(workdir.disk_matches_head), cx);
    }

    fn start_failed(&mut self, err: Error, cx: &mut Context<Self>) {
        log::warn!("starting the assistant: {err}");
        match err {
            Error::NotFound(_) => {
                self.lookup = Lookup::Missing;
                self.set_phase(Phase::Idle, cx);
            }
            Error::Unsupported => {
                let text = tr().assistant_unsupported.clone();
                self.set_phase(Phase::Failed(text), cx);
            }
            err => {
                let text = tr().assistant_start_failed(&logic::limit_text(&err.to_string()));
                self.set_phase(Phase::Failed(text.into()), cx);
            }
        }
    }

    fn install_live(&mut self, parts: Parts, cwd: PathBuf, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let Parts {
            session,
            receiver,
            resume,
            instructions,
            scratch,
        } = parts;
        let fingerprint = prompt::instructions_fingerprint(&instructions);
        let due = logic::instructions_due(resume.as_ref(), &fingerprint);
        let head = self
            .context
            .as_ref()
            .map(|context| context.head.clone())
            .unwrap_or_default();
        let (brief_delivered, moved_from) = logic::resumed_brief(resume.as_ref(), &head);
        let brief_head = if brief_delivered { head } else { String::new() };
        let pump = cx.spawn(async move |this, cx| pump(this, cx, receiver, generation).await);
        self.live = Some(Live {
            session,
            cwd,
            scratch,
            brief_delivered,
            brief_head,
            moved_from,
            instructions_due: due.then_some(instructions),
            instructions: fingerprint,
            claim: self.bound.as_ref().map(|bound| bound.claim.clone()),
            _pump: pump,
        });
    }

    fn send_now(
        &mut self,
        request: Request,
        retried: bool,
        disk_matches: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let (Some(context), Some(live)) = (self.context.clone(), self.live.as_ref()) else {
            self.set_phase(Phase::Idle, cx);
            return;
        };
        let included = !live.brief_delivered;
        let needs_brief = included || !matches!(request, Request::Chat { .. });
        let due = live.instructions_due.clone();
        let moved_from = live.moved_from.clone();
        let text = match disk_matches {
            Some(disk) if needs_brief => {
                let mut brief = context.brief.clone();
                brief.disk_matches_head = disk;
                logic::compose(
                    &request,
                    &brief,
                    context.local,
                    &context.head,
                    included,
                    &self.seen,
                    moved_from.as_deref(),
                )
            }
            _ => logic::compose(
                &request,
                &context.brief,
                context.local,
                &context.head,
                included,
                &self.seen,
                moved_from.as_deref(),
            ),
        };
        let text = logic::with_updated_instructions(due.as_deref(), text);
        if let Some(live) = self.live.as_ref() {
            live.session.prompt(text);
        }
        self.turn = Some(Turn {
            request,
            retried,
            carried_brief: needs_brief,
            carried_instructions: due.is_some(),
            sent: brief::fingerprints(&context.brief.threads),
            head: context.head.clone(),
            assistant: None,
            tools: Vec::new(),
            activity: Activity::Working,
        });
        self.set_phase(Phase::Running, cx);
    }

    pub(super) fn apply_events(
        &mut self,
        generation: u64,
        events: Vec<Event>,
        cx: &mut Context<Self>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let mut alive = true;
        for event in events {
            if !self.apply_event(event, cx) {
                alive = false;
                break;
            }
        }
        cx.notify();
        alive
    }

    fn apply_event(&mut self, event: Event, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Session { id } => self.remember_session(id, cx),
            Event::Text(text) => self.on_text(text, cx),
            Event::Thinking(_) => {
                if let Some(turn) = self.turn.as_mut()
                    && turn.tools.is_empty()
                {
                    turn.activity = Activity::Thinking;
                }
            }
            Event::ToolStarted { id, title } => {
                if let Some(turn) = self.turn.as_mut() {
                    let title: SharedString = title.into();
                    turn.tools.push((id, title.clone()));
                    turn.activity = Activity::Tool(title);
                }
            }
            Event::ToolFinished { id, .. } => {
                if let Some(turn) = self.turn.as_mut() {
                    turn.tools.retain(|(tool, _)| *tool != id);
                    turn.activity = match turn.tools.last() {
                        Some((_, title)) => Activity::Tool(title.clone()),
                        None => Activity::Working,
                    };
                }
            }
            Event::Notice(text) => {
                if let Some(turn) = self.turn.as_mut() {
                    turn.activity = Activity::Notice(text.into());
                }
            }
            Event::TurnFinished(outcome) => return self.on_turn_finished(outcome, cx),
            Event::Exited(reason) => {
                self.on_exited(reason, cx);
                return false;
            }
        }
        true
    }

    fn remember_session(&mut self, id: String, cx: &mut Context<Self>) {
        if self
            .bound
            .as_ref()
            .is_some_and(|bound| bound.claim.id() == id)
        {
            return;
        }
        self.bound = None;
        let Some(claim) = Claim::acquire(&id, self.owner) else {
            log::warn!("the assistant's session is already held by another window");
            return;
        };
        let (brief_delivered, brief_head, instructions) = match &self.live {
            Some(live) => (
                live.brief_delivered,
                live.brief_head.clone(),
                live.instructions.clone(),
            ),
            None => (false, String::new(), String::new()),
        };
        if let Some(live) = self.live.as_mut() {
            live.claim = Some(claim.clone());
        }
        self.bound = Some(Bound {
            claim,
            brief_delivered,
            brief_head: brief_head.clone(),
            instructions: instructions.clone(),
        });
        self.resumable = false;
        let (Some(context), Some(db)) = (self.context.as_ref(), crate::db::db(cx)) else {
            return;
        };
        let session = AgentSession {
            id,
            brief_delivered,
            brief_head,
            instructions,
        };
        let write = db.set_agent_session(
            &context.repo,
            &context.key,
            &self.profile.id,
            session,
            now(),
        );
        cx.background_spawn(async move {
            if let Err(err) = write.await {
                log::warn!("saving the assistant's session: {err}");
            }
        })
        .detach();
    }

    fn save_session_state(&self, cx: &mut Context<Self>) {
        let (Some(bound), Some(context), Some(db)) = (
            self.bound.as_ref(),
            self.context.as_ref(),
            crate::db::db(cx),
        ) else {
            return;
        };
        let session = AgentSession {
            id: bound.claim.id().to_owned(),
            brief_delivered: bound.brief_delivered,
            brief_head: bound.brief_head.clone(),
            instructions: bound.instructions.clone(),
        };
        let write = db.update_agent_session(
            &context.repo,
            &context.key,
            &self.profile.id,
            session,
            now(),
        );
        cx.background_spawn(async move {
            if let Err(err) = write.await {
                log::warn!("saving the assistant's session: {err}");
            }
        })
        .detach();
    }

    fn clear_session_id(&mut self, cx: &mut Context<Self>) {
        let id = self.bound.take().map(|bound| bound.claim.id().to_owned());
        self.resumable = false;
        let (Some(id), Some(context), Some(db)) = (id, self.context.as_ref(), crate::db::db(cx))
        else {
            return;
        };
        let write =
            db.clear_agent_session(&context.repo, &context.key, &self.profile.id, Some(&id));
        cx.background_spawn(async move {
            if let Err(err) = write.await {
                log::warn!("clearing the assistant's session: {err}");
            }
        })
        .detach();
    }

    fn on_text(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(turn) = self.turn.as_ref() else {
            return;
        };
        let existing = turn.assistant.and_then(|id| self.index_of(id));
        let ix = match existing {
            Some(ix) => ix,
            None => {
                let (kind, head) = (turn.request.kind(), turn.head.clone());
                let ix = self.push_message(Role::Assistant, kind, head, String::new(), cx);
                let id = self.messages[ix].id;
                if let Some(turn) = self.turn.as_mut() {
                    turn.assistant = Some(id);
                }
                ix
            }
        };
        self.messages[ix].append(&text, cx);
        self.scroll
            .update(cx, |scroll, cx| scroll.remeasure_items(ix..ix + 1, cx));
    }

    fn on_turn_finished(&mut self, outcome: Outcome, cx: &mut Context<Self>) -> bool {
        let turn = self.turn.take();
        if let Some(turn) = &turn {
            self.finish_assistant(turn.assistant, cx);
            if logic::answered(&outcome) {
                self.seen = turn.sent.clone();
            }
            self.auto_attempted = logic::auto_attempted_after(
                self.auto_attempted.as_deref(),
                turn.request.kind(),
                &turn.head,
                &outcome,
            );
            self.record_delivery(
                turn.carried_brief,
                turn.carried_instructions,
                &turn.head,
                &outcome,
                cx,
            );
        }
        match outcome {
            Outcome::Completed => {
                self.set_phase(Phase::Ready, cx);
                self.after_turn(cx);
            }
            Outcome::Interrupted => {
                self.add_notice(tr().assistant_notice_stopped.to_string(), cx);
                self.set_phase(Phase::Ready, cx);
                self.after_turn(cx);
            }
            Outcome::Cancelled => {
                self.set_phase(Phase::Idle, cx);
                self.after_turn(cx);
            }
            Outcome::Failed(failure) => {
                log::warn!(
                    "the assistant's turn failed ({:?}): {}",
                    failure.kind,
                    failure.message
                );
                if failure.kind == ::agent::FailureKind::SessionLost {
                    return self.recover_lost_session(turn, failure.message, cx);
                }
                let problem = logic::classify(&failure);
                let text = logic::problem_text(&problem, self.profile.login_hint(), tr());
                self.set_phase(Phase::Failed(text.into()), cx);
                self.after_turn(cx);
            }
        }
        true
    }

    fn record_delivery(
        &mut self,
        carried_brief: bool,
        carried_instructions: bool,
        head: &str,
        outcome: &Outcome,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let answered = logic::answered(outcome);
        let brief_sent = carried_brief && answered;
        let instructions_sent = carried_instructions && answered;
        live.brief_delivered = logic::brief_delivered(live.brief_delivered, carried_brief, outcome);
        if brief_sent {
            live.brief_head = head.to_owned();
            live.moved_from = None;
        }
        if instructions_sent {
            live.instructions_due = None;
        }
        if !(brief_sent || instructions_sent) {
            return;
        }
        let (delivered, brief_head, fingerprint) = (
            live.brief_delivered,
            live.brief_head.clone(),
            live.instructions.clone(),
        );
        if let Some(bound) = self.bound.as_mut() {
            bound.brief_delivered = delivered;
            bound.brief_head = brief_head;
            if instructions_sent {
                bound.instructions = fingerprint;
            }
            self.save_session_state(cx);
        }
    }

    fn recover_lost_session(
        &mut self,
        turn: Option<Turn>,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        self.clear_session_id(cx);
        self.retire_live();
        if turn.is_some() {
            self.add_notice(tr().assistant_notice_lost.to_string(), cx);
        }
        match turn {
            Some(turn) if !turn.retried => self.dispatch(turn.request, true, cx),
            _ => {
                let problem = logic::Problem::Other(message);
                let text = logic::problem_text(&problem, self.profile.login_hint(), tr());
                self.set_phase(Phase::Failed(text.into()), cx);
            }
        }
        false
    }

    fn on_exited(&mut self, reason: Option<String>, cx: &mut Context<Self>) {
        self.retire_live();
        let turn = self.turn.take();
        if let Some(turn) = turn {
            self.finish_assistant(turn.assistant, cx);
            log::warn!("the assistant stopped during a turn: {reason:?}");
            let text = logic::exit_text(reason.as_deref(), tr());
            self.set_phase(Phase::Failed(text.into()), cx);
            self.after_turn(cx);
        } else if let Some(reason) = reason {
            log::warn!("the assistant stopped: {reason}");
        }
    }

    fn after_turn(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.stale) {
            self.evaluate(cx);
        } else {
            self.refresh_overview();
        }
    }

    fn finish_assistant(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        let Some(ix) = id.and_then(|id| self.index_of(id)) else {
            return;
        };
        if self.messages[ix].text.trim().is_empty() {
            self.remove_message(ix, cx);
        }
    }

    pub(super) fn add_notice(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(head) = self.context.as_ref().map(|context| context.head.clone()) else {
            return;
        };
        self.push_message(Role::Notice, Kind::Chat, head, text, cx);
    }

    fn push_message(
        &mut self,
        role: Role,
        kind: Kind,
        head: String,
        text: String,
        cx: &mut Context<Self>,
    ) -> usize {
        let id = self.next_message;
        self.next_message += 1;
        self.messages.push(Message::new(id, role, kind, head, text));
        let ix = self.messages.len() - 1;
        self.scroll
            .update(cx, |scroll, cx| scroll.splice(ix..ix, 1, cx));
        self.rows += 1;
        cx.notify();
        ix
    }

    fn remove_message(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.messages.remove(ix);
        self.scroll
            .update(cx, |scroll, cx| scroll.splice(ix..ix + 1, 0, cx));
        self.rows -= 1;
    }

    pub(super) fn index_of(&self, id: u64) -> Option<usize> {
        self.messages.iter().rposition(|message| message.id == id)
    }

    pub(super) fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let desired = self.messages.len() + usize::from(self.phase.is_busy());
        let rows = self.rows;
        if desired > rows {
            self.scroll
                .update(cx, |scroll, cx| scroll.append(desired - rows, cx));
        } else if desired < rows {
            self.scroll
                .update(cx, |scroll, cx| scroll.splice(desired..rows, 0, cx));
        }
        self.rows = desired;
    }

    pub(super) fn row_changed(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(ix) = self.index_of(id) {
            self.scroll
                .update(cx, |scroll, cx| scroll.remeasure_items(ix..ix + 1, cx));
        }
    }
}
