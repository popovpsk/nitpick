use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

use crate::review::ReviewSource;

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
            .and_then(|bytes| serde_json::from_slice::<RecentFile>(&bytes).ok())
            .unwrap_or_default();
        let mut this = Self { path, data };
        sort(&mut this.data.repos);
        this
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
        let opened_at = self
            .data
            .repos
            .iter()
            .map(|r| r.opened_at)
            .max()
            .map_or(now, |latest| now.max(latest + 1));
        match self.data.repos.iter_mut().find(|r| r.path == repo) {
            Some(entry) => entry.opened_at = opened_at,
            None => self.data.repos.push(RecentRepo {
                path: repo.to_path_buf(),
                opened_at,
                last_review: None,
            }),
        }
        sort(&mut self.data.repos);
        self.save();
    }

    pub fn add(&mut self, repos: &[PathBuf]) -> usize {
        let mut known: HashSet<PathBuf> = self.data.repos.iter().map(|r| r.path.clone()).collect();
        let before = self.data.repos.len();
        for path in repos {
            if known.insert(path.clone()) {
                self.data.repos.push(RecentRepo {
                    path: path.clone(),
                    opened_at: 0,
                    last_review: None,
                });
            }
        }
        let added = self.data.repos.len() - before;
        if added > 0 {
            sort(&mut self.data.repos);
            self.save();
        }
        added
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

fn sort(repos: &mut [RecentRepo]) {
    repos.sort_by_cached_key(|r| {
        let name = r
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        (Reverse(r.opened_at), name, r.path.clone())
    });
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

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
