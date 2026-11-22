use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::acp::AcpDriver;
use crate::claude::ClaudeDriver;
use crate::codex::CodexDriver;
use crate::process::{Environment, Exit, Grace, Invocation, Pipes, Process};
use crate::registry::Registry;
use crate::{Error, Event, Failure, HarnessKind, Launch, Outcome};

const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
const DRIVER_DRAIN: Duration = Duration::from_secs(1);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Prompt(String),
    Interrupt,
    Shutdown,
}

pub type BoxReader = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWriter = Box<dyn AsyncWrite + Send + Unpin>;
pub type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub struct Transport {
    pub reader: BoxReader,
    pub writer: BoxWriter,
}

impl Transport {
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }
}

pub struct Channels {
    pub commands: flume::Receiver<Command>,
    pub events: flume::Sender<Event>,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub refusal: Option<Failure>,
}

pub trait Driver: Send + Sync + 'static {
    fn plan(&self, launch: &Launch) -> Result<Plan, Error>;

    fn run(&self, launch: Launch, transport: Transport, channels: Channels) -> BoxFuture;
}

pub fn driver_for(kind: HarnessKind) -> Result<Arc<dyn Driver>, Error> {
    match kind {
        HarnessKind::Claude => Ok(Arc::new(ClaudeDriver::new())),
        HarnessKind::Codex => Ok(Arc::new(CodexDriver::new())),
        HarnessKind::Acp(flavor) => Ok(Arc::new(AcpDriver::new(flavor))),
        HarnessKind::Pi => Err(Error::Unsupported),
    }
}

pub struct Control {
    id: u64,
    shutting_down: AtomicBool,
    commands: flume::Sender<Command>,
    finished: watch::Receiver<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Control {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_finished(&self) -> bool {
        *self.finished.borrow()
    }

    pub fn request_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Shutdown);
    }

    pub fn shutdown(self: &Arc<Self>) -> impl Future<Output = ()> + Send + 'static + use<> {
        self.request_shutdown();
        let control = Arc::clone(self);
        let task = net_runtime::handle().spawn(async move { control.wait_finished().await });
        async move {
            if let Err(error) = task.await {
                log::warn!("waiting for an assistant session to end: {error}");
            }
        }
    }

    async fn wait_finished(&self) {
        let mut finished = self.finished.clone();
        let waited = tokio::time::timeout(SHUTDOWN_DEADLINE, finished.wait_for(|done| *done)).await;
        if waited.is_err() {
            log::warn!("an assistant session did not end in {SHUTDOWN_DEADLINE:?}; aborting it");
            let task = self
                .task
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(task) = task {
                task.abort();
            }
        }
    }
}

struct Handle {
    control: Arc<Control>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.control.request_shutdown();
    }
}

#[derive(Clone)]
pub struct Session {
    handle: Arc<Handle>,
}

impl Session {
    pub fn start(launch: Launch) -> Result<(Session, flume::Receiver<Event>), Error> {
        let driver = driver_for(launch.kind)?;
        start_in(Registry::global(), Environment::current(), driver, launch)
    }

    pub fn prompt(&self, text: String) {
        self.send(Command::Prompt(text));
    }

    pub fn interrupt(&self) {
        self.send(Command::Interrupt);
    }

    pub fn shutdown(&self) -> impl Future<Output = ()> + Send + 'static + use<> {
        self.handle.control.shutdown()
    }

    pub fn control(&self) -> &Arc<Control> {
        &self.handle.control
    }

    fn send(&self, command: Command) {
        if self.handle.control.commands.send(command).is_err() {
            log::debug!("the assistant session has already ended");
        }
    }
}

pub fn start_in(
    registry: Arc<Registry>,
    env: Environment,
    driver: Arc<dyn Driver>,
    launch: Launch,
) -> Result<(Session, flume::Receiver<Event>), Error> {
    if !launch.scratch.as_os_str().is_empty() {
        std::fs::create_dir_all(&launch.scratch)?;
    }
    let plan = driver.plan(&launch)?;
    if let Some(failure) = plan.refusal {
        return Ok(refused(failure));
    }
    let invocation =
        Invocation::resolve_in(&launch.command, plan.args, &plan.env, env, &launch.cwd)?;
    let (process, pipes) = Process::spawn(&invocation)?;

    let (events_sender, events_receiver) = flume::unbounded();
    let (commands_sender, commands_receiver) = flume::unbounded();
    let (finished_sender, finished_receiver) = watch::channel(false);
    let control = Arc::new(Control {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        shutting_down: AtomicBool::new(false),
        commands: commands_sender,
        finished: finished_receiver,
        task: Mutex::new(None),
    });
    registry.insert(Arc::clone(&control));

    let supervised = Supervised {
        driver,
        launch,
        process,
        pipes,
        channels: Channels {
            commands: commands_receiver,
            events: events_sender,
        },
        control: Arc::clone(&control),
        finish: Finish {
            registry,
            id: control.id,
            done: finished_sender,
        },
    };
    let task = net_runtime::handle().spawn(supervise(supervised));
    *control.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);

    let session = Session {
        handle: Arc::new(Handle { control }),
    };
    Ok((session, events_receiver))
}

fn refused(failure: Failure) -> (Session, flume::Receiver<Event>) {
    let (events_sender, events_receiver) = flume::unbounded();
    let (commands_sender, _) = flume::unbounded();
    let (_, finished_receiver) = watch::channel(true);
    let control = Arc::new(Control {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        shutting_down: AtomicBool::new(false),
        commands: commands_sender,
        finished: finished_receiver,
        task: Mutex::new(None),
    });
    let message = failure.message.clone();
    let _ = events_sender.send(Event::TurnFinished(Outcome::Failed(failure)));
    let _ = events_sender.send(Event::Exited(Some(message)));
    let session = Session {
        handle: Arc::new(Handle { control }),
    };
    (session, events_receiver)
}

struct Finish {
    registry: Arc<Registry>,
    id: u64,
    done: watch::Sender<bool>,
}

impl Drop for Finish {
    fn drop(&mut self) {
        self.registry.remove(self.id);
        self.done.send_replace(true);
    }
}

struct Supervised {
    driver: Arc<dyn Driver>,
    launch: Launch,
    process: Process,
    pipes: Pipes,
    channels: Channels,
    control: Arc<Control>,
    finish: Finish,
}

async fn supervise(supervised: Supervised) {
    let Supervised {
        driver,
        launch,
        mut process,
        pipes,
        channels,
        control,
        finish,
    } = supervised;
    let events = channels.events.clone();
    let transport = Transport::new(pipes.stdout, pipes.stdin);
    let mut run = driver.run(launch, transport, channels);
    let mut exit: Option<Exit> = None;
    tokio::select! {
        () = &mut run => {}
        early = process.wait() => {
            if tokio::time::timeout(DRIVER_DRAIN, &mut run).await.is_err() {
                log::debug!("the driver kept running after its process exited");
            }
            exit = Some(early);
        }
    }
    drop(run);
    let exit = match exit {
        Some(exit) => exit,
        None => process.shutdown(Grace::default()).await,
    };
    let reason = if control.shutting_down.load(Ordering::SeqCst) {
        None
    } else {
        Some(exit_message(&exit))
    };
    let _ = events.send(Event::Exited(reason));
    drop(finish);
}

pub fn exit_message(exit: &Exit) -> String {
    if !exit.stderr.is_empty() {
        return exit.stderr.clone();
    }
    match exit.status {
        Some(status) => format!("the process exited ({status})"),
        None => "the process ended".to_owned(),
    }
}
