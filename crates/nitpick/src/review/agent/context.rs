use std::path::{Path, PathBuf};

use ::agent::brief::{self, Brief};
use forge::ForgeKind;
use git_repo::Repo;
use review_core::{FileStatus, Side};
use ui_resources::i18n::Lang;

use crate::review::file_actions::checkout_of;
use crate::review::{LoadState, ReviewSource, ReviewView};

pub(in crate::review) struct ReviewContext {
    pub repo: String,
    pub key: String,
    pub head: String,
    pub local: bool,
    pub brief: Brief,
    pub workdir: WorkdirPlan,
}

#[derive(Clone)]
pub(in crate::review) struct WorkdirPlan {
    pub repo: Repo,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Workdir {
    pub cwd: PathBuf,
    pub disk_matches_head: bool,
}

pub(super) struct FileInput {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub viewed: bool,
    pub hidden: bool,
}

pub(super) struct DiscussionInput {
    pub id: String,
    pub resolved: bool,
    pub position: Option<brief::Position>,
    pub notes: Vec<(String, String)>,
}

pub(super) struct BriefInputs {
    pub kind: brief::Kind,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub description: String,
    pub commits: Vec<brief::Commit>,
    pub files: Vec<FileInput>,
    pub discussions: Vec<DiscussionInput>,
}

pub(super) fn review_key(source: &ReviewSource) -> String {
    match source {
        ReviewSource::MergeRequest {
            forge,
            project,
            number,
        } => {
            let forge = match forge {
                ForgeKind::GitLab => "gitlab",
                ForgeKind::GitHub => "github",
            };
            format!("mr:{forge}:{project}:{number}")
        }
        ReviewSource::Branches { target, source } => format!("branches:{target}..{source}"),
        ReviewSource::WorkingTree => "worktree".to_owned(),
    }
}

pub(super) fn local_head<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    for (path, fingerprint) in files {
        feed(path.as_bytes());
        feed(fingerprint.as_bytes());
    }
    format!("local:{hash:016x}")
}

pub(super) fn thread_position(
    old_path: Option<&str>,
    new_path: Option<&str>,
    old_line: Option<u32>,
    new_line: Option<u32>,
) -> Option<brief::Position> {
    let path = new_path.or(old_path)?;
    let (side, line) = match (new_line, old_line) {
        (Some(line), _) => (Side::New, line),
        (None, Some(line)) => (Side::Old, line),
        (None, None) => return None,
    };
    Some(brief::Position {
        path: path.to_owned(),
        side,
        line,
    })
}

pub(super) fn assemble(inputs: BriefInputs) -> Brief {
    let mut hidden = Vec::new();
    let mut viewed = Vec::new();
    let mut files = Vec::with_capacity(inputs.files.len());
    for file in inputs.files {
        if file.hidden {
            hidden.push(file.path.clone());
        } else if file.viewed {
            viewed.push(file.path.clone());
        }
        let status = match file.status {
            FileStatus::Added => brief::Status::Added,
            FileStatus::Deleted => brief::Status::Deleted,
            FileStatus::Modified => brief::Status::Modified,
            FileStatus::Renamed => brief::Status::Renamed,
        };
        let counted = (!file.binary).then_some((file.additions, file.deletions));
        files.push(brief::File {
            old_path: file.old_path.filter(|old| *old != file.path),
            path: file.path,
            status,
            added: counted.map(|(added, _)| added),
            removed: counted.map(|(_, removed)| removed),
        });
    }
    let mut threads: Vec<brief::Thread> = inputs
        .discussions
        .into_iter()
        .map(|discussion| brief::Thread {
            id: discussion.id,
            resolved: discussion.resolved,
            position: discussion.position,
            notes: discussion
                .notes
                .into_iter()
                .map(|(author, body)| brief::Note { author, body })
                .collect(),
        })
        .collect();
    threads.sort_by_key(|thread| thread.resolved);
    Brief {
        kind: inputs.kind,
        base_sha: inputs.base_sha,
        head_sha: inputs.head_sha,
        disk_matches_head: true,
        description: inputs.description,
        commits: inputs.commits,
        files,
        hidden,
        viewed,
        threads,
    }
}

pub(super) fn language_name(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "English",
        Lang::Ru => "Russian",
    }
}

pub(super) fn disk_matches(disk_head: Option<&str>, head_sha: Option<&str>) -> bool {
    match (disk_head, head_sha) {
        (Some(disk), Some(head)) => !head.is_empty() && disk == head,
        _ => false,
    }
}

