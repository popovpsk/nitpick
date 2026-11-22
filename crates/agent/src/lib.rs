use std::path::PathBuf;

pub mod acp;
pub mod brief;
pub mod claude;
pub mod codex;
pub mod jsonrpc;
pub mod lines;
pub mod process;
pub mod prompt;
pub mod registry;
pub mod session;

#[cfg(test)]
mod tests;

pub use registry::shutdown_all;
pub use session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarnessKind {
    Claude,
    Codex,
    Acp(AcpFlavor),
    Pi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AcpFlavor {
    OpenCode,
    DeepSeek,
    Generic,
}

#[derive(Debug, Clone)]
pub struct Launch {
    pub kind: HarnessKind,
    pub command: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub resume: Option<String>,
    pub instructions: String,
    pub scratch: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Session { id: String },
    Text(String),
    Thinking(String),
    ToolStarted { id: String, title: String },
    ToolFinished { id: String, failed: bool },
    Notice(String),
    TurnFinished(Outcome),
    Exited(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    NotLoggedIn,
    UsageLimit,
    ContextTooLong,
    ModelNotFound,
    SessionLost,
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this assistant is not supported yet")]
    Unsupported,
    #[error("the launch command is empty")]
    EmptyCommand,
    #[error("`{0}` was not found")]
    NotFound(String),
    #[error("cannot start `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` failed: {detail}")]
    Probe { command: String, detail: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub async fn probe(command: &str, kind: HarnessKind) -> Result<String, Error> {
    let command = command.to_owned();
    process::on_runtime(async move { process::probe_version(&command, kind).await }).await
}
