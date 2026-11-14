use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::command::{self, checked_arg, command_error, text};
use crate::repo::Repo;
use crate::{BlameCommit, BlameLine, GitError};

const UNCOMMITTED_ID: &str = "0000000000000000000000000000000000000000";
const UNCOMMITTED_AUTHOR: &str = "Not Committed Yet";

impl Repo {
    pub fn blame(&self, rev: Option<&str>, path: &str) -> Result<Vec<BlameLine>, GitError> {
        if path.is_empty() {
            return Err(GitError::Parse("blame needs a path".to_string()));
        }
        let mut args = vec!["blame", "--porcelain", "--no-textconv"];
        if let Some(rev) = rev {
            args.push(checked_arg(rev)?);
        }
        args.push("--");
        args.push(path);
        let output = command::output(self.root(), &args)?;
        if output.status.success() {
            return parse_porcelain(&output.stdout);
        }
        let failure = match rev {
            Some(rev) if self.has_commit(rev) && !self.is_blob(rev, path) => return Ok(Vec::new()),
            Some(_) => command_error(&output, &args),
            None => match self.blame_untracked(path) {
                Some(result) => return result,
                None => command_error(&output, &args),
            },
        };
        args.insert(1, "--no-ignore-revs-file");
        match command::output(self.root(), &args) {
            Ok(retry) if retry.status.success() => {
                log::warn!("blame {path}: {failure}; retried without blame.ignoreRevsFile");
                parse_porcelain(&retry.stdout)
            }
            _ => Err(failure),
        }
    }

    fn is_blob(&self, rev: &str, path: &str) -> bool {
        let spec = format!("{rev}:{path}");
        command::output(self.root(), &["cat-file", "-t", spec.as_str()])
            .is_ok_and(|output| output.status.success() && text(&output.stdout) == "blob")
    }

    fn blame_untracked(&self, path: &str) -> Option<Result<Vec<BlameLine>, GitError>> {
        let full = self.root().join(path);
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some(Ok(Vec::new())),
            Err(err) => return Some(Err(err.into())),
        };
        if meta.is_dir() {
            return Some(Ok(Vec::new()));
        }
        if self.is_blob("HEAD", path) {
            return None;
        }
        let count = if meta.file_type().is_symlink() {
            1
        } else {
            match std::fs::read(&full) {
                Ok(bytes) => line_count(&bytes),
                Err(err) => return Some(Err(err.into())),
            }
        };
        let commit = Arc::new(BlameCommit {
            id: UNCOMMITTED_ID.to_string(),
            author: UNCOMMITTED_AUTHOR.to_string(),
            author_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64),
            summary: String::new(),
        });
        Some(Ok(vec![BlameLine { commit }; count]))
    }
}

fn line_count(bytes: &[u8]) -> usize {
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    newlines + usize::from(bytes.last().is_some_and(|byte| *byte != b'\n'))
}

struct Entry {
    id: String,
    line: usize,
}

pub(crate) fn parse_porcelain(out: &[u8]) -> Result<Vec<BlameLine>, GitError> {
    let mut commits: HashMap<String, Arc<BlameCommit>> = HashMap::new();
    let mut building: Option<BlameCommit> = None;
    let mut entry: Option<Entry> = None;
    let mut slots: Vec<Option<Arc<BlameCommit>>> = Vec::new();
    for raw in out.split(|byte| *byte == b'\n') {
        if raw.first() == Some(&b'\t') {
            let Entry { id, line } = entry.take().ok_or_else(|| bad_line(raw))?;
            let commit = match commits.get(&id) {
                Some(commit) => commit.clone(),
                None => {
                    let built = building.take().ok_or_else(|| {
                        GitError::Parse(format!("blame: no commit details for {id}"))
                    })?;
                    let built = Arc::new(built);
                    commits.insert(id, built.clone());
                    built
                }
            };
            if slots.len() < line {
                slots.resize(line, None);
            }
            slots[line - 1] = Some(commit);
            continue;
        }
        let Some(current) = &entry else {
            if raw.is_empty() {
                continue;
            }
            let header = parse_header(raw).ok_or_else(|| bad_line(raw))?;
            if !commits.contains_key(&header.id) {
                building = Some(BlameCommit {
                    id: header.id.clone(),
                    author: String::new(),
                    author_time: 0,
                    summary: String::new(),
                });
            }
            entry = Some(header);
            continue;
        };
        let Some(commit) = building.as_mut().filter(|commit| commit.id == current.id) else {
            continue;
        };
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches('\r');
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => commit.author = value.to_string(),
            "author-time" => commit.author_time = value.trim().parse().unwrap_or(0),
            "summary" => commit.summary = value.to_string(),
            _ => {}
        }
    }
    if entry.is_some() {
        return Err(GitError::Parse("truncated git blame output".to_string()));
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(ix, slot)| {
            slot.map(|commit| BlameLine { commit })
                .ok_or_else(|| GitError::Parse(format!("git blame skipped line {}", ix + 1)))
        })
        .collect()
}

fn parse_header(raw: &[u8]) -> Option<Entry> {
    let line = std::str::from_utf8(raw).ok()?;
    let mut fields = line.split(' ');
    let id = fields.next()?;
    let _original = fields.next()?;
    let line: usize = fields.next()?.parse().ok()?;
    let well_formed = matches!(id.len(), 40 | 64)
        && id.bytes().all(|byte| byte.is_ascii_hexdigit())
        && line > 0
        && fields
            .next()
            .is_none_or(|count| count.parse::<usize>().is_ok())
        && fields.next().is_none();
    well_formed.then(|| Entry {
        id: id.to_string(),
        line,
    })
}

fn bad_line(raw: &[u8]) -> GitError {
    GitError::Parse(format!(
        "unexpected git blame line {:?}",
        String::from_utf8_lossy(raw)
    ))
}