pub(super) fn resolve_workdir(plan: &WorkdirPlan, fixed: Option<&Path>) -> Workdir {
    let cwd = match fixed {
        Some(cwd) => cwd.to_path_buf(),
        None => plan
            .branch
            .as_deref()
            .and_then(|branch| checkout_of(&plan.repo, branch))
            .unwrap_or_else(|| plan.repo.root().to_path_buf()),
    };
    let Some(head_sha) = plan.head_sha.as_deref() else {
        return Workdir {
            cwd,
            disk_matches_head: true,
        };
    };
    let disk_head = if cwd == plan.repo.root() {
        plan.repo.resolve("HEAD").ok()
    } else {
        Repo::open(&cwd)
            .and_then(|repo| repo.resolve("HEAD"))
            .inspect_err(|err| log::warn!("{}: {err}", cwd.display()))
            .ok()
    };
    Workdir {
        disk_matches_head: disk_matches(disk_head.as_deref(), Some(head_sha)),
        cwd,
    }
}

pub(in crate::review) fn from_view(view: &ReviewView) -> Option<ReviewContext> {
    let review = view.review.as_deref()?;
    if review.commit.is_some() || !matches!(view.load_state, LoadState::Ready) {
        return None;
    }
    let (kind, head_sha, description, branch, local) = match (&view.source, &view.mr) {
        (ReviewSource::MergeRequest { .. }, Some(mr)) => {
            let request = &mr.request;
            let author = if request.author.name.is_empty() {
                request.author.username.clone()
            } else {
                request.author.name.clone()
            };
            let kind = brief::Kind::Request(brief::Request {
                kind: match request.kind {
                    ForgeKind::GitLab => brief::RequestKind::MergeRequest,
                    ForgeKind::GitHub => brief::RequestKind::PullRequest,
                },
                reference: request.reference(),
                title: request.title.clone(),
                url: request.web_url.clone(),
                author,
                source_branch: request.source_branch.clone(),
                target_branch: request.target_branch.clone(),
                state: match request.state {
                    forge::RequestState::Open => brief::RequestState::Open,
                    forge::RequestState::Merged => brief::RequestState::Merged,
                    forge::RequestState::Closed => brief::RequestState::Closed,
                },
                draft: request.draft,
            });
            (
                kind,
                Some(mr.refs.head_sha.clone()),
                request.description.clone().unwrap_or_default(),
                Some(request.source_branch.clone()),
                false,
            )
        }
        (ReviewSource::MergeRequest { .. }, None) => return None,
        (ReviewSource::Branches { .. }, _) => (
            brief::Kind::Branches {
                source: review.source.clone(),
                target: review.target.clone(),
            },
            Some(review.head_sha.clone()),
            String::new(),
            Some(review.source.clone()),
            false,
        ),
        (ReviewSource::WorkingTree, _) => (
            brief::Kind::Local {
                branch: Some(review.target.clone()).filter(|branch| !branch.is_empty()),
            },
            None,
            String::new(),
            None,
            true,
        ),
    };
    let head = if local {
        local_head(
            review
                .files
                .iter()
                .map(|file| (file.path.as_ref(), file.fingerprint.as_str())),
        )
    } else {
        head_sha.clone().unwrap_or_default()
    };
    let files = review
        .files
        .iter()
        .enumerate()
        .map(|(ix, file)| FileInput {
            path: file.path.to_string(),
            old_path: Some(file.old_path.to_string()),
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            binary: file.binary,
            viewed: view.viewed.get(ix).copied().unwrap_or(false),
            hidden: view.hidden_mask.get(ix).copied().unwrap_or(false),
        })
        .collect();
    let discussions = view
        .discussions
        .iter()
        .map(|discussion| DiscussionInput {
            id: discussion.id.clone(),
            resolved: discussion.resolved,
            position: discussion.position.as_ref().and_then(|position| {
                thread_position(
                    position.old_path.as_deref(),
                    position.new_path.as_deref(),
                    position.old_line,
                    position.new_line,
                )
            }),
            notes: discussion
                .notes
                .iter()
                .map(|note| (note.author.to_string(), note.body.to_string()))
                .collect(),
        })
        .collect();
    let commits = review
        .commits
        .iter()
        .map(|commit| brief::Commit {
            sha: commit.sha.clone(),
            subject: commit.subject.clone(),
            author: commit.author.clone(),
        })
        .collect();
    let brief = assemble(BriefInputs {
        kind,
        base_sha: Some(review.base_sha.clone()),
        head_sha: head_sha.clone(),
        description,
        commits,
        files,
        discussions,
    });
    Some(ReviewContext {
        repo: view.repo.root().to_string_lossy().into_owned(),
        key: review_key(&view.source),
        head,
        local,
        brief,
        workdir: WorkdirPlan {
            repo: view.repo.clone(),
            branch,
            head_sha,
        },
    })
}
