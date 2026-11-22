use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use crate::jsonrpc::{self, CallError, Client, Connection, Incoming, Options, RpcError};
use crate::session::{BoxFuture, Channels, Command, Driver, Plan, Transport};
use crate::{Error, Event, Failure, FailureKind, Launch, Outcome};

pub mod classify;
pub mod normalize;
pub mod wire;

use classify::{ResumeError, Version};
use normalize::{Normalizer, Output};
use wire::ErrorInfo;

const STOPPED: &str = "The assistant stopped before it finished the reply.";
const NOT_READY: &str = "Codex stopped before the session was ready.";

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub initialize: Duration,
    pub thread: Duration,
    pub turn_start: Duration,
    pub interrupt: Duration,
    pub stop: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(30),
            thread: Duration::from_secs(60),
            turn_start: Duration::from_secs(30),
            interrupt: Duration::from_secs(10),
            stop: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Default)]
pub struct CodexDriver {
    limits: Limits,
}

impl CodexDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

impl Driver for CodexDriver {
    fn plan(&self, _launch: &Launch) -> Result<Plan, Error> {
        Ok(Plan {
            args: arguments(),
            env: Vec::new(),
            refusal: None,
        })
    }

    fn run(&self, launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
        Box::pin(converse(launch, self.limits, transport, channels))
    }
}

pub fn arguments() -> Vec<String> {
    vec!["app-server".to_owned()]
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
}

#[derive(Default)]
struct Interrupt {
    wanted: bool,
    sent: bool,
    deadline: Option<Instant>,
}

struct Conversation {
    launch: Launch,
    limits: Limits,
    client: Client,
    connection: Connection,
    commands: flume::Receiver<Command>,
    events: flume::Sender<Event>,
    thread: String,
    queue: VecDeque<String>,
    normalizer: Normalizer,
    interrupt: Interrupt,
}

async fn converse(launch: Launch, limits: Limits, transport: Transport, channels: Channels) {
    let Transport { reader, writer } = transport;
    let connection = jsonrpc::connect(reader, writer, Options { tag_version: false });
    let mut conversation = Conversation {
        launch,
        limits,
        client: connection.client(),
        connection,
        commands: channels.commands,
        events: channels.events,
        thread: String::new(),
        queue: VecDeque::new(),
        normalizer: Normalizer::default(),
        interrupt: Interrupt::default(),
    };
    let (stop, ready) = match conversation.handshake().await {
        Ok(()) => (conversation.serve().await, true),
        Err(stop) => (stop, false),
    };
    conversation.conclude(stop, ready);
    conversation.connection.close().await;
}

impl Conversation {
    async fn handshake(&mut self) -> Result<(), Stop> {
        let limit = self.limits.initialize;
        let init = self
            .call("initialize", wire::initialize_params(), limit)
            .await
            .map_err(|fault| setup_stop("initialize", fault, limit))?;
        let version = wire::user_agent(&init).and_then(classify::version_from_user_agent);
        require_version(version)?;
        self.client
            .notify("initialized", json!({}))
            .map_err(|_| Stop::Failed(other(NOT_READY)))?;
        self.open_thread(version.is_some()).await
    }

    async fn open_thread(&mut self, version_known: bool) -> Result<(), Stop> {
        let limit = self.limits.thread;
        let requested = self.launch.resume.clone().filter(|id| !id.is_empty());
        let result = match &requested {
            None => {
                let params = wire::thread_start_params(&self.launch);
                self.call("thread/start", params, limit)
                    .await
                    .map_err(|fault| setup_stop("thread/start", fault, limit))?
            }
            Some(id) => self.resume(id).await?,
        };
        let id = wire::thread_id(&result)
            .map(str::to_owned)
            .or(requested)
            .ok_or_else(|| Stop::Failed(other("Codex did not return a thread id.")))?;
        if !version_known {
            require_version(wire::cli_version(&result).and_then(classify::parse_version))?;
        }
        self.normalizer.set_thread(&id);
        self.thread = id.clone();
        let _ = self.events.send(Event::Session { id });
        Ok(())
    }

    async fn resume(&mut self, id: &str) -> Result<Value, Stop> {
        let limit = self.limits.thread;
        let mut unarchived = false;
        loop {
            let params = wire::thread_resume_params(&self.launch, id);
            match self.call("thread/resume", params, limit).await {
                Ok(result) => return Ok(result),
                Err(Fault::Rpc(error)) => match classify::resume_error(&error.message) {
                    ResumeError::Archived if !unarchived => {
                        unarchived = true;
                        self.call("thread/unarchive", wire::unarchive_params(id), limit)
                            .await
                            .map_err(|fault| setup_stop("thread/unarchive", fault, limit))?;
                    }
                    ResumeError::Lost => {
                        return Err(Stop::Failed(Failure {
                            kind: FailureKind::SessionLost,
                            message: error.message,
                        }));
                    }
                    ResumeError::Archived | ResumeError::Other => {
                        return Err(Stop::Failed(rpc_failure(&error)));
                    }
                },
                Err(fault) => return Err(setup_stop("thread/resume", fault, limit)),
            }
        }
    }

