use std::collections::HashMap;

use anyhow::{Context as _, anyhow};
use git_repo::{ChangeStatus, ChangedFile, Comparison, DiffOptions, Repo};
use gpui::SharedString;
use gpui_component::diff::{DiffFile, DiffParseOptions};
use review_core::{FileStatus, FileTree, LineMap, TreeFile};

const FULL_CONTEXT: u32 = 1_000_000;
const SHORT_CONTEXT: u32 = 3;
const FULL_CONTEXT_PATCH_LIMIT: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewSource {
    Branches { target: String, source: String },
    MergeRequest { project: String, iid: u64 },
}

impl ReviewSource {
    pub fn label(&self) -> SharedString {
        match self {
            ReviewSource::Branches { target, source } => format!("{target} ← {source}").into(),
            ReviewSource::MergeRequest { iid, .. } => format!("!{iid}").into(),
        }
    }
}

#[derive(Clone)]
pub struct MrContext {
    pub client: gitlab::Client,
    pub project: gitlab::ProjectRef,
    pub project_path: String,
    pub mr: gitlab::MergeRequest,
    pub refs: gitlab::DiffRefs,
}

pub struct ReviewFile {
    pub diff: DiffFile,
    pub path: SharedString,
    pub old_path: SharedString,
    pub new_path: SharedString,
    pub fingerprint: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub line_map: LineMap,
}

pub struct LoadedReview {
    pub base_sha: String,
    pub head_sha: String,
    pub target: String,
    pub source: String,
    pub merge_base_refs: Vec<String>,
    pub files: Vec<ReviewFile>,
    pub tree: FileTree,
    pub context_limited: bool,
    pub comments_aligned: bool,
    pub commits: Vec<git_repo::Commit>,
}

impl LoadedReview {
    pub fn file_index(&self, path: &str) -> Option<usize> {
        self.files.iter().position(|f| f.path.as_ref() == path)
    }
}

pub fn build(
    repo: &Repo,
    comparison: Comparison,
    ignore_whitespace: bool,
) -> anyhow::Result<LoadedReview> {
    let base = comparison.base_sha.clone();
    let head = comparison.head_sha.clone();
    let mut options = DiffOptions {
        context_lines: SHORT_CONTEXT,
        find_renames: true,
        ignore_whitespace,
    };
    let changed = repo
        .changed_files(&base, &head, &options)
        .context("listing changed files")?;
    options.context_lines = FULL_CONTEXT;
    let mut patch = repo
        .diff(&base, &head, &options)
        .context("computing the diff")?;
    let mut context_limited = false;
    if patch.len() > FULL_CONTEXT_PATCH_LIMIT {
        options.context_lines = SHORT_CONTEXT;
        patch = repo
            .diff(&base, &head, &options)
            .context("computing the diff")?;
        context_limited = true;
    }
    let diffs = parse(&patch)?;
    let (maps, comments_aligned) = if ignore_whitespace {
        options.ignore_whitespace = false;
        drop(patch);
        let exact = repo
            .diff(&base, &head, &options)
            .context("computing the diff")?;
        let exact_files = parse(&exact)?;
        let (exact_maps, aligned) = aligned_maps(&exact, exact_files.len());
        let by_path: HashMap<SharedString, LineMap> = exact_files
            .iter()
            .map(|f| f.path().clone())
            .zip(exact_maps)
            .collect();
        let maps = diffs
            .iter()
            .map(|f| by_path.get(f.path()).cloned().unwrap_or_default())
            .collect();
        (maps, aligned)
    } else {
        let (maps, aligned) = aligned_maps(&patch, diffs.len());
        drop(patch);
        (maps, aligned)
    };
    let by_path: HashMap<&str, &ChangedFile> =
        changed.iter().map(|f| (f.path.as_str(), f)).collect();
    let files: Vec<ReviewFile> = diffs
        .into_iter()
        .zip(maps)
        .map(|(diff, line_map)| review_file(diff, line_map, &by_path))
        .collect();
    let tree = FileTree::build(
        &files
            .iter()
            .map(|f| TreeFile {
                path: f.path.to_string(),
                status: f.status,
                additions: f.additions,
                deletions: f.deletions,
            })
            .collect::<Vec<_>>(),
    );
    let commits = repo.commits(&base, &head).unwrap_or_else(|err| {
        log::warn!("commits {base}..{head}: {err}");
        Vec::new()
    });
    Ok(LoadedReview {
        base_sha: base,
        head_sha: head,
        target: comparison.target,
        source: comparison.source,
        merge_base_refs: comparison.merge_base_refs,
        files,
        tree,
        context_limited,
        comments_aligned,
        commits,
    })
}

