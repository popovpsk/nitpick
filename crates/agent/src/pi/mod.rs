use std::collections::VecDeque;
use std::fs;
use std::io;
use std::ops::ControlFlow;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::time::Instant;

use crate::lines::Lines;
use crate::session::{BoxFuture, BoxReader, BoxWriter, Channels, Command, Driver, Plan, Transport};
use crate::{Error, Event, Failure, FailureKind, Launch, Outcome};

pub mod classify;
pub mod normalize;
pub mod wire;

use normalize::{Normalizer, STATE_ID};
use wire::State;

const INSTRUCTIONS_FILE: &str = "instructions.md";
const TOOLS: &str = "read,grep,find,ls,bash";
const STOPPED: &str = "The assistant stopped before it finished the reply.";
const NOT_READY: &str = "Pi stopped before the session was ready.";

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub state: Duration,
    pub stop: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            state: Duration::from_secs(30),
            stop: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Default)]
pub struct PiDriver {
    limits: Limits,
}

impl PiDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

impl Driver for PiDriver {
    fn plan(&self, launch: &Launch) -> Result<Plan, Error> {
        let session = match launch.resume.as_deref().filter(|id| !id.is_empty()) {
            Some(id) if valid_session_id(id) => id.to_owned(),
            Some(id) => {
                return Ok(Plan {
                    refusal: Some(Failure {
                        kind: FailureKind::SessionLost,
                        message: format!("Pi cannot open the session {id}: it is not a valid id."),
                    }),
                    ..Plan::default()
                });
            }
            None => format!("nitpick-{}", uuid::Uuid::new_v4()),
        };
        if launch.scratch.as_os_str().is_empty() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the Pi driver needs a scratch folder",
            )));
        }
        fs::write(launch.scratch.join(INSTRUCTIONS_FILE), &launch.instructions)?;
        Ok(Plan {
            args: arguments(launch, &session),
            env: Vec::new(),
            refusal: None,
        })
    }

    fn run(&self, launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
        Box::pin(converse(launch, self.limits, transport, channels))
    }
}

pub fn arguments(launch: &Launch, session: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--mode",
        "rpc",
        "--no-themes",
        "--no-approve",
        "--no-skills",
        "--no-prompt-templates",
        "--tools",
        TOOLS,
        "--session-id",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(session.to_owned());
    args.push("--append-system-prompt".to_owned());
    args.push(
        launch
            .scratch
            .join(INSTRUCTIONS_FILE)
            .to_string_lossy()
            .into_owned(),
    );
    if let Some(model) = launch.model.as_deref().filter(|model| !model.is_empty()) {
        args.push("--model".to_owned());
        args.push(model.to_owned());
    }
    args
}

pub fn valid_session_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
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
    requested: Option<String>,
    limits: Limits,
    lines: Lines<BoxReader>,
    writer: BoxWriter,
    commands: flume::Receiver<Command>,
    events: flume::Sender<Event>,
    queue: VecDeque<String>,
    normalizer: Normalizer,
    interrupt: Interrupt,
    prompts: u64,
    stops: u64,
}

async fn converse(launch: Launch, limits: Limits, transport: Transport, channels: Channels) {
    let Transport { reader, writer } = transport;
    let mut conversation = Conversation {
        requested: launch.resume.filter(|id| !id.is_empty()),
        limits,
        lines: Lines::new(reader),
        writer,
        commands: channels.commands,
        events: channels.events,
        queue: VecDeque::new(),
        normalizer: Normalizer::default(),
        interrupt: Interrupt::default(),
        prompts: 0,
        stops: 0,
    };
    let stop = match conversation.handshake().await {
        Ok(()) => conversation.serve().await,
        Err(stop) => stop,
    };
    conversation.conclude(stop);
}