    async fn serve(&mut self) -> Stop {
        loop {
            if let Err(stop) = self.advance().await {
                return stop;
            }
            let deadline = self.interrupt.deadline;
            let stop_timer = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                command = self.commands.recv_async() => {
                    if self.on_command(command).is_break() {
                        return Stop::Shutdown;
                    }
                }
                incoming = self.connection.next() => match incoming {
                    Some(incoming) => self.on_incoming(incoming),
                    None => return Stop::Ended,
                },
                () = stop_timer => return Stop::Stuck,
            }
        }
    }

    async fn advance(&mut self) -> Result<(), Stop> {
        loop {
            if !self.normalizer.active() {
                self.interrupt = Interrupt::default();
                let Some(prompt) = self.queue.pop_front() else {
                    return Ok(());
                };
                self.start_turn(&prompt).await?;
                continue;
            }
            if self.interrupt.wanted
                && !self.interrupt.sent
                && let Some(turn) = self.normalizer.turn_id().map(str::to_owned)
            {
                self.interrupt.sent = true;
                self.send_interrupt(&turn).await?;
                continue;
            }
            return Ok(());
        }
    }

    async fn start_turn(&mut self, prompt: &str) -> Result<(), Stop> {
        self.normalizer.begin_turn();
        let params = wire::turn_start_params(&self.thread, prompt);
        let limit = self.limits.turn_start;
        match self.call("turn/start", params, limit).await {
            Ok(result) => {
                if self.normalizer.active()
                    && let Some(id) = wire::turn_id(&result)
                {
                    self.normalizer.set_turn(id);
                }
                Ok(())
            }
            Err(Fault::Rpc(error)) => {
                if self.normalizer.active() {
                    let info = ErrorInfo::from_rpc(&error.message, error.data.as_ref());
                    let output = self.normalizer.fail_with(&info);
                    self.emit(output);
                }
                Ok(())
            }
            Err(Fault::Shutdown) => Err(Stop::Shutdown),
            Err(Fault::Closed) => Err(Stop::Ended),
            Err(Fault::TimedOut) => Err(Stop::Failed(other(
                "Codex did not accept the prompt in time. The session was closed.",
            ))),
        }
    }

    async fn send_interrupt(&mut self, turn: &str) -> Result<(), Stop> {
        let params = wire::interrupt_params(&self.thread, turn);
        let limit = self.limits.interrupt;
        match self.call("turn/interrupt", params, limit).await {
            Ok(_) => Ok(()),
            Err(Fault::Rpc(error)) => {
                log::warn!("Codex refused to interrupt the turn: {error}");
                Ok(())
            }
            Err(Fault::Shutdown) => Err(Stop::Shutdown),
            Err(Fault::Closed) => Err(Stop::Ended),
            Err(Fault::TimedOut) => Err(Stop::Stuck),
        }
    }

    async fn call(&mut self, method: &str, params: Value, limit: Duration) -> Result<Value, Fault> {
        let client = self.client.clone();
        let request = client.request(method, params);
        tokio::pin!(request);
        let timer = tokio::time::sleep(limit);
        tokio::pin!(timer);
        loop {
            tokio::select! {
                biased;
                result = &mut request => {
                    return match result {
                        Ok(value) => Ok(value),
                        Err(CallError::Rpc(error)) => Err(Fault::Rpc(error)),
                        Err(CallError::Closed) => Err(Fault::Closed),
                    };
                }
                () = &mut timer => return Err(Fault::TimedOut),
                command = self.commands.recv_async() => {
                    if self.on_command(command).is_break() {
                        return Err(Fault::Shutdown);
                    }
                }
                incoming = self.connection.next() => match incoming {
                    Some(incoming) => self.on_incoming(incoming),
                    None => return Err(Fault::Closed),
                },
            }
        }
    }

    fn on_command(&mut self, command: Result<Command, flume::RecvError>) -> ControlFlow<()> {
        match command {
            Ok(Command::Prompt(text)) => self.queue.push_back(text),
            Ok(Command::Interrupt) => {
                if self.normalizer.active() {
                    self.interrupt.wanted = true;
                    self.interrupt
                        .deadline
                        .get_or_insert_with(|| Instant::now() + self.limits.stop);
                }
            }
            Ok(Command::Shutdown) | Err(_) => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }

    fn on_incoming(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } => {
                let output = self.normalizer.feed(&method, &params);
                self.emit(output);
            }
            Incoming::Request { id, method, .. } => {
                let sent = match wire::server_reply(&method) {
                    Some(reply) => {
                        log::debug!("declined the Codex request {method}");
                        self.client.respond(&id, reply)
                    }
                    None => {
                        log::debug!("rejected the unknown Codex request {method}");
                        self.client.respond_error(
                            &id,
                            -32601,
                            &format!("Nitpick does not handle {method}."),
                        )
                    }
                };
                if let Err(error) = sent {
                    log::debug!("answering the Codex request {method} failed: {error}");
                }
            }
        }
    }

    fn emit(&self, output: Output) {
        for event in output.events {
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
            Stop::Stuck => {
                let _ = self.events.send(Event::Notice(
                    "Codex did not stop in time. The session was closed.".to_owned(),
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
        if self.normalizer.active() {
            let output = self.normalizer.abort(running);
            self.emit(output);
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

fn rpc_failure(error: &RpcError) -> Failure {
    let info = ErrorInfo::from_rpc(&error.message, error.data.as_ref());
    normalize::failure_from(Some(&info), None)
}

fn require_version(version: Option<Version>) -> Result<(), Stop> {
    classify::check_version(version).map_err(|message| Stop::Failed(other(&message)))
}

fn setup_stop(method: &str, fault: Fault, limit: Duration) -> Stop {
    match fault {
        Fault::Shutdown => Stop::Shutdown,
        Fault::Closed => Stop::Failed(other(NOT_READY)),
        Fault::TimedOut => Stop::Failed(other(&format!(
            "Codex did not answer {method} within {limit:?}."
        ))),
        Fault::Rpc(error) => Stop::Failed(rpc_failure(&error)),
    }
}