fn parse(patch: &str) -> anyhow::Result<Vec<DiffFile>> {
    grammars::register_highlight_queries();
    let files = DiffFile::parse_with(patch, DiffParseOptions::new().truncated_hunks(true))
        .map_err(|err| anyhow!("parsing the diff, line {}: {}", err.line(), err.message()))?;
    Ok(files.into_iter().map(with_language).collect())
}

fn with_language(file: DiffFile) -> DiffFile {
    match review_core::language_override(file.path()) {
        Some(language) => file.with_language(language),
        None => file,
    }
}

fn aligned_maps(patch: &str, files: usize) -> (Vec<LineMap>, bool) {
    let maps = review_core::line_maps(patch);
    if maps.len() == files {
        (maps, true)
    } else {
        log::warn!(
            "line maps: {} files in the patch walk, {files} parsed",
            maps.len()
        );
        (vec![LineMap::default(); files], false)
    }
}

fn review_file(
    diff: DiffFile,
    line_map: LineMap,
    by_path: &HashMap<&str, &ChangedFile>,
) -> ReviewFile {
    let path = diff.path().clone();
    let changed = by_path.get(path.as_ref()).copied();
    let status = match changed.map(|c| c.status) {
        Some(ChangeStatus::Added) => FileStatus::Added,
        Some(ChangeStatus::Deleted) => FileStatus::Deleted,
        Some(ChangeStatus::Renamed | ChangeStatus::Copied) => FileStatus::Renamed,
        Some(_) => FileStatus::Modified,
        None => match (diff.original_path(), diff.modified_path()) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            (Some(a), Some(b)) if a != b => FileStatus::Renamed,
            _ => FileStatus::Modified,
        },
    };
    let fingerprint = match changed {
        Some(c) => review_core::fingerprint(&c.old_blob, &c.new_blob),
        None => index_fingerprint(&diff).unwrap_or_else(|| format!("?{path}")),
    };
    let old_path = diff
        .original_path()
        .cloned()
        .unwrap_or_else(|| path.clone());
    let new_path = diff
        .modified_path()
        .cloned()
        .unwrap_or_else(|| path.clone());
    ReviewFile {
        additions: diff.additions() as u32,
        deletions: diff.deletions() as u32,
        binary: diff.is_binary(),
        diff,
        path,
        old_path,
        new_path,
        fingerprint,
        status,
        line_map,
    }
}

fn index_fingerprint(diff: &DiffFile) -> Option<String> {
    diff.extended_headers().iter().find_map(|h| {
        let rest = h.strip_prefix("index ")?;
        let ids = rest.split_whitespace().next()?;
        let (old, new) = ids.split_once("..")?;
        Some(review_core::fingerprint(old, new))
    })
}

pub fn gitlab_remote(
    repo: &Repo,
    base_url: &str,
    project: Option<&str>,
) -> Option<(String, String)> {
    let host = host_of(base_url)?;
    let remotes = repo.remotes().ok()?;
    let mut origin = None;
    let mut fallback = None;
    for remote in remotes {
        let Some(location) = git_repo::parse_remote_url(&remote.url) else {
            continue;
        };
        if host_of(&location.https_base).as_deref() != Some(host.as_str()) {
            continue;
        }
        if project.is_some_and(|p| p.eq_ignore_ascii_case(&location.path)) {
            return Some((remote.name, location.path));
        }
        if remote.name == "origin" {
            origin = Some((remote.name, location.path));
        } else {
            fallback.get_or_insert((remote.name, location.path));
        }
    }
    origin.or(fallback)
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

pub fn mr_refspecs(mr: &gitlab::MergeRequest) -> Vec<String> {
    vec![format!(
        "+refs/merge-requests/{0}/head:refs/slopcop/mr/{0}",
        mr.iid
    )]
}

pub fn comparison_for_mr(mr: &gitlab::MergeRequest, refs: &gitlab::DiffRefs) -> Comparison {
    Comparison {
        base_sha: refs.base_sha.clone(),
        head_sha: refs.head_sha.clone(),
        target: mr.target_branch.clone(),
        source: mr.source_branch.clone(),
        target_sha: refs.start_sha.clone(),
        merge_base_refs: vec![mr.target_branch.clone()],
    }
}

#[path = "model_grammars.rs"]
mod grammars;

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
