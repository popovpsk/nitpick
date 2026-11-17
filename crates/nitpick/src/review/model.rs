use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use forge::ForgeKind;
use git_repo::{ChangeStatus, ChangedFile, Comparison, DiffOptions, Repo};
use gpui::SharedString;
use gpui_component::diff::{DiffFile, DiffParseOptions};
use review_core::{Change, FileStatus, FileTree, LineMap, TreeFile};

const FULL_CONTEXT: u32 = 1_000_000;
const SHORT_CONTEXT: u32 = 3;
const FULL_CONTEXT_PATCH_LIMIT: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewSource {
    Branches {
        target: String,
        source: String,
    },
    MergeRequest {
        forge: ForgeKind,
        project: String,
        number: u64,
    },
    WorkingTree,
}

impl ReviewSource {
    pub fn label(&self) -> SharedString {
        match self {
            ReviewSource::Branches { target, source } => format!("{target} ← {source}").into(),
            ReviewSource::MergeRequest { forge, number, .. } => forge.reference(*number).into(),
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
    pub client: forge::Client,
    pub project: String,
    pub request: forge::Request,
    pub refs: MrRefs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MrRefs {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
}

impl MrContext {
    pub fn kind(&self) -> ForgeKind {
        self.request.kind
    }
}

pub struct ReviewFile {
    pub patch: Range<usize>,
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
    pub commit: Option<git_repo::Commit>,
    pub merge_base_sha: String,
    pub patch: String,
    parsed: RefCell<Option<(usize, DiffFile)>>,
}

impl LoadedReview {
    pub fn file_index(&self, path: &str) -> Option<usize> {
        self.files.iter().position(|f| f.path.as_ref() == path)
    }

    pub fn diff_file(&self, ix: usize) -> Option<DiffFile> {
        if let Some((cached, diff)) = self.parsed.borrow().as_ref()
            && *cached == ix
        {
            return Some(diff.clone());
        }
        let file = self.files.get(ix)?;
        let section = self.patch.get(file.patch.clone())?;
        let diff = parse_file(section)
            .inspect_err(|err| log::warn!("{}: {err:#}", file.path))
            .ok()?;
        *self.parsed.borrow_mut() = Some((ix, diff.clone()));
        Some(diff)
    }

    pub fn commit_sha(&self) -> Option<&str> {
        self.commit.as_ref().map(|c| c.sha.as_str())
    }
}

struct ReviewRange {
    commits: Vec<git_repo::Commit>,
    commit: Option<git_repo::Commit>,
    merge_base_sha: String,
}

pub fn build(
    repo: &Repo,
    comparison: Comparison,
    commit: Option<&str>,
    ignore_whitespace: bool,
) -> anyhow::Result<LoadedReview> {
    let (base, head) = (&comparison.base_sha, &comparison.head_sha);
    let commits = repo.commits(base, head).unwrap_or_else(|err| {
        log::warn!("commits {base}..{head}: {err}");
        Vec::new()
    });
    let chosen = commit.and_then(|sha| commits.iter().find(|c| c.sha == sha).cloned());
    let merge_base_sha = comparison.base_sha.clone();
    let (comparison, head) = match &chosen {
        Some(commit) => {
            let parent = repo
                .first_parent_or_empty_tree(&commit.sha)
                .context("resolving the commit's parent")?;
            let comparison = Comparison {
                base_sha: parent,
                head_sha: commit.sha.clone(),
                ..comparison
            };
            (comparison, commit.sha.clone())
        }
        None => {
            let head = comparison.head_sha.clone();
            (comparison, head)
        }
    };
    let range = ReviewRange {
        commits,
        commit: chosen,
        merge_base_sha,
    };
    build_range(
        repo,
        comparison,
        Head::Commit(&head),
        range,
        ignore_whitespace,
    )
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
    let range = ReviewRange {
        commits: Vec::new(),
        commit: None,
        merge_base_sha: comparison.base_sha.clone(),
    };
    build_range(
        repo,
        comparison,
        Head::WorkingTree,
        range,
        ignore_whitespace,
    )
}

fn build_range(
    repo: &Repo,
    comparison: Comparison,
    range_head: Head<'_>,
    range: ReviewRange,
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
    let spans = review_core::file_spans(&patch);
    let by_path: HashMap<&str, &ChangedFile> =
        changed.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut files: Vec<ReviewFile> = spans
        .into_iter()
        .map(|span| {
            let diff = parse_file(&patch[span.clone()])?;
            Ok(review_file(span, &diff, &by_path))
        })
        .collect::<anyhow::Result<_>>()?;
    let comments_aligned = if ignore_whitespace {
        let display = display_maps(&patch, files.len());
        options.ignore_whitespace = false;
        let exact = range_head
            .diff(repo, &base, &options)
            .context("computing the diff")?;
        let exact_spans = review_core::file_spans(&exact);
        let (exact_maps, aligned) = aligned_maps(&exact, exact_spans.len());
        let mut by_path: HashMap<SharedString, VecDeque<LineMap>> = HashMap::new();
        for (span, map) in exact_spans.into_iter().zip(exact_maps) {
            by_path
                .entry(parse_file(&exact[span])?.path().clone())
                .or_default()
                .push_back(map);
        }
        for (file, display_map) in files.iter_mut().zip(display) {
            file.line_map = by_path
                .get_mut(&file.path)
                .and_then(VecDeque::pop_front)
                .unwrap_or_default();
            file.changes = display_map.changes().to_vec();
            file.display_map = Some(display_map);
        }
        aligned
    } else {
        let (maps, aligned) = aligned_maps(&patch, files.len());
        for (file, line_map) in files.iter_mut().zip(maps) {
            file.changes = line_map.changes().to_vec();
            file.line_map = line_map;
        }
        aligned
    };
    let tree = Arc::new(FileTree::build(&tree_files(&files)));
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
        commits: range.commits,
        commit: range.commit,
        merge_base_sha: range.merge_base_sha,
        patch,
        parsed: RefCell::new(None),
    })
}

pub struct Interdiff {
    pub file: usize,
    pub key: String,
    pub previous: String,
    pub diff: Option<DiffFile>,
    pub map: LineMap,
    pub changes: Vec<Change>,
}

impl Interdiff {
    pub fn is_empty(&self) -> bool {
        self.diff.is_none()
    }
}

pub fn interdiff(
    repo: &Repo,
    file: usize,
    key: String,
    path: &str,
    previous: &str,
    current: &str,
    ignore_whitespace: bool,
) -> anyhow::Result<Interdiff> {
    let options = DiffOptions {
        context_lines: FULL_CONTEXT,
        find_renames: false,
        ignore_whitespace,
    };
    let patch = repo
        .diff_blobs(previous, current, path, &options)
        .context("comparing with the version viewed before")?;
    let diff = if patch.contains("\n@@ ") {
        Some(
            parse(&patch)?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("comparing with the version viewed before: no file"))?,
        )
    } else {
        None
    };
    let map = review_core::line_maps(&patch)
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok(Interdiff {
        file,
        key,
        previous: previous.to_string(),
        changes: map.changes().to_vec(),
        diff,
        map,
    })
}

