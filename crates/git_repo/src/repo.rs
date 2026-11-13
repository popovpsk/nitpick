use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::command::{self, NETWORK_TIMEOUT, checked_arg, records, text};
use crate::{Branch, BranchKind, Commit, Comparison, GitError, Remote};

const DEFAULT_BRANCH_NAMES: [&str; 4] = ["main", "master", "develop", "trunk"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
    network_timeout: Duration,
}

pub(crate) struct RefRecord {
    pub name: String,
    pub sha: String,
    pub upstream: String,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Repo, GitError> {
        let output = command::output(path, &["rev-parse", "--show-toplevel"])?;
        if !output.status.success() {
            return Err(GitError::NotARepository(path.to_path_buf()));
        }
        let toplevel = path_from_bytes(&output.stdout);
        if toplevel.as_os_str().is_empty() {
            return Err(GitError::NotARepository(path.to_path_buf()));
        }
        Ok(Repo {
            root: toplevel,
            network_timeout: NETWORK_TIMEOUT,
        })
    }

    pub fn with_network_timeout(mut self, timeout: Duration) -> Repo {
        self.network_timeout = timeout;
        self
    }

    pub fn network_timeout(&self) -> Duration {
        self.network_timeout
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub(crate) fn git<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Result<Vec<u8>, GitError> {
        command::run(&self.root, args)
    }

    pub fn current_branch(&self) -> Result<Option<String>, GitError> {
        let output = command::output(&self.root, &["symbolic-ref", "-q", "HEAD"])?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        let full = text(&command::check(output, &["symbolic-ref", "-q", "HEAD"])?);
        let Some(branch) = full.strip_prefix("refs/heads/") else {
            return Ok(None);
        };
        if !self.has_commit("HEAD") {
            return Ok(None);
        }
        Ok(Some(branch.to_string()))
    }

    pub fn branches(&self) -> Result<Vec<Branch>, GitError> {
        let out = self.git(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(upstream)%00%(HEAD)%00%(committerdate:unix)%00%(symref)%00%(contents:subject)%00",
            "refs/heads",
            "refs/remotes",
        ])?;
        let mut branches: Vec<Branch> = records(&out, 7)
            .into_iter()
            .filter_map(|record| {
                let [refname, sha, upstream, head, time, symref, subject] =
                    <[String; 7]>::try_from(record).ok()?;
                if !symref.is_empty() {
                    return None;
                }
                let (name, kind) = if let Some(name) = refname.strip_prefix("refs/heads/") {
                    (name, BranchKind::Local)
                } else {
                    let name = refname.strip_prefix("refs/remotes/")?;
                    if name.ends_with("/HEAD") {
                        return None;
                    }
                    (name, BranchKind::Remote)
                };
                Some(Branch {
                    name: name.to_string(),
                    kind,
                    sha,
                    upstream: (!upstream.is_empty()).then(|| short_ref(&upstream).to_string()),
                    is_head: kind == BranchKind::Local && head == "*",
                    committer_time: time.trim().parse().unwrap_or(0),
                    subject,
                })
            })
            .collect();
        branches.sort_by(|a, b| {
            (a.kind == BranchKind::Remote)
                .cmp(&(b.kind == BranchKind::Remote))
                .then(b.committer_time.cmp(&a.committer_time))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(branches)
    }

    pub fn default_target(&self, branches: &[Branch]) -> Option<String> {
        let has = |name: &str, kind: BranchKind| {
            branches
                .iter()
                .any(|branch| branch.kind == kind && branch.name == name)
        };
        let remote = if branches
            .iter()
            .any(|branch| branch.kind == BranchKind::Remote && branch.name.starts_with("origin/"))
        {
            "origin".to_string()
        } else {
            self.upstream_remote()
                .unwrap_or_else(|| "origin".to_string())
        };
        let remote_head = format!("refs/remotes/{remote}/HEAD");
        if let Ok(out) = self.git(&["symbolic-ref", "-q", remote_head.as_str()])
            && let Some(short) = text(&out).strip_prefix("refs/remotes/")
        {
            if let Some(name) = short.strip_prefix(&format!("{remote}/"))
                && has(name, BranchKind::Local)
            {
                return Some(name.to_string());
            }
            if has(short, BranchKind::Remote) {
                return Some(short.to_string());
            }
        }
        DEFAULT_BRANCH_NAMES.iter().find_map(|name| {
            if has(name, BranchKind::Local) {
                return Some(name.to_string());
            }
            let remote_name = format!("{remote}/{name}");
            has(&remote_name, BranchKind::Remote).then_some(remote_name)
        })
    }

    pub fn resolve(&self, rev: &str) -> Result<String, GitError> {
        let spec = format!("{}^{{commit}}", checked_arg(rev)?);
        let sha = text(&self.git(&["rev-parse", "--verify", "-q", spec.as_str()])?);
        if sha.is_empty() {
            return Err(GitError::Parse(format!("{rev} does not name a commit")));
        }
        Ok(sha)
    }

    pub fn has_commit(&self, sha: &str) -> bool {
        let Ok(sha) = checked_arg(sha) else {
            return false;
        };
        let spec = format!("{sha}^{{commit}}");
        command::output(&self.root, &["cat-file", "-e", spec.as_str()])
            .is_ok_and(|output| output.status.success())
    }

    pub(crate) fn ref_records(&self) -> Result<Vec<RefRecord>, GitError> {
        let out = self.git(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(upstream)%00",
            "refs/heads",
            "refs/remotes",
        ])?;
        Ok(records(&out, 3)
            .into_iter()
            .filter_map(|record| {
                let [name, sha, upstream] = <[String; 3]>::try_from(record).ok()?;
                Some(RefRecord {
                    name,
                    sha,
                    upstream,
                })
            })
            .collect())
    }

    pub fn compare(&self, target: &str, source: &str) -> Result<Comparison, GitError> {
        let target_sha = self.resolve(target)?;
        let head_sha = self.resolve(source)?;
        let refs = self.ref_records()?;
        let find = |name: &str| refs.iter().find(|record| record.name == name);
        let full_name = |short: &str| {
            find(&format!("refs/heads/{short}"))
                .or_else(|| find(&format!("refs/remotes/{short}")))
                .map(|record| record.name.clone())
        };
        let source_ref = full_name(source);
        let extras: Vec<&RefRecord> = if let Some(local) = find(&format!("refs/heads/{target}")) {
            find(&local.upstream).into_iter().collect()
        } else if let Some(remote) = find(&format!("refs/remotes/{target}")) {
            refs.iter()
                .filter(|record| {
                    record.name.starts_with("refs/heads/") && record.upstream == remote.name
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut merge_base_refs = vec![target.to_string()];
        let mut args = vec![
            "merge-base".to_string(),
            head_sha.clone(),
            target_sha.clone(),
        ];
        for extra in extras {
            if source_ref.as_deref() == Some(extra.name.as_str()) {
                continue;
            }
            merge_base_refs.push(short_ref(&extra.name).to_string());
            args.push(extra.sha.clone());
        }
        let output = command::output(&self.root, &args)?;
        let base_sha = text(&output.stdout);
        if output.status.code() == Some(1) && base_sha.is_empty() {
            return Err(GitError::NoMergeBase {
                target: target.to_string(),
                source_ref: source.to_string(),
            });
        }
        let base_sha = text(&command::check(output, &args)?);
        Ok(Comparison {
            base_sha,
            head_sha,
            target: target.to_string(),
            source: source.to_string(),
            target_sha,
            merge_base_refs,
        })
    }

    pub fn commits(&self, base: &str, head: &str) -> Result<Vec<Commit>, GitError> {
        let range = format!("{}..{}", checked_arg(base)?, checked_arg(head)?);
        let out = self.git(&[
            "log",
            "-z",
            "--no-color",
            "--no-show-signature",
            "--format=%H%x1f%an%x1f%at%x1f%s",
            range.as_str(),
            "--",
        ])?;
        let text = String::from_utf8_lossy(&out);
        text.split('\0')
            .map(|record| record.trim_start_matches('\n'))
            .filter(|record| !record.is_empty())
            .map(|record| {
                let mut fields = record.splitn(4, '\x1f');
                let mut next = || fields.next().unwrap_or_default().to_string();
                let sha = next();
                let author = next();
                let time = next();
                let subject = next();
                let time = time
                    .parse()
                    .map_err(|_| GitError::Parse(format!("bad commit time in {record:?}")))?;
                Ok(Commit {
                    sha,
                    subject,
                    author,
                    time,
                })
            })
            .collect()
    }

    pub fn remotes(&self) -> Result<Vec<Remote>, GitError> {
        let out = self.git(&["remote", "-v"])?;
        let mut remotes: Vec<Remote> = Vec::new();
        for line in String::from_utf8_lossy(&out).lines() {
            let Some((name, rest)) = line.split_once('\t') else {
                continue;
            };
            let Some(end) = rest.rfind(" (fetch)") else {
                continue;
            };
            if remotes.iter().any(|remote| remote.name == name) {
                continue;
            }
            remotes.push(Remote {
                name: name.to_string(),
                url: rest[..end].to_string(),
            });
        }
        Ok(remotes)
    }

    pub fn upstream_remote(&self) -> Option<String> {
        let from_head = self
            .git(&[
                "for-each-ref",
                "--format=%(HEAD)%00%(upstream:remotename)%00",
                "refs/heads",
            ])
            .ok()
            .and_then(|out| {
                records(&out, 2).into_iter().find_map(|record| {
                    let remote = &record[1];
                    (record[0] == "*" && !remote.is_empty() && remote != ".")
                        .then(|| remote.clone())
                })
            });
        if from_head.is_some() {
            return from_head;
        }
        let remotes = self.remotes().ok()?;
        if remotes.iter().any(|remote| remote.name == "origin") {
            return Some("origin".to_string());
        }
        remotes.into_iter().next().map(|remote| remote.name)
    }
}

pub(crate) fn short_ref(full: &str) -> &str {
    full.strip_prefix("refs/heads/")
        .or_else(|| full.strip_prefix("refs/remotes/"))
        .unwrap_or(full)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = bytes.to_vec();
    while bytes
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        bytes.pop();
    }
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(text(bytes))
}
