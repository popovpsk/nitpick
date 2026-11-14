use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use git_repo::{ChangeStatus, ChangedFile, Comparison, DiffOptions, Repo};
use gpui::SharedString;
use gpui_component::diff::{DiffFile, DiffParseOptions};
use review_core::{Change, FileStatus, FileTree, LineMap, TreeFile};

const FULL_CONTEXT: u32 = 1_000_000;
const SHORT_CONTEXT: u32 = 3;
const FULL_CONTEXT_PATCH_LIMIT: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewSource {
    Branches { target: String, source: String },
    MergeRequest { project: String, iid: u64 },
    WorkingTree,
}

impl ReviewSource {
    pub fn label(&self) -> SharedString {
        match self {
            ReviewSource::Branches { target, source } => format!("{target} ← {source}").into(),
            ReviewSource::MergeRequest { iid, .. } => format!("!{iid}").into(),
            ReviewSource::WorkingTree => crate::localization::tr().uncommitted_changes.clone(),
        }
    }
}

#[derive(Clone, Copy)]
enum Head<'a> {
    Commit(&'a str),
    WorkingTree,
}

impl Head<'_> {
    fn changed_files(
        self,
        repo: &Repo,
        base: &str,
        options: &DiffOptions,
    ) -> Result<Vec<ChangedFile>, git_repo::GitError> {
        match self {
            Head::Commit(head) => repo.changed_files(base, head, options),
            Head::WorkingTree => repo.working_tree_changes(base, options),
        }
    }

    fn diff(
        self,
        repo: &Repo,
        base: &str,
        options: &DiffOptions,
    ) -> Result<String, git_repo::GitError> {
        match self {
            Head::Commit(head) => repo.diff(base, head, options),
            Head::WorkingTree => repo.working_tree_diff(base, options),
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
    pub old_blob: String,
    pub new_blob: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub line_map: LineMap,
    pub display_map: Option<LineMap>,
    pub changes: Vec<Change>,
}

impl ReviewFile {
    pub fn shown_map(&self) -> &LineMap {
        self.display_map.as_ref().unwrap_or(&self.line_map)
    }
}

pub struct LoadedReview {
    pub base_sha: String,
    pub head_sha: String,
    pub target: String,
    pub source: String,
    pub merge_base_refs: Vec<String>,
    pub files: Vec<ReviewFile>,
    pub tree: Arc<FileTree>,
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
    let head = comparison.head_sha.clone();
    build_range(repo, comparison, Head::Commit(&head), ignore_whitespace)
}

pub fn build_working_tree(repo: &Repo, ignore_whitespace: bool) -> anyhow::Result<LoadedReview> {
    let base = repo.head_or_empty_tree().context("resolving HEAD")?;
    let branch = repo
        .current_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| "HEAD".to_string());
    let comparison = Comparison {
        base_sha: base.clone(),
        head_sha: String::new(),
        target: branch,
        source: String::new(),
        target_sha: base,
        merge_base_refs: Vec::new(),
    };
    build_range(repo, comparison, Head::WorkingTree, ignore_whitespace)
}

fn build_range(
    repo: &Repo,
    comparison: Comparison,
    range_head: Head<'_>,
    ignore_whitespace: bool,
) -> anyhow::Result<LoadedReview> {
    let base = comparison.base_sha.clone();
    let head = comparison.head_sha.clone();
    let mut options = DiffOptions {
        context_lines: SHORT_CONTEXT,
        find_renames: true,
        ignore_whitespace,
    };
    let changed = range_head
        .changed_files(repo, &base, &options)
        .context("listing changed files")?;
    options.context_lines = FULL_CONTEXT;
    let mut patch = range_head
        .diff(repo, &base, &options)
        .context("computing the diff")?;
    let mut context_limited = false;
    if patch.len() > FULL_CONTEXT_PATCH_LIMIT {
        options.context_lines = SHORT_CONTEXT;
        patch = range_head
            .diff(repo, &base, &options)
            .context("computing the diff")?;
        context_limited = true;
    }
    let diffs = parse(&patch)?;
    let (maps, display, comments_aligned) = if ignore_whitespace {
        let display = display_maps(&patch, diffs.len());
        options.ignore_whitespace = false;
        drop(patch);
        let exact = range_head
            .diff(repo, &base, &options)
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
        (maps, Some(display), aligned)
    } else {
        let (maps, aligned) = aligned_maps(&patch, diffs.len());
        drop(patch);
        (maps, None, aligned)
    };
    let mut display = display.map(Vec::into_iter);
    let by_path: HashMap<&str, &ChangedFile> =
        changed.iter().map(|f| (f.path.as_str(), f)).collect();
    let files: Vec<ReviewFile> = diffs
        .into_iter()
        .zip(maps)
        .map(|(diff, line_map)| {
            let display_map = display.as_mut().and_then(Iterator::next);
            review_file(diff, line_map, display_map, &by_path)
        })
        .collect();
    let tree = Arc::new(FileTree::build(&tree_files(&files)));
    let commits = match range_head {
        Head::Commit(head) => repo.commits(&base, head).unwrap_or_else(|err| {
            log::warn!("commits {base}..{head}: {err}");
            Vec::new()
        }),
        Head::WorkingTree => Vec::new(),
    };
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

pub fn tree_files(files: &[ReviewFile]) -> Vec<TreeFile> {
    files
        .iter()
        .map(|f| TreeFile {
            path: f.path.to_string(),
            status: f.status,
            additions: f.additions,
            deletions: f.deletions,
        })
        .collect()
}

fn display_maps(patch: &str, files: usize) -> Vec<LineMap> {
    let maps = review_core::line_maps(patch);
    if maps.len() == files {
        maps
    } else {
        vec![LineMap::default(); files]
    }
}

fn review_file(
    diff: DiffFile,
    line_map: LineMap,
    display_map: Option<LineMap>,
    by_path: &HashMap<&str, &ChangedFile>,
) -> ReviewFile {
    let changes: Vec<Change> = display_map.as_ref().unwrap_or(&line_map).changes().to_vec();
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
    let blobs = changed
        .map(|c| (c.old_blob.clone(), c.new_blob.clone()))
        .or_else(|| index_blobs(&diff));
    let (old_blob, new_blob, fingerprint) = match blobs {
        Some((old, new)) => {
            let fingerprint = review_core::fingerprint(&old, &new);
            (old, new, fingerprint)
        }
        None => (String::new(), String::new(), format!("?{path}")),
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
        old_blob,
        new_blob,
        status,
        line_map,
        display_map,
        changes,
    }
}

fn index_blobs(diff: &DiffFile) -> Option<(String, String)> {
    diff.extended_headers().iter().find_map(|h| {
        let rest = h.strip_prefix("index ")?;
        let ids = rest.split_whitespace().next()?;
        let (old, new) = ids.split_once("..")?;
        Some((old.to_string(), new.to_string()))
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
        "+refs/merge-requests/{0}/head:refs/lgtm-ish/mr/{0}",
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
