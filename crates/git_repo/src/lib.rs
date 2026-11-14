mod blame;
mod command;
mod diff;
mod pull;
mod remote_url;
mod repo;
mod scan;
mod worktree;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

pub use command::NETWORK_TIMEOUT;
pub use remote_url::{RemoteLocation, parse_remote_url};
pub use repo::Repo;
pub use scan::{SCAN_DEPTH, find_repositories};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{} is not inside a git work tree", .0.display())]
    NotARepository(PathBuf),
    #[error("`git {args}` failed ({}): {}", status_text(*.status), .stderr.trim())]
    Command {
        args: String,
        status: Option<i32>,
        stderr: String,
    },
    #[error("could not run git: {0}")]
    Io(#[from] std::io::Error),
    #[error("`git {args}` did not finish in time and was stopped")]
    TimedOut { args: String },
    #[error("{0}")]
    Parse(String),
    #[error("{0} has no upstream branch to pull from")]
    NoUpstream(String),
    #[error("{0} has diverged from its upstream; only fast-forward updates are possible")]
    NotFastForward(String),
    #[error("{target} and {source_ref} have no common history")]
    NoMergeBase { target: String, source_ref: String },
    #[error("{branch} is checked out in another work tree ({}); pull it there", .worktree.display())]
    CheckedOutElsewhere { branch: String, worktree: PathBuf },
    #[error("{0} is neither a local nor a remote-tracking branch")]
    UnknownBranch(String),
}

fn status_text(status: Option<i32>) -> String {
    match status {
        Some(code) => format!("exit code {code}"),
        None => "killed by a signal".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub kind: BranchKind,
    pub sha: String,
    pub upstream: Option<String>,
    pub is_head: bool,
    pub committer_time: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub base_sha: String,
    pub head_sha: String,
    pub target: String,
    pub source: String,
    pub target_sha: String,
    pub merge_base_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffOptions {
    pub context_lines: u32,
    pub find_renames: bool,
    pub ignore_whitespace: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        DiffOptions {
            context_lines: 3,
            find_renames: true,
            ignore_whitespace: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub old_blob: String,
    pub new_blob: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommit {
    pub id: String,
    pub author: String,
    pub author_time: i64,
    pub summary: String,
}

impl BlameCommit {
    pub fn is_uncommitted(&self) -> bool {
        !self.id.is_empty() && self.id.bytes().all(|byte| byte == b'0')
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub commit: Arc<BlameCommit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    UpToDate,
    Updated { from: String, to: String },
}
