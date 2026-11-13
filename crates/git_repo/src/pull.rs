use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Output;

use crate::command::{self, checked_arg, records, text};
use crate::repo::Repo;
use crate::{GitError, PullOutcome};

struct LocalBranch {
    sha: String,
    remote: String,
    remote_ref: String,
    upstream: String,
}

impl Repo {
    async fn git_async<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Vec<u8>, GitError> {
        command::run_async(self.root(), args, self.network_timeout()).await
    }

    async fn git_output_async<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Output, GitError> {
        command::output_async(self.root(), args, self.network_timeout()).await
    }

    pub async fn fetch(&self, remote: &str, refspecs: &[String]) -> Result<(), GitError> {
        let mut args = vec![
            "fetch".to_string(),
            "--quiet".to_string(),
            "--no-tags".to_string(),
            "--no-recurse-submodules".to_string(),
            checked_arg(remote)?.to_string(),
        ];
        for refspec in refspecs {
            args.push(checked_arg(refspec)?.to_string());
        }
        self.git_async(&args).await.map(|_| ())
    }

    pub async fn pull(&self, branch: &str) -> Result<PullOutcome, GitError> {
        let branch = checked_arg(branch)?;
        let local_ref = format!("refs/heads/{branch}");
        let remote_ref = format!("refs/remotes/{branch}");
        let out = self
            .git_async(&[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)%00",
                local_ref.as_str(),
                remote_ref.as_str(),
            ])
            .await?;
        let mut local = None;
        let mut remote_sha = None;
        for record in records(&out, 5) {
            let Ok([name, sha, remote, remote_branch, upstream]) = <[String; 5]>::try_from(record)
            else {
                continue;
            };
            if name == local_ref {
                local = Some(LocalBranch {
                    sha,
                    remote,
                    remote_ref: remote_branch,
                    upstream,
                });
            } else if name == remote_ref {
                remote_sha = Some(sha);
            }
        }
        match local {
            Some(local) => self.pull_local(branch, local).await,
            None => self.pull_remote_tracking(branch, remote_sha).await,
        }
    }

    async fn pull_remote_tracking(
        &self,
        branch: &str,
        before: Option<String>,
    ) -> Result<PullOutcome, GitError> {
        let names = text(&self.git_async(&["remote"]).await?);
        let remote = names
            .lines()
            .filter(|name| {
                branch
                    .strip_prefix(*name)
                    .is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
            })
            .max_by_key(|name| name.len())
            .ok_or_else(|| GitError::UnknownBranch(branch.to_string()))?;
        let rest = &branch[remote.len() + 1..];
        let tracking = format!("refs/remotes/{branch}");
        self.fetch(remote, &[format!("+refs/heads/{rest}:{tracking}")])
            .await?;
        let after = self.resolve_async(&tracking).await?;
        Ok(match before {
            Some(before) if before == after => PullOutcome::UpToDate,
            Some(before) => PullOutcome::Updated {
                from: before,
                to: after,
            },
            None => PullOutcome::Updated {
                from: "0".repeat(after.len()),
                to: after,
            },
        })
    }

    async fn pull_local(&self, branch: &str, local: LocalBranch) -> Result<PullOutcome, GitError> {
        if local.remote.is_empty() || local.upstream.is_empty() {
            return Err(GitError::NoUpstream(branch.to_string()));
        }
        if local.remote != "." {
            if !local.upstream.starts_with("refs/remotes/") || local.remote_ref.is_empty() {
                return Err(GitError::NoUpstream(branch.to_string()));
            }
            self.fetch(
                &local.remote,
                &[format!("+{}:{}", local.remote_ref, local.upstream)],
            )
            .await?;
        }
        let old = local.sha;
        let new = self.resolve_async(&local.upstream).await?;
        if old == new {
            return Ok(PullOutcome::UpToDate);
        }
        if !self.is_ancestor_async(&old, &new).await? {
            if self.is_ancestor_async(&new, &old).await? {
                return Ok(PullOutcome::UpToDate);
            }
            return Err(GitError::NotFastForward(branch.to_string()));
        }
        let local_ref = format!("refs/heads/{branch}");
        let head = self
            .git_output_async(&["symbolic-ref", "-q", "HEAD"])
            .await?;
        if head.status.success() && text(&head.stdout) == local_ref {
            self.git_async(&[
                "merge",
                "--ff-only",
                "--no-autostash",
                "--no-edit",
                "--quiet",
                new.as_str(),
            ])
            .await?;
        } else {
            if let Some(worktree) = self.worktree_with(&local_ref).await? {
                return Err(GitError::CheckedOutElsewhere {
                    branch: branch.to_string(),
                    worktree,
                });
            }
            let message = format!("pull: fast-forward {branch} to {}", local.upstream);
            self.git_async(&[
                "update-ref",
                "-m",
                message.as_str(),
                local_ref.as_str(),
                new.as_str(),
                old.as_str(),
            ])
            .await?;
        }
        Ok(PullOutcome::Updated { from: old, to: new })
    }

    async fn resolve_async(&self, refname: &str) -> Result<String, GitError> {
        let spec = format!("{refname}^{{commit}}");
        let sha = text(
            &self
                .git_async(&["rev-parse", "--verify", "-q", spec.as_str()])
                .await?,
        );
        if sha.is_empty() {
            return Err(GitError::Parse(format!("{refname} does not name a commit")));
        }
        Ok(sha)
    }

    async fn is_ancestor_async(&self, ancestor: &str, descendant: &str) -> Result<bool, GitError> {
        let args = ["merge-base", "--is-ancestor", ancestor, descendant];
        let output = self.git_output_async(&args).await?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(command::command_error(&output, &args)),
        }
    }

    async fn worktree_with(&self, local_ref: &str) -> Result<Option<PathBuf>, GitError> {
        let out = self.git_async(&["worktree", "list", "--porcelain"]).await?;
        let listing = String::from_utf8_lossy(&out);
        let mut current: Option<&str> = None;
        for line in listing.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current = Some(path);
            } else if line.strip_prefix("branch ") == Some(local_ref) {
                return Ok(Some(PathBuf::from(current.unwrap_or_default())));
            }
        }
        Ok(None)
    }
}
