use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::lines::Lines;
use crate::process::Environment;
use crate::session::{BoxFuture, Channels, Command, Driver, Plan, Transport};
use crate::{Error, Failure, FailureKind, Launch};

pub mod normalize;
pub mod wire;

use normalize::Normalizer;

const TOOLS: &str = "Bash,Read,Grep,Glob";
const DISALLOWED_TOOLS: &str = "Agent,Task,Workflow,AskUserQuestion,ExitPlanMode";
const SETTINGS_FILE: &str = "settings.json";
const INSTRUCTIONS_FILE: &str = "instructions.md";
const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Unix,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) { Os::Windows } else { Os::Unix }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionArg {
    New(String),
    Resume(String),
}

pub type SessionLookup = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct ClaudeDriver {
    os: Os,
    lookup: SessionLookup,
}

impl ClaudeDriver {
    pub fn new() -> Self {
        Self::with_lookup(Arc::new(session_exists))
    }

    pub fn with_lookup(lookup: SessionLookup) -> Self {
        Self {
            os: Os::current(),
            lookup,
        }
    }

    pub fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }
}

impl Default for ClaudeDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for ClaudeDriver {
    fn plan(&self, launch: &Launch) -> Result<Plan, Error> {
        let session = match launch.resume.as_deref().filter(|id| !id.is_empty()) {
            Some(id) if (self.lookup)(id) => SessionArg::Resume(id.to_owned()),
            Some(id) => {
                return Ok(Plan {
                    refusal: Some(Failure {
                        kind: FailureKind::SessionLost,
                        message: format!("No conversation found with session ID: {id}"),
                    }),
                    ..Plan::default()
                });
            }
            None => SessionArg::New(uuid::Uuid::new_v4().to_string()),
        };
        if launch.scratch.as_os_str().is_empty() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the Claude Code driver needs a scratch folder",
            )));
        }
        fs::write(
            launch.scratch.join(SETTINGS_FILE),
            settings_json(&launch.cwd, self.os),
        )?;
        fs::write(launch.scratch.join(INSTRUCTIONS_FILE), &launch.instructions)?;
        Ok(Plan {
            args: arguments(launch, &session),
            env: environment(),
            refusal: None,
        })
    }

    fn run(&self, _launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
        Box::pin(converse(transport, channels))
    }
}

pub fn arguments(launch: &Launch, session: &SessionArg) -> Vec<String> {
    let path = |name: &str| launch.scratch.join(name).to_string_lossy().into_owned();
    let mut args: Vec<String> = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        TOOLS,
        "--allowedTools",
        TOOLS,
        "--disallowedTools",
        DISALLOWED_TOOLS,
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "user",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--settings",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(path(SETTINGS_FILE));
    args.push("--append-system-prompt-file".to_owned());
    args.push(path(INSTRUCTIONS_FILE));
    if let Some(model) = launch.model.as_deref().filter(|model| !model.is_empty()) {
        args.push("--model".to_owned());
        args.push(model.to_owned());
    }
    match session {
        SessionArg::New(id) => {
            args.push("--session-id".to_owned());
            args.push(id.clone());
        }
        SessionArg::Resume(id) => args.push(format!("--resume={id}")),
    }
    args
}

pub fn environment() -> Vec<(String, String)> {
    [
        "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
        "CLAUDE_CODE_DISABLE_AUTO_MEMORY",
        "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), "1".to_owned()))
    .collect()
}

pub fn settings_json(cwd: &Path, os: Os) -> String {
    let mut settings = json!({"disableAllHooks": true});
    if os == Os::Unix {
        let cwd = std::path::absolute(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        settings["sandbox"] = json!({
            "enabled": true,
            "filesystem": {"denyWrite": [cwd.to_string_lossy()]},
        });
    }
    settings.to_string()
}

pub fn config_dir(configured: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    match configured.filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.map(|home| home.join(".claude")),
    }
}

pub fn session_file_exists(config: &Path, id: &str) -> bool {
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return false;
    }
    let Ok(projects) = fs::read_dir(config.join("projects")) else {
        return false;
    };
    let file = format!("{id}.jsonl");
    projects
        .flatten()
        .any(|project| project.path().join(&file).is_file())
}

fn session_exists(id: &str) -> bool {
    let env = Environment::current();
    config_dir(env.get(CONFIG_DIR_VAR), dirs::home_dir().as_deref())
        .is_some_and(|config| session_file_exists(&config, id))
}

struct Turns {
    running: bool,
    queue: VecDeque<String>,
    interrupts: u32,
}

async fn converse(transport: Transport, channels: Channels) {
    let Transport { reader, mut writer } = transport;
    let mut lines = Lines::new(reader);
    let mut normalizer = Normalizer::default();
    let mut turns = Turns {
        running: false,
        queue: VecDeque::new(),
        interrupts: 0,
    };
    let events = &channels.events;
    loop {
        tokio::select! {
            command = channels.commands.recv_async() => match command {
                Ok(Command::Prompt(text)) => {
                    if turns.running {
                        turns.queue.push_back(text);
                    } else if !start_turn(&mut writer, &mut turns, &text).await {
                        break;
                    }
                }
                Ok(Command::Interrupt) => {
                    if turns.running {
                        turns.interrupts += 1;
                        let id = format!("nitpick-interrupt-{}", turns.interrupts);
                        if write_line(&mut writer, &wire::interrupt_request(&id)).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(Command::Shutdown) | Err(_) => return,
            },
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let output = normalizer.feed(&line);
                    for event in output.events {
                        let _ = events.send(event);
                    }
                    if let Some(reply) = output.reply
                        && write_line(&mut writer, &reply).await.is_err()
                    {
                        break;
                    }
                    if output.finished {
                        turns.running = false;
                        if let Some(next) = turns.queue.pop_front()
                            && !start_turn(&mut writer, &mut turns, &next).await
                        {
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    log::debug!("reading the assistant's output failed: {error}");
                    break;
                }
            },
        }
    }
    end_unfinished(&mut normalizer, &mut turns, &channels);
}

fn end_unfinished(normalizer: &mut Normalizer, turns: &mut Turns, channels: &Channels) {
    for command in channels.commands.try_iter() {
        match command {
            Command::Prompt(text) => turns.queue.push_back(text),
            Command::Shutdown => return,
            Command::Interrupt => {}
        }
    }
    let events = &channels.events;
    let unanswered = usize::from(turns.running) + turns.queue.len();
    for _ in 0..unanswered {
        let failure = Failure {
            kind: FailureKind::Other,
            message: "The assistant stopped before it finished the reply.".to_owned(),
        };
        for event in normalizer.abort(failure).events {
            let _ = events.send(event);
        }
    }
}

async fn start_turn<W: AsyncWrite + Unpin>(writer: &mut W, turns: &mut Turns, text: &str) -> bool {
    turns.running = true;
    write_line(writer, &wire::user_message(text)).await.is_ok()
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}
