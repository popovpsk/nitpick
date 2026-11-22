use std::collections::VecDeque;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use crate::jsonrpc::{self, CallError, Client, Connection, Id, Incoming, Options, RpcError};
use crate::session::{BoxFuture, Channels, Command, Driver, Plan, Transport};
use crate::{AcpFlavor, Error, Event, Failure, FailureKind, HarnessKind, Launch, Outcome};

pub mod flavor;
pub mod normalize;
pub mod permission;
pub mod wire;

use flavor::{Dialect, ResumeVia};
use normalize::{Normalizer, outcome_from_stop};
use wire::Capabilities;

const STOPPED: &str = "The assistant stopped before it finished the reply.";
const NOT_READY: &str = "The assistant stopped before the session was ready.";
const SILENT: &str = "The assistant is not responding.";
const PERMISSION_METHOD: &str = "session/request_permission";
const UPDATE_METHOD: &str = "session/update";
const VERSION_PROBE: Duration = Duration::from_secs(5);

pub type VersionProbe = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub initialize: Duration,
    pub session: Duration,
    pub load: Duration,
    pub option: Duration,
    pub idle: Duration,
    pub cancel: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(30),
            session: Duration::from_secs(60),
            load: Duration::from_secs(90),
            option: Duration::from_secs(15),
            idle: Duration::from_secs(120),
            cancel: Duration::from_secs(12),
        }
    }
}

pub struct AcpDriver {
    flavor: AcpFlavor,
    limits: Limits,
    probe: VersionProbe,
}