fn parse(patch: &str) -> anyhow::Result<Vec<DiffFile>> {
    grammars::register_highlight_queries();
    let files = DiffFile::parse_with(patch, DiffParseOptions::new().truncated_hunks(true))
        .map_err(|err| anyhow!("parsing the diff, line {}: {}", err.line(), err.message()))?;
    Ok(files.into_iter().map(with_language).collect())
}

fn parse_file(section: &str) -> anyhow::Result<DiffFile> {
    let mut files = parse(section)?;
    match (files.pop(), files.is_empty()) {
        (Some(file), true) => Ok(file),
        _ => Err(anyhow!(
            "parsing the diff: not one file in a `diff --git` section"
        )),
    }
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
    patch: Range<usize>,
    diff: &DiffFile,
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
    let blobs = changed
        .map(|c| (c.old_blob.clone(), c.new_blob.clone()))
        .or_else(|| index_blobs(diff));
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
        patch,
        path,
        old_path,
        new_path,
        fingerprint,
        old_blob,
        new_blob,
        status,
        line_map: LineMap::default(),
        display_map: None,
        changes: Vec::new(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeRemote {
    pub kind: ForgeKind,
    pub remote: String,
    pub project: String,
}

pub fn forge_remote(
    repo: &Repo,
    forges: &[(ForgeKind, String)],
    project: Option<&str>,
) -> Option<ForgeRemote> {
    let hosts: Vec<(ForgeKind, String)> = forges
        .iter()
        .filter_map(|(kind, url)| Some((*kind, host_of(url)?)))
        .collect();
    let remotes = repo.remotes().ok()?;
    let mut upstream = None;
    let mut origin = None;
    let mut fallback = None;
    for remote in remotes {
        let Some(location) = git_repo::parse_remote_url(&remote.url) else {
            continue;
        };
        let Some(host) = host_of(&location.https_base) else {
            continue;
        };
        let Some(kind) = hosts.iter().find(|(_, h)| *h == host).map(|(k, _)| *k) else {
            continue;
        };
        let found = ForgeRemote {
            kind,
            remote: remote.name,
            project: location.path,
        };
        if project.is_some_and(|p| p.eq_ignore_ascii_case(&found.project)) {
            return Some(found);
        }
        match found.remote.as_str() {
            "upstream" => upstream = Some(found),
            "origin" => origin = Some(found),
            _ => {
                fallback.get_or_insert(found);
            }
        }
    }
    match (upstream, origin) {
        (Some(upstream), Some(origin)) if upstream.kind == origin.kind => Some(upstream),
        (upstream, origin) => origin.or(upstream).or(fallback),
    }
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

pub fn resolve_mr_refs(repo: &Repo, refs: &forge::DiffRefs) -> anyhow::Result<MrRefs> {
    let base_sha = match &refs.base_sha {
        Some(base) => base.clone(),
        None => {
            repo.compare(&refs.start_sha, &refs.head_sha)
                .context("finding the merge base")?
                .base_sha
        }
    };
    Ok(MrRefs {
        base_sha,
        start_sha: refs.start_sha.clone(),
        head_sha: refs.head_sha.clone(),
    })
}

pub fn comparison_for_mr(request: &forge::Request, refs: &MrRefs) -> Comparison {
    Comparison {
        base_sha: refs.base_sha.clone(),
        head_sha: refs.head_sha.clone(),
        target: request.target_branch.clone(),
        source: request.source_branch.clone(),
        target_sha: refs.start_sha.clone(),
        merge_base_refs: vec![request.target_branch.clone()],
    }
}

#[path = "model_grammars.rs"]
pub(super) mod grammars;

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
