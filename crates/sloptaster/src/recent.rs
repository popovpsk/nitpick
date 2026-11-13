use std::path::{Path, PathBuf};

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

use crate::review::ReviewSource;

const LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredReview {
    Branches { target: String, source: String },
    MergeRequest { project: String, iid: u64 },
}

impl StoredReview {
    pub fn from_source(source: &ReviewSource) -> Self {
        match source {
            ReviewSource::Branches { target, source } => StoredReview::Branches {
                target: target.clone(),
                source: source.clone(),
            },
            ReviewSource::MergeRequest { project, iid } => StoredReview::MergeRequest {
                project: project.clone(),
                iid: *iid,
            },
        }
    }

    pub fn to_source(&self) -> ReviewSource {
        match self {
            StoredReview::Branches { target, source } => ReviewSource::Branches {
                target: target.clone(),
                source: source.clone(),
            },
            StoredReview::MergeRequest { project, iid } => ReviewSource::MergeRequest {
                project: project.clone(),
                iid: *iid,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRepo {
    pub path: PathBuf,
    #[serde(default)]
    pub opened_at: i64,
    #[serde(default)]
    pub last_review: Option<StoredReview>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    repos: Vec<RecentRepo>,
    #[serde(default)]
    open_windows: Vec<PathBuf>,
}

pub struct RecentRepos {
    path: PathBuf,
    data: RecentFile,
}

impl Global for RecentRepos {}

impl RecentRepos {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    pub fn repos(&self) -> &[RecentRepo] {
        &self.data.repos
    }

    pub fn open_windows(&self) -> &[PathBuf] {
        &self.data.open_windows
    }

    pub fn last_review(&self, repo: &Path) -> Option<&StoredReview> {
        self.data
            .repos
            .iter()
            .find(|r| r.path == repo)
            .and_then(|r| r.last_review.as_ref())
    }

    pub fn set_last_review(&mut self, repo: &Path, review: StoredReview) {
        if let Some(entry) = self.data.repos.iter_mut().find(|r| r.path == repo) {
            if entry.last_review.as_ref() == Some(&review) {
                return;
            }
            entry.last_review = Some(review);
            self.save();
        }
    }

    pub fn touch(&mut self, repo: &Path, now: i64) {
        let last_review = self
            .data
            .repos
            .iter()
            .find(|r| r.path == repo)
            .and_then(|r| r.last_review.clone());
        self.data.repos.retain(|r| r.path != repo);
        self.data.repos.insert(
            0,
            RecentRepo {
                path: repo.to_path_buf(),
                opened_at: now,
                last_review,
            },
        );
        self.data.repos.truncate(LIMIT);
        self.save();
    }

    pub fn set_open_windows(&mut self, repos: Vec<PathBuf>) {
        if self.data.open_windows != repos {
            self.data.open_windows = repos;
            self.save();
        }
    }

    fn save(&self) {
        match serde_json::to_vec_pretty(&self.data) {
            Ok(bytes) => {
                if let Err(err) = review_core::write_atomic(&self.path, &bytes) {
                    log::warn!("recent repos: {err}");
                }
            }
            Err(err) => log::warn!("recent repos: {err}"),
        }
    }
}

pub fn recent(cx: &App) -> &RecentRepos {
    cx.global::<RecentRepos>()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