impl Conversation {
    async fn handshake(&mut self) -> Result<(), Stop> {
        if self.write(&wire::get_state(STATE_ID)).await.is_err() {
            return Err(Stop::Failed(other(NOT_READY)));
        }
        let limit = self.limits.state;
        let deadline = Instant::now() + limit;
        loop {
            tokio::select! {
                command = self.commands.recv_async() => {
                    if self.on_command(command).is_break() {
                        return Err(Stop::Shutdown);
                    }
                }
                line = self.lines.next_line() => match line {
                    Ok(Some(line)) => {
                        let output = self.normalizer.feed(&line);
                        if let Some(reply) = output.reply
                            && self.write(&reply).await.is_err()
                        {
                            return Err(Stop::Failed(other(NOT_READY)));
                        }
                        if let Some(state) = output.state {
                            return self.admit(state);
                        }
                    }
                    Ok(None) => return Err(Stop::Failed(other(NOT_READY))),
                    Err(error) => {
                        log::debug!("reading the assistant's output failed: {error}");
                        return Err(Stop::Failed(other(NOT_READY)));
                    }
                },
                () = tokio::time::sleep_until(deadline) => {
                    return Err(Stop::Failed(other(&format!(
                        "Pi did not answer get_state within {limit:?}."
                    ))));
                }
            }
        }
    }

    fn admit(&mut self, state: Result<State, Failure>) -> Result<(), Stop> {
        let state = state.map_err(Stop::Failed)?;
        if let Some(requested) = &self.requested
            && state.message_count.unwrap_or(0) == 0
        {
            return Err(Stop::Failed(Failure {
                kind: FailureKind::SessionLost,
                message: format!("Pi has no history for the session {requested}."),
            }));
        }
        let id = state
            .session_id
            .filter(|id| !id.is_empty())
            .or_else(|| self.requested.clone());
        match id {
            Some(id) => {
                let _ = self.events.send(Event::Session { id });
            }
            None => log::warn!("Pi did not report the id of a new session"),
        }
        Ok(())
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
                line = self.lines.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Err(stop) = self.on_line(&line).await {
                            return stop;
                        }
                    }
                    Ok(None) => return Stop::Ended,
                    Err(error) => {
                        log::debug!("reading the assistant's output failed: {error}");
                        return Stop::Ended;
                    }
                },
                () = stop_timer => return Stop::Stuck,
            }
        }
    }

    async fn advance(&mut self) -> Result<(), Stop> {
        if !self.normalizer.active() {
            self.interrupt = Interrupt::default();
            let Some(prompt) = self.queue.pop_front() else {
                return Ok(());
            };
            self.prompts += 1;
            let id = format!("nitpick-prompt-{}", self.prompts);
            self.normalizer.begin_turn(&id);
            self.write(&wire::prompt(&id, &prompt))
                .await
                .map_err(ended)?;
        } else if self.interrupt.wanted && !self.interrupt.sent {
            self.interrupt.sent = true;
            self.stops += 1;
            let clear = wire::clear_queue(&format!("nitpick-clear-{}", self.stops));
            let abort = wire::abort(&format!("nitpick-abort-{}", self.stops));
            self.write(&clear).await.map_err(ended)?;
            self.write(&abort).await.map_err(ended)?;
        }
        Ok(())
    }

    async fn on_line(&mut self, line: &str) -> Result<(), Stop> {
        let output = self.normalizer.feed(line);
        for event in output.events {
            let _ = self.events.send(event);
        }
        if let Some(reply) = output.reply {
            self.write(&reply).await.map_err(ended)?;
        }
        Ok(())
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

    async fn write(&mut self, line: &str) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await
    }

    fn conclude(&mut self, stop: Stop) {
        match stop {
            Stop::Shutdown => {}
            Stop::Ended => self.end(Outcome::Failed(other(STOPPED)), other(STOPPED), false),
            Stop::Failed(failure) => self.end(Outcome::Failed(failure.clone()), failure, true),
            Stop::Stuck => {
                let _ = self.events.send(Event::Notice(
                    "Pi did not stop in time. The session was closed.".to_owned(),
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
            for event in self.normalizer.end(running).events {
                let _ = self.events.send(event);
            }
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

fn ended(error: io::Error) -> Stop {
    log::debug!("writing to the assistant failed: {error}");
    Stop::Ended
}

fn other(message: &str) -> Failure {
    Failure {
        kind: FailureKind::Other,
        message: message.to_owned(),
    }
}