impl AcpDriver {
    pub fn new(flavor: AcpFlavor) -> Self {
        Self {
            flavor,
            limits: Limits::default(),
            probe: Arc::new(probe_program_version),
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_probe(mut self, probe: VersionProbe) -> Self {
        self.probe = probe;
        self
    }
}

impl Driver for AcpDriver {
    fn plan(&self, launch: &Launch) -> Result<Plan, Error> {
        let dialect = if flavor::needs_version(self.flavor) {
            let program = crate::process::split_words(&launch.command)
                .into_iter()
                .next();
            let version = program.and_then(|program| (self.probe)(&program));
            Dialect::from_version(version.as_deref())
        } else {
            Dialect::V1
        };
        Ok(Plan {
            args: Vec::new(),
            env: flavor::environment(self.flavor, dialect),
            refusal: None,
        })
    }

    fn run(&self, launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
        Box::pin(converse(
            self.flavor,
            launch,
            self.limits,
            transport,
            channels,
        ))
    }
}

pub fn probe_program_version(program: &str) -> Option<String> {
    let program = program.to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    let task = net_runtime::handle().spawn(async move {
        let kind = HarnessKind::Acp(AcpFlavor::OpenCode);
        let probed =
            tokio::time::timeout(VERSION_PROBE, crate::process::probe_version(&program, kind))
                .await;
        let _ = sender.send(probed.ok().and_then(Result::ok));
    });
    let version = receiver
        .recv_timeout(VERSION_PROBE + Duration::from_secs(1))
        .ok()
        .flatten();
    task.abort();
    version
}

enum Fault {
    Shutdown,
    Closed,
    TimedOut,
    Rpc(RpcError),
}

enum Stop {
    Shutdown,
    Ended,
    Failed(Failure),
    Stuck,
    Silent,
}

type Call = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

struct Turn {
    call: Call,
    activity: Instant,
    cancel_deadline: Option<Instant>,
}

enum Expiry {
    Idle,
    Cancel,
}

struct Conversation {
    flavor: AcpFlavor,
    launch: Launch,
    limits: Limits,
    client: Client,
    connection: Connection,
    commands: flume::Receiver<Command>,
    events: flume::Sender<Event>,
    session: String,
    capabilities: Capabilities,
    instructions_due: bool,
    queue: VecDeque<String>,
    normalizer: Normalizer,
    turn: Option<Turn>,
}

async fn converse(
    flavor: AcpFlavor,
    launch: Launch,
    limits: Limits,
    transport: Transport,
    channels: Channels,
) {
    let Transport { reader, writer } = transport;
    let connection = jsonrpc::connect(reader, writer, Options::default());
    let mut conversation = Conversation {
        flavor,
        launch,
        limits,
        client: connection.client(),
        connection,
        commands: channels.commands,
        events: channels.events,
        session: String::new(),
        capabilities: Capabilities::default(),
        instructions_due: false,
        queue: VecDeque::new(),
        normalizer: Normalizer::default(),
        turn: None,
    };
    let (stop, ready) = match conversation.handshake().await {
        Ok(()) => (conversation.serve().await, true),
        Err(stop) => (stop, false),
    };
    conversation.conclude(stop, ready);
    conversation.connection.close().await;
}

async fn turn_response(turn: &mut Option<Turn>) -> Result<Value, CallError> {
    match turn {
        Some(turn) => (&mut turn.call).await,
        None => std::future::pending().await,
    }
}

impl Conversation {
    async fn handshake(&mut self) -> Result<(), Stop> {
        let limit = self.limits.initialize;
        let init = self
            .call("initialize", wire::initialize_params(), limit)
            .await
            .map_err(|fault| self.setup_stop("initialize", fault, limit, false))?;
        self.capabilities = Capabilities::from_initialize(&init);
        if init.get("protocolVersion").and_then(Value::as_i64) != Some(wire::PROTOCOL_VERSION) {
            log::warn!("the agent answered with another protocol version: {init}");
        }
        self.open_session().await
    }

    async fn open_session(&mut self) -> Result<(), Stop> {
        let requested = self.launch.resume.clone().filter(|id| !id.is_empty());
        let system_prompt = self.capabilities.system_prompt;
        let (id, result) = match requested {
            None => {
                let limit = self.limits.session;
                let params = wire::new_session_params(&self.launch, system_prompt);
                let result = self
                    .call("session/new", params, limit)
                    .await
                    .map_err(|fault| self.setup_stop("session/new", fault, limit, false))?;
                let id = wire::session_id(&result)
                    .map(str::to_owned)
                    .ok_or_else(|| Stop::Failed(other("The agent did not return a session id.")))?;
                self.instructions_due = !system_prompt;
                (id, result)
            }
            Some(id) => {
                let Some(via) = flavor::resume_via(self.flavor, &self.capabilities) else {
                    return Err(Stop::Failed(Failure {
                        kind: FailureKind::SessionLost,
                        message: "The agent cannot resume a session.".to_owned(),
                    }));
                };
                let limit = match via {
                    ResumeVia::Resume => self.limits.session,
                    ResumeVia::Load => self.limits.load,
                };
                let params = wire::resume_params(&self.launch, &id, system_prompt);
                let result = self
                    .call(via.method(), params, limit)
                    .await
                    .map_err(|fault| self.setup_stop(via.method(), fault, limit, true))?;
                let id = wire::session_id(&result).map(str::to_owned).unwrap_or(id);
                (id, result)
            }
        };
        self.session = id.clone();
        self.normalizer.set_session(&id);
        self.configure(&result).await?;
        let _ = self.events.send(Event::Session { id });
        Ok(())
    }

    async fn configure(&mut self, result: &Value) -> Result<(), Stop> {
        let limit = self.limits.option;
        let wanted = self.launch.model.clone().filter(|model| !model.is_empty());
        if let (Some(model), Some(option)) = (wanted, wire::config_option(result, "model"))
            && !option.is_current(&model)
        {
            let params = wire::set_option_params(&self.session, &option.id, &Value::String(model));
            match self.call("session/set_config_option", params, limit).await {
                Ok(_) => {}
                Err(Fault::Rpc(error)) => {
                    return Err(Stop::Failed(Failure {
                        kind: FailureKind::ModelNotFound,
                        message: error.message,
                    }));
                }
                Err(fault) => {
                    return Err(self.setup_stop("session/set_config_option", fault, limit, false));
                }
            }
        }
        if flavor::wants_plan_mode(self.flavor)
            && let Some(option) = wire::config_option(result, "mode")
            && option.offers("plan")
            && !option.is_current("plan")
        {
            let params = wire::set_option_params(&self.session, &option.id, &Value::from("plan"));
            match self.call("session/set_config_option", params, limit).await {
                Ok(_) => {}
                Err(Fault::Rpc(error)) => {
                    log::warn!("the agent did not switch to plan mode: {error}");
                }
                Err(fault) => {
                    return Err(self.setup_stop("session/set_config_option", fault, limit, false));
                }
            }
        }
        Ok(())
    }

    async fn serve(&mut self) -> Stop {
        loop {
            if self.turn.is_none()
                && let Some(prompt) = self.queue.pop_front()
            {
                self.start_turn(&prompt);
            }
            let deadline = self.deadline();
            let timer = async {
                match deadline {
                    Some((at, _)) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                biased;
                result = turn_response(&mut self.turn) => {
                    if let Some(stop) = self.on_response(result) {
                        return stop;
                    }
                }
                command = self.commands.recv_async() => {
                    if self.on_command(command).is_break() {
                        return Stop::Shutdown;
                    }
                }
                incoming = self.connection.next() => match incoming {
                    Some(incoming) => self.on_incoming(incoming),
                    None => return Stop::Ended,
                },
                () = timer => {
                    return match deadline.map(|(_, expiry)| expiry) {
                        Some(Expiry::Cancel) => Stop::Stuck,
                        _ => Stop::Silent,
                    };
                }
            }
        }
    }

    fn deadline(&self) -> Option<(Instant, Expiry)> {
        let turn = self.turn.as_ref()?;
        if let Some(at) = turn.cancel_deadline {
            return Some((at, Expiry::Cancel));
        }
        if self.normalizer.open_tools() > 0 {
            return None;
        }
        Some((turn.activity + self.limits.idle, Expiry::Idle))
    }

    fn start_turn(&mut self, prompt: &str) {
        self.normalizer.begin_turn();
        let instructions = if self.instructions_due && !self.launch.instructions.trim().is_empty() {
            Some(self.launch.instructions.as_str())
        } else {
            None
        };
        let params = wire::prompt_params(&self.session, instructions, prompt);
        let client = self.client.clone();
        self.turn = Some(Turn {
            call: Box::pin(async move { client.request("session/prompt", params).await }),
            activity: Instant::now(),
            cancel_deadline: None,
        });
    }

    fn on_response(&mut self, result: Result<Value, CallError>) -> Option<Stop> {
        self.drain_incoming();
        let outcome = match result {
            Ok(value) => {
                self.instructions_due = false;
                outcome_from_stop(wire::stop_reason(&value))
            }
            Err(CallError::Rpc(error)) => {
                Outcome::Failed(flavor::failure(self.flavor, &error, false))
            }
            Err(CallError::Closed) => return Some(Stop::Ended),
        };
        self.turn = None;
        let events = self.normalizer.finish(outcome);
        self.emit(events);
        None
    }

    fn drain_incoming(&mut self) {
        while let Some(incoming) = self.connection.try_next() {
            self.on_incoming(incoming);
        }
    }

    fn on_command(&mut self, command: Result<Command, flume::RecvError>) -> ControlFlow<()> {
        match command {
            Ok(Command::Prompt(text)) => self.queue.push_back(text),
            Ok(Command::Interrupt) => self.cancel_turn(),
            Ok(Command::Shutdown) | Err(_) => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }

    fn cancel_turn(&mut self) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        if turn.cancel_deadline.is_some() {
            return;
        }
        turn.cancel_deadline = Some(Instant::now() + self.limits.cancel);
        self.normalizer.mark_cancelled();
        if let Err(error) = self
            .client
            .notify("session/cancel", wire::cancel_params(&self.session))
        {
            log::debug!("sending session/cancel failed: {error}");
        }
    }

    fn on_incoming(&mut self, incoming: Incoming) {
        if let Some(turn) = self.turn.as_mut() {
            turn.activity = Instant::now();
        }
        match incoming {
            Incoming::Notification { method, params } => {
                if method == UPDATE_METHOD {
                    let events = self.normalizer.feed(&params);
                    self.emit(events);
                }
            }
            Incoming::Request { id, method, params } => self.on_request(&id, &method, &params),
        }
    }

    fn on_request(&mut self, id: &Id, method: &str, params: &Value) {
        let sent = if method == PERMISSION_METHOD {
            let recalled = permission::tool_call_id(params)
                .and_then(|call| self.normalizer.kind_of(call))
                .map(str::to_owned);
            let outcome = permission::answer(params, recalled.as_deref());
            log::debug!("answered a permission request: {outcome}");
            self.client.respond(id, outcome)
        } else {
            log::debug!("rejected the agent request {method}");
            self.client
                .respond_error(id, -32601, &format!("Nitpick does not handle {method}."))
        };
        if let Err(error) = sent {
            log::debug!("answering the agent request {method} failed: {error}");
        }
    }

    async fn call(&mut self, method: &str, params: Value, limit: Duration) -> Result<Value, Fault> {
        let client = self.client.clone();
        let request = client.request(method, params);
        tokio::pin!(request);
        let timer = tokio::time::sleep(limit);
        tokio::pin!(timer);
        let result = loop {
            tokio::select! {
                biased;
                result = &mut request => {
                    break match result {
                        Ok(value) => Ok(value),
                        Err(CallError::Rpc(error)) => Err(Fault::Rpc(error)),
                        Err(CallError::Closed) => Err(Fault::Closed),
                    };
                }
                () = &mut timer => return Err(Fault::TimedOut),
                command = self.commands.recv_async() => match command {
                    Ok(Command::Prompt(text)) => self.queue.push_back(text),
                    Ok(Command::Interrupt) => {}
                    Ok(Command::Shutdown) | Err(_) => return Err(Fault::Shutdown),
                },
                incoming = self.connection.next() => match incoming {
                    Some(incoming) => self.on_incoming(incoming),
                    None => return Err(Fault::Closed),
                },
            }
        };
        self.drain_incoming();
        result
    }

    fn setup_stop(&self, method: &str, fault: Fault, limit: Duration, resuming: bool) -> Stop {
        match fault {
            Fault::Shutdown => Stop::Shutdown,
            Fault::Closed => Stop::Failed(other(NOT_READY)),
            Fault::TimedOut => Stop::Failed(other(&format!(
                "The agent did not answer {method} within {limit:?}."
            ))),
            Fault::Rpc(error) => Stop::Failed(flavor::failure(self.flavor, &error, resuming)),
        }
    }

    fn emit(&self, events: Vec<Event>) {
        for event in events {
            let _ = self.events.send(event);
        }
    }

    fn conclude(&mut self, stop: Stop, ready: bool) {
        match stop {
            Stop::Shutdown => {}
            Stop::Ended => self.end(Outcome::Failed(other(STOPPED)), other(STOPPED), !ready),
            Stop::Failed(failure) => {
                self.end(Outcome::Failed(failure.clone()), failure, !ready);
            }
            Stop::Silent => self.end(Outcome::Failed(other(SILENT)), other(SILENT), false),
            Stop::Stuck => {
                let _ = self.events.send(Event::Notice(
                    "The assistant did not stop in time. The session was closed.".to_owned(),
                ));
                self.end(Outcome::Interrupted, other(STOPPED), false);
            }
        }
    }

    fn end(&mut self, running: Outcome, queued: Failure, at_least_one: bool) {
        for command in self.commands.try_iter() {
            match command {
                Command::Prompt(text) => self.queue.push_back(text),
                Command::Shutdown => return,
                Command::Interrupt => {}
            }
        }
        let mut answered = 0;
        if self.turn.take().is_some() {
            let events = self.normalizer.finish(running);
            self.emit(events);
            answered += 1;
        }
        while self.queue.pop_front().is_some() {
            let _ = self
                .events
                .send(Event::TurnFinished(Outcome::Failed(queued.clone())));
            answered += 1;
        }
        if answered == 0 && at_least_one {
            let _ = self
                .events
                .send(Event::TurnFinished(Outcome::Failed(queued)));
        }
    }
}

fn other(message: &str) -> Failure {
    Failure {
        kind: FailureKind::Other,
        message: message.to_owned(),
    }
}
