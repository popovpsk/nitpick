use std::path::{Path, PathBuf};

use crate::GitError;
use crate::repo::Repo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
}

impl Repo {
    pub fn worktrees(&self) -> Result<Vec<Worktree>, GitError> {
        let out = self.git(&["worktree", "list", "--porcelain"])?;
        Ok(parse_worktrees(&String::from_utf8_lossy(&out)))
    }
}

pub fn parse_worktrees(listing: &str) -> Vec<Worktree> {
    let mut worktrees: Vec<Worktree> = Vec::new();
    for line in listing.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktrees.push(Worktree {
                path: PathBuf::from(path),
                branch: None,
            });
        } else if let Some(branch) = line.strip_prefix("branch ")
            && let Some(last) = worktrees.last_mut()
        {
            last.branch = Some(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_string(),
            );
        }
    }
    worktrees
}

pub fn worktree_for<'a>(
    worktrees: &'a [Worktree],
    branch: &str,
    remotes: &[String],
) -> Option<&'a Path> {
    let local = remotes
        .iter()
        .find_map(|remote| branch.strip_prefix(remote.as_str())?.strip_prefix('/'));
    let find = |name: &str| {
        worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(name))
            .map(|w| w.path.as_path())
    };
    find(branch).or_else(|| local.and_then(find))
}
