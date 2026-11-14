mod branch_picker;
mod comments;
mod diff_panel;
pub mod model;
mod tree_panel;
mod viewed;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use git_repo::{PullOutcome, Repo};
use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, ElementId, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, KeyBinding, SharedString, Subscription, Task,
    UniformListScrollHandle, Window, actions,
};
use gpui_component::{
    WindowExt as _,
    diff::{
        DiffAnnotation, DiffEvent, DiffLinePosition, DiffLineRange, DiffMode, DiffSide, DiffState,
    },
    input::{InputEvent, TextareaState},
    notification::Notification,
};
use review_core::{FileStatus, FileTree, HideRules, LineKind, Progress, Side};

use crate::localization::tr;
use crate::settings_store::{
    DiffLayout, SettingsStore, TREE_WIDTH_RANGE, settings, update_settings,
};
pub use model::{LoadedReview, MrContext, ReviewSource};
use viewed::SharedViewed;
pub use viewed::flush_all as flush_viewed;

actions!(
    review,
    [
        NextFile,
        PreviousFile,
        NextChange,
        PreviousChange,
        ToggleViewed,
        CollapseAll,
        ExpandAll,
        SelectNext,
        SelectPrevious,
        CollapseSelected,
        ExpandSelected,
        OpenSelected,
        FocusTree,
        FocusDiff,
        Refresh,
        ToggleTree,
    ]
);

const TREE_SLIDE: Duration = Duration::from_millis(200);
const TREE_WIDTH_SAVE_DELAY: Duration = Duration::from_millis(400);
const AUTO_MARK_DWELL: Duration = Duration::from_millis(1000);
const CROSS_HINT_TIMEOUT: Duration = Duration::from_millis(3000);

static TREE_WIDTH_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("ReviewTree")),
        KeyBinding::new("j", SelectNext, Some("ReviewTree")),
        KeyBinding::new("up", SelectPrevious, Some("ReviewTree")),
        KeyBinding::new("k", SelectPrevious, Some("ReviewTree")),
        KeyBinding::new("left", CollapseSelected, Some("ReviewTree")),
        KeyBinding::new("h", CollapseSelected, Some("ReviewTree")),
        KeyBinding::new("right", ExpandSelected, Some("ReviewTree")),
        KeyBinding::new("l", ExpandSelected, Some("ReviewTree")),
        KeyBinding::new("enter", OpenSelected, Some("ReviewTree")),
        KeyBinding::new("space", ToggleViewed, Some("ReviewTree")),
        KeyBinding::new("v", ToggleViewed, Some("ReviewTree")),
        KeyBinding::new("n", NextFile, Some("ReviewTree")),
        KeyBinding::new("p", PreviousFile, Some("ReviewTree")),
        KeyBinding::new("space", ToggleViewed, Some("Review")),
        KeyBinding::new("alt-down", NextChange, Some("Review")),
        KeyBinding::new("alt-up", PreviousChange, Some("Review")),
        KeyBinding::new("f7", NextChange, Some("Review")),
        KeyBinding::new("shift-f7", PreviousChange, Some("Review")),
        KeyBinding::new("alt-right", NextFile, Some("Review")),
        KeyBinding::new("alt-left", PreviousFile, Some("Review")),
        KeyBinding::new("cmd-1", FocusTree, Some("Review")),
        KeyBinding::new("ctrl-1", FocusTree, Some("Review")),
        KeyBinding::new("cmd-2", FocusDiff, Some("Review")),
        KeyBinding::new("ctrl-2", FocusDiff, Some("Review")),
        KeyBinding::new("cmd-b", ToggleTree, Some("Review")),
        KeyBinding::new("ctrl-b", ToggleTree, Some("Review")),
        KeyBinding::new("cmd-r", Refresh, Some("Review")),
        KeyBinding::new("ctrl-r", Refresh, Some("Review")),
    ]);
}

#[derive(Clone)]
pub struct TreeRow {
    pub node: usize,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub name: SharedString,
    pub file: Option<usize>,
    pub status: FileStatus,
    pub progress: Progress,
    pub comments: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CrossHint {
    PressForNextFile,
    PressForPreviousFile,
    NoNextFile,
    NoPreviousFile,
}

#[derive(Clone)]
pub struct HiddenFile {
    pub ix: usize,
    pub path: SharedString,
    pub rule: SharedString,
}

pub enum LoadState {
    Loading,
    Failed(SharedString),
    Ready,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PullSide {
    Target,
    Source,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Pulling(PullSide),
    Refreshing,
    Posting,
}

#[derive(Clone)]
pub enum ComposerTarget {
    Line(DiffLinePosition),
    Reply(String),
    General,
}

pub struct Composer {
    pub target: ComposerTarget,
    pub input: Entity<TextareaState>,
    _submit: Subscription,
}

#[derive(Clone)]
pub struct NoteView {
    pub id: u64,
    pub author: SharedString,
    pub meta: SharedString,
    pub body: SharedString,
}

#[derive(Clone)]
pub struct DiscussionView {
    pub id: String,
    pub notes: Vec<NoteView>,
    pub resolvable: bool,
    pub resolved: bool,
    pub position: Option<gitlab::Position>,
    pub location: Option<SharedString>,
}

impl DiscussionView {
    fn from_gitlab(discussion: &gitlab::Discussion) -> Option<Self> {
        let notes: Vec<NoteView> = discussion
            .notes
            .iter()
            .filter(|n| !n.system)
            .map(|note| {
                let time = note
                    .created_at
                    .get(..16)
                    .unwrap_or(&note.created_at)
                    .replace('T', " ");
                NoteView {
                    id: note.id,
                    author: note.author.name.clone().into(),
                    meta: format!("@{} · {time}", note.author.username).into(),
                    body: note.body.clone().into(),
                }
            })
            .collect();
        if notes.is_empty() {
            return None;
        }
        let position = discussion.position().cloned();
        let location = position
            .as_ref()
            .and_then(|p| p.new_path.clone().or_else(|| p.old_path.clone()))
            .map(SharedString::from);
        Some(Self {
            id: discussion.id.clone(),
            notes,
            resolvable: discussion.is_resolvable(),
            resolved: discussion.is_resolved(),
            position,
            location,
        })
    }
}

pub enum ReviewEvent {
    Changed,
    Open(ReviewSource),
}

impl EventEmitter<ReviewEvent> for ReviewView {}

pub struct ReviewView {
    repo: Repo,
    source: ReviewSource,
    load_state: LoadState,
    review: Option<Rc<LoadedReview>>,
    mr: Option<MrContext>,
    discussions: Rc<Vec<DiscussionView>>,
    by_file: HashMap<SharedString, Vec<usize>>,
    general: Vec<usize>,
    discussions_loading: bool,
    viewed: Vec<bool>,
    tree: Arc<FileTree>,
    hidden: Rc<Vec<HiddenFile>>,
    hide_patterns: (Vec<String>, Vec<String>),
    viewed_store: Entity<SharedViewed>,
    loaded_whitespace: bool,
    window_handle: AnyWindowHandle,
    collapsed: HashSet<String>,
    rows: Rc<Vec<TreeRow>>,
    selected_row: Option<usize>,
    current_file: Option<usize>,
    annotations: Rc<Vec<DiffAnnotation>>,
    diff: Entity<DiffState>,
    tree_focus: FocusHandle,
    tree_scroll: UniformListScrollHandle,
    composer: Option<Composer>,
    kept_unviewed: HashSet<(SharedString, String)>,
    change_cursor: Option<usize>,
    change_scrolled: bool,
    cross_hint: Option<CrossHint>,
    _cross_hint_clear: Task<()>,
    shown_at: Option<Instant>,
    installed_whitespace: bool,
    tree_visible: bool,
    tree_closing: bool,
    tree_toggles: usize,
    tree_width: f32,
    busy: Option<Busy>,
    _tree_close: Task<()>,
    branch_pickers: branch_picker::BranchPickers,
    _load: Task<()>,
    _discussions: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ReviewView {
    pub fn new(
        repo: Repo,
        source: ReviewSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = match settings(cx).diff_layout {
            DiffLayout::Split => DiffMode::Split,
            DiffLayout::Unified => DiffMode::Unified,
        };
        let diff = cx.new(|cx| DiffState::new([], cx).with_mode(mode));
        let subscription = cx.subscribe_in(&diff, window, |_, _, event: &DiffEvent, _, cx| {
            if matches!(
                event,
                DiffEvent::FileCollapsed(_) | DiffEvent::FileExpanded(_)
            ) {
                cx.notify();
            }
        });
        let viewed_store = viewed::shared_for(repo.root(), cx);
        let viewed_sync = cx.observe(&viewed_store, |this, _, cx| this.sync_viewed(cx));
        let settings_sync = cx.observe_global::<SettingsStore>(|this, cx| this.apply_settings(cx));
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active()
                && this.source == ReviewSource::WorkingTree
                && this.busy.is_none()
                && this.review.is_some()
            {
                this.reload(cx);
            }
        });
        let branch_pickers = branch_picker::BranchPickers::new(window, cx);
        let mut this = Self {
            repo,
            source,
            load_state: LoadState::Loading,
            review: None,
            mr: None,
            discussions: Rc::default(),
            by_file: HashMap::new(),
            general: Vec::new(),
            discussions_loading: false,
            viewed: Vec::new(),
            tree: Arc::default(),
            hidden: Rc::default(),
            hide_patterns: (Vec::new(), Vec::new()),
            viewed_store,
            loaded_whitespace: settings(cx).ignore_whitespace,
            window_handle: window.window_handle(),
            collapsed: HashSet::new(),
            rows: Rc::default(),
            selected_row: None,
            current_file: None,
            annotations: Rc::default(),
            diff,
            tree_focus: cx.focus_handle(),
            tree_scroll: UniformListScrollHandle::new(),
            composer: None,
            kept_unviewed: HashSet::new(),
            change_cursor: None,
            change_scrolled: false,
            cross_hint: None,
            _cross_hint_clear: Task::ready(()),
            shown_at: None,
            installed_whitespace: settings(cx).ignore_whitespace,
            tree_visible: true,
            tree_closing: false,
            tree_toggles: 0,
            tree_width: settings(cx)
                .tree_width
                .clamp(TREE_WIDTH_RANGE.0, TREE_WIDTH_RANGE.1),
            busy: None,
            _tree_close: Task::ready(()),
            branch_pickers,
            _load: Task::ready(()),
            _discussions: Task::ready(()),
            _subscriptions: vec![subscription, viewed_sync, settings_sync, activation],
        };
        this.load(cx);
        this
    }

    pub fn source(&self) -> &ReviewSource {
        &self.source
    }

    pub fn title(&self) -> SharedString {
        match (&self.source, &self.mr) {
            (_, Some(mr)) => format!("!{} {}", mr.mr.iid, mr.mr.title).into(),
            (source, None) => source.label(),
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        let order = self.tree.file_order();
        (
            order
                .iter()
                .filter(|&&f| self.viewed.get(f).copied().unwrap_or(false))
                .count(),
            order.len(),
        )
    }

    pub fn repo_root(&self) -> &std::path::Path {
        self.repo.root()
    }

    pub fn tree_node(&self, node: usize) -> Option<&review_core::Node> {
        self.tree.nodes().get(node)
    }

    pub fn hidden_files(&self) -> Rc<Vec<HiddenFile>> {
        self.hidden.clone()
    }

    fn apply_hide_rules(&mut self, cx: &mut Context<Self>) -> bool {
        let current = settings(cx);
        let patterns = (
            current.hidden_files.clone(),
            current.repo_hidden(self.repo.root()).to_vec(),
        );
        let Some(review) = self.review.clone() else {
            self.hide_patterns = patterns;
            return false;
        };
        let (rules, errors) = HideRules::new(&patterns.0, &patterns.1);
        for error in &errors {
            log::warn!(
                "hidden files pattern {:?}: {}",
                error.pattern,
                error.message
            );
        }
        self.hide_patterns = patterns;
        let hidden: Vec<HiddenFile> = if rules.is_empty() {
            Vec::new()
        } else {
            review
                .files
                .iter()
                .enumerate()
                .filter_map(|(ix, file)| {
                    let rule = rules.matching_rule(&file.path)?;
                    Some(HiddenFile {
                        ix,
                        path: file.path.clone(),
                        rule: rule.into(),
                    })
                })
                .collect()
        };
        let changed = hidden
            .iter()
            .map(|h| h.ix)
            .ne(self.hidden.iter().map(|h| h.ix));
        self.tree = if hidden.is_empty() {
            review.tree.clone()
        } else {
            let mut skip = vec![false; review.files.len()];
            for file in &hidden {
                skip[file.ix] = true;
            }
            Arc::new(FileTree::build_with(
                &model::tree_files(&review.files),
                |ix| !skip[ix],
            ))
        };
        self.hidden = Rc::new(hidden);
        changed
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let source = self.source.clone();
        let client = settings(cx).gitlab_client();
        let ignore_whitespace = settings(cx).ignore_whitespace;
        self.loaded_whitespace = ignore_whitespace;
        if self.review.is_none() {
            self.load_state = LoadState::Loading;
        }
        self._load = cx.spawn(async move |this, cx| {
            let result = load_review(repo, source, client, ignore_whitespace, cx).await;
            let _ = this.update(cx, |this, cx| this.install(result, ignore_whitespace, cx));
        });
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.busy = Some(Busy::Refreshing);
        self.load(cx);
    }

    fn install(
        &mut self,
        result: anyhow::Result<(LoadedReview, Option<MrContext>)>,
        whitespace: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy == Some(Busy::Refreshing) {
            self.busy = None;
        }
        match result {
            Ok((review, mr)) => {
                let same_shape = |old: &LoadedReview| {
                    self.installed_whitespace == whitespace
                        && old.context_limited == review.context_limited
                };
                let unchanged = self.mr.is_none()
                    && mr.is_none()
                    && self.review.as_deref().is_some_and(|old| {
                        same_shape(old)
                            && old.base_sha == review.base_sha
                            && old.head_sha == review.head_sha
                            && old.files.len() == review.files.len()
                            && old.files.iter().zip(&review.files).all(|(a, b)| {
                                a.path == b.path
                                    && a.fingerprint == b.fingerprint
                                    && a.status == b.status
                            })
                    });
                if unchanged {
                    cx.notify();
                    return;
                }
                let previous = self.current_file.and_then(|ix| {
                    let old = self.review.as_deref()?;
                    let file = &old.files[ix];
                    same_shape(old).then(|| (file.path.clone(), file.fingerprint.clone()))
                });
                let kept_path = self
                    .current_file
                    .and_then(|ix| self.review.as_ref().map(|r| r.files[ix].path.clone()));
                let store = self.viewed_store.read(cx).store();
                self.viewed = review
                    .files
                    .iter()
                    .map(|f| store.is_viewed(&f.path, &f.fingerprint))
                    .collect();
                let review = Rc::new(review);
                self.review = Some(review.clone());
                self.apply_hide_rules(cx);
                self.installed_whitespace = whitespace;
                self.mr = mr;
                self.load_state = LoadState::Ready;
                self.index_discussions();
                self.rebuild_rows();
                let same_file = previous.and_then(|(path, fingerprint)| {
                    let ix = review.file_index(&path)?;
                    (review.files[ix].fingerprint == fingerprint).then_some(ix)
                });
                if let Some(ix) = same_file {
                    self.current_file = Some(ix);
                    self.selected_row = self.rows.iter().position(|r| r.file == Some(ix));
                    self.rebuild_annotations();
                } else {
                    let target = kept_path
                        .and_then(|path| review.file_index(&path))
                        .or_else(|| self.first_unviewed())
                        .or_else(|| self.tree.file_order().first().copied());
                    self.current_file = None;
                    if let Some(ix) = target {
                        self.show_file(ix, cx);
                    } else {
                        self.diff.update(cx, |state, cx| state.set_files([], cx));
                    }
                }
                if self.mr.is_some() {
                    self.load_discussions(cx);
                }
            }
            Err(err) => {
                log::warn!("review load failed: {err:#}");
                let message: SharedString = format!("{err:#}").into();
                if self.review.is_some() {
                    self.notify(Notification::error(message), cx);
                } else {
                    self.load_state = LoadState::Failed(message);
                }
            }
        }
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    fn notify(&self, notification: Notification, cx: &mut Context<Self>) {
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.push_notification(notification, cx)
            });
        });
    }

    fn sync_viewed(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let store = self.viewed_store.read(cx).store();
        let viewed: Vec<bool> = review
            .files
            .iter()
            .map(|f| store.is_viewed(&f.path, &f.fingerprint))
            .collect();
        if viewed != self.viewed {
            self.viewed = viewed;
            self.rebuild_rows();
            cx.emit(ReviewEvent::Changed);
            cx.notify();
        }
    }

    fn diff_mode(&self, cx: &App) -> DiffMode {
        let whole_file = self
            .current_file
            .and_then(|f| self.review.as_ref()?.files.get(f))
            .is_some_and(|f| matches!(f.status, FileStatus::Added | FileStatus::Deleted));
        match settings(cx).diff_layout {
            DiffLayout::Split if !whole_file => DiffMode::Split,
            _ => DiffMode::Unified,
        }
    }

    fn sync_diff_mode(&mut self, cx: &mut Context<Self>) {
        let mode = self.diff_mode(cx);
        if self.diff.read(cx).mode() != mode {
            self.diff.update(cx, |state, cx| state.set_mode(mode, cx));
        }
    }

    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let whitespace = settings(cx).ignore_whitespace;
        self.sync_diff_mode(cx);
        let patterns_changed = settings(cx).hidden_files != self.hide_patterns.0
            || settings(cx).repo_hidden(self.repo.root()) != self.hide_patterns.1.as_slice();
        if patterns_changed && self.apply_hide_rules(cx) {
            self.rebuild_rows();
            let current_hidden = self
                .current_file
                .is_some_and(|f| self.tree.file_node(f).is_none());
            if current_hidden {
                let next = self
                    .first_unviewed()
                    .or_else(|| self.tree.file_order().first().copied());
                self.current_file = None;
                match next {
                    Some(ix) => self.show_file(ix, cx),
                    None => self.diff.update(cx, |state, cx| state.set_files([], cx)),
                }
            }
            cx.emit(ReviewEvent::Changed);
        }
        if whitespace != self.loaded_whitespace && self.review.is_some() {
            self.reload(cx);
        }
        cx.notify();
    }

    fn first_unviewed(&self) -> Option<usize> {
        self.review.as_ref()?;
        self.tree
            .file_order()
            .iter()
            .copied()
            .find(|&ix| !self.viewed[ix])
    }

    fn rebuild_rows(&mut self) {
        let Some(review) = self.review.as_ref() else {
            self.rows = Rc::default();
            return;
        };
        let progress = self.tree.progress(&self.viewed);
        let rows: Vec<TreeRow> = self
            .tree
            .rows(&self.collapsed)
            .into_iter()
            .map(|row| {
                let node = self.tree.node(row.node);
                let file = node.file();
                TreeRow {
                    node: row.node,
                    depth: row.depth,
                    is_dir: node.is_dir(),
                    expanded: row.expanded,
                    name: node.name.clone().into(),
                    file,
                    status: file.map_or(FileStatus::Modified, |f| review.files[f].status),
                    progress: progress[row.node],
                    comments: file
                        .and_then(|f| self.by_file.get(&review.files[f].path))
                        .map_or(0, Vec::len),
                }
            })
            .collect();
        if let Some(current) = self.current_file {
            self.selected_row = rows
                .iter()
                .position(|r| r.file == Some(current))
                .or(self.selected_row);
        }
        self.rows = Rc::new(rows);
    }

    pub fn show_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(file) = review.files.get(ix) else {
            return;
        };
        if self.current_file != Some(ix) {
            if let Some(left) = self.current_file
                && settings(cx).auto_mark_viewed
                && self
                    .shown_at
                    .is_some_and(|at| at.elapsed() >= AUTO_MARK_DWELL)
                && !self.viewed.get(left).copied().unwrap_or(true)
                && !review.files.get(left).is_some_and(|f| {
                    self.kept_unviewed
                        .contains(&(f.path.clone(), f.fingerprint.clone()))
                })
            {
                self.set_viewed(&[left], true, cx);
            }
            self.shown_at = Some(Instant::now());
            self.change_cursor = None;
            self.change_scrolled = false;
            self.cross_hint = None;
            self.current_file = Some(ix);
            let diff_file = file.diff.clone();
            self.sync_diff_mode(cx);
            self.diff
                .update(cx, |state, cx| state.set_files([diff_file], cx));
        }
        if let Some(node) = self.tree.file_node(ix) {
            let mut changed = false;
            for ancestor in self.tree.ancestors(node) {
                changed |= self.collapsed.remove(&ancestor.path);
            }
            if changed {
                self.rebuild_rows();
            }
        }
        self.selected_row = self.rows.iter().position(|r| r.file == Some(ix));
        if let Some(row) = self.selected_row {
            self.tree_scroll
                .scroll_to_item(row, gpui::ScrollStrategy::Center);
        }
        self.rebuild_annotations();
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(current) = self.current_file else {
            self.step_file(if forward { 1 } else { -1 }, cx);
            return;
        };
        let count = review.files[current].changes.len();
        let target = if forward {
            Some(self.change_cursor.map_or(0, |k| k + 1)).filter(|&k| k < count)
        } else if self.change_scrolled {
            Some(self.change_cursor.unwrap_or(0)).filter(|&k| k < count)
        } else {
            self.change_cursor.and_then(|k| k.checked_sub(1))
        };
        match target {
            Some(k) => self.scroll_to_change(current, k, cx),
            None => self.cross_file(forward, cx),
        }
    }

    fn cross_file(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(current) = self.current_file else {
            return;
        };
        let order = self.tree.file_order();
        let neighbour = order.iter().position(|&f| f == current).and_then(|pos| {
            if forward {
                order.get(pos + 1).copied()
            } else {
                pos.checked_sub(1).map(|p| order[p])
            }
        });
        let Some(next) = neighbour else {
            self.show_cross_hint(
                if forward {
                    CrossHint::NoNextFile
                } else {
                    CrossHint::NoPreviousFile
                },
                cx,
            );
            return;
        };
        let armed = if forward {
            CrossHint::PressForNextFile
        } else {
            CrossHint::PressForPreviousFile
        };
        if self.cross_hint != Some(armed) {
            self.show_cross_hint(armed, cx);
            return;
        }
        self.show_file(next, cx);
        let count = review.files[next].changes.len();
        if count > 0 {
            self.scroll_to_change(next, if forward { 0 } else { count - 1 }, cx);
        }
    }

    fn scroll_to_change(&mut self, file: usize, change: usize, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(target) = review.files.get(file).and_then(|f| f.changes.get(change)) else {
            return;
        };
        let (side, line) = match target.anchor {
            LineKind::Context { new, .. } | LineKind::Added { new } => (DiffSide::Modified, new),
            LineKind::Removed { old } => (DiffSide::Original, old),
        };
        let position = DiffLinePosition::new(review.files[file].path.clone(), side, line as usize);
        self.diff
            .update(cx, |state, cx| state.scroll_to_line(position, cx));
        self.change_cursor = Some(change);
        self.change_scrolled = false;
        self.cross_hint = None;
        cx.notify();
    }

    fn show_cross_hint(&mut self, hint: CrossHint, cx: &mut Context<Self>) {
        self.cross_hint = Some(hint);
        self._cross_hint_clear = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CROSS_HINT_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                if this.cross_hint == Some(hint) {
                    this.cross_hint = None;
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    pub fn note_manual_scroll(&mut self, cx: &mut Context<Self>) {
        self.change_scrolled = true;
        if self.cross_hint.take().is_some() {
            cx.notify();
        }
    }

    fn step_file(&mut self, delta: isize, cx: &mut Context<Self>) {
        let tree = self.tree.clone();
        let order = tree.file_order();
        if order.is_empty() {
            return;
        }
        let pos = self
            .current_file
            .and_then(|f| order.iter().position(|&x| x == f))
            .map_or(0, |p| p as isize + delta)
            .clamp(0, order.len() as isize - 1);
        self.show_file(order[pos as usize], cx);
    }

    pub fn set_viewed(&mut self, files: &[usize], viewed: bool, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let mut marks = Vec::with_capacity(files.len());
        for &ix in files {
            if let Some(file) = review.files.get(ix) {
                self.viewed[ix] = viewed;
                let key = (file.path.clone(), file.fingerprint.clone());
                if viewed {
                    self.kept_unviewed.remove(&key);
                } else {
                    self.kept_unviewed.insert(key);
                }
                marks.push((file.path.as_ref(), file.fingerprint.as_str()));
            }
        }
        self.viewed_store
            .update(cx, |store, cx| store.set(&marks, viewed, cx));
        self.rebuild_rows();
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    pub fn toggle_file_viewed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(viewed) = self.viewed.get(ix).copied() else {
            return;
        };
        self.set_viewed(&[ix], !viewed, cx);
        if !viewed && self.current_file == Some(ix) && settings(cx).advance_on_viewed {
            let next = self.next_unviewed_after(ix);
            if let Some(next) = next {
                self.show_file(next, cx);
            }
        }
    }

    fn next_unviewed_after(&self, ix: usize) -> Option<usize> {
        let order = self.tree.file_order();
        let pos = order.iter().position(|&x| x == ix)?;
        order[pos + 1..]
            .iter()
            .chain(order[..pos].iter())
            .copied()
            .find(|&f| !self.viewed[f])
    }

    pub fn toggle_row_viewed(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(row).cloned() else {
            return;
        };
        if let Some(file) = row.file {
            self.toggle_file_viewed(file, cx);
        } else if self.review.is_some() {
            let files = self.tree.files_under(row.node);
            let viewed = !row.progress.is_complete();
            self.set_viewed(&files, viewed, cx);
        }
    }

    pub fn toggle_dir(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(row).cloned() else {
            return;
        };
        if !row.is_dir {
            return;
        }
        let path = self.tree.node(row.node).path.clone();
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.rebuild_rows();
        self.selected_row = self.rows.iter().position(|r| r.node == row.node);
        cx.notify();
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.collapsed = self.tree.dir_paths().map(str::to_string).collect();
        self.rebuild_rows();
        self.selected_row = self
            .current_file
            .and_then(|f| self.rows.iter().position(|r| r.file == Some(f)));
        cx.notify();
    }

    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.collapsed.clear();
        self.rebuild_rows();
        if let Some(current) = self.current_file {
            self.selected_row = self.rows.iter().position(|r| r.file == Some(current));
        }
        cx.notify();
    }

    pub fn click_row(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected_row = Some(row);
        match self.rows.get(row).map(|r| (r.is_dir, r.file)) {
            Some((true, _)) => self.toggle_dir(row, cx),
            Some((false, Some(file))) => self.show_file(file, cx),
            _ => {}
        }
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let row = self
            .selected_row
            .map_or(0, |r| r as isize + delta)
            .clamp(0, self.rows.len() as isize - 1) as usize;
        self.selected_row = Some(row);
        self.tree_scroll
            .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
        if let Some(file) = self.rows[row].file {
            self.show_file(file, cx);
        }
        cx.notify();
    }

    pub fn set_layout(&mut self, layout: DiffLayout, cx: &mut Context<Self>) {
        update_settings(cx, |s| s.diff_layout = layout);
        self.sync_diff_mode(cx);
    }

    pub fn pull(&mut self, side: PullSide, window: &mut Window, cx: &mut Context<Self>) {
        let ReviewSource::Branches { target, source } = &self.source else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        let branch = match side {
            PullSide::Target => target.clone(),
            PullSide::Source => source.clone(),
        };
        let repo = self.repo.clone();
        self.busy = Some(Busy::Pulling(side));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let pulled = branch.clone();
            let result = net_runtime::spawn(async move { repo.pull(&pulled).await }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                let s = tr();
                match result {
                    Ok(PullOutcome::UpToDate) => {
                        window
                            .push_notification(Notification::info(s.pull_up_to_date(&branch)), cx);
                    }
                    Ok(PullOutcome::Updated { .. }) => {
                        window
                            .push_notification(Notification::success(s.pull_updated(&branch)), cx);
                        this.reload(cx);
                    }
                    Err(err) => {
                        log::warn!("pull {branch}: {err}");
                        window.push_notification(
                            Notification::error(err.to_string()).title(s.pull_failed(&branch)),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.reload(cx);
    }

    fn load_discussions(&mut self, cx: &mut Context<Self>) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        self.discussions_loading = true;
        self._discussions = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client.discussions(&ctx.project, ctx.mr.iid).await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.discussions_loading = false;
                match result {
                    Ok(discussions) => {
                        this.discussions = Rc::new(
                            discussions
                                .iter()
                                .filter_map(DiscussionView::from_gitlab)
                                .collect(),
                        );
                        this.index_discussions();
                        this.rebuild_rows();
                        this.rebuild_annotations();
                    }
                    Err(err) => {
                        log::warn!("discussions: {err}");
                        this.notify(
                            Notification::error(err.to_string())
                                .title(tr().comments_failed.clone()),
                            cx,
                        );
                    }
                }
                cx.emit(ReviewEvent::Changed);
                cx.notify();
            });
        });
    }

    fn index_discussions(&mut self) {
        self.by_file.clear();
        self.general.clear();
        let (Some(review), Some(ctx)) = (self.review.as_ref(), self.mr.as_ref()) else {
            return;
        };
        for (ix, discussion) in self.discussions.iter().enumerate() {
            let placed = discussion.position.as_ref().and_then(|pos| {
                if pos.head_sha.as_deref() != Some(ctx.refs.head_sha.as_str())
                    || (pos.new_line.is_none() && pos.old_line.is_none())
                {
                    return None;
                }
                let path = pos.new_path.as_deref().or(pos.old_path.as_deref())?;
                review
                    .files
                    .iter()
                    .find(|f| f.new_path.as_ref() == path || f.old_path.as_ref() == path)
                    .map(|f| f.path.clone())
            });
            match placed {
                Some(path) if review.comments_aligned => {
                    self.by_file.entry(path).or_default().push(ix)
                }
                _ => self.general.push(ix),
            }
        }
    }

    fn rebuild_annotations(&mut self) {
        let mut annotations = Vec::new();
        if let (Some(review), Some(file)) = (self.review.as_ref(), self.current_file) {
            let path = review.files[file].path.clone();
            if let Some(list) = self.by_file.get(&path) {
                for &ix in list {
                    let Some(pos) = self.discussions[ix].position.as_ref() else {
                        continue;
                    };
                    let position = match (pos.new_line, pos.old_line) {
                        (Some(line), _) => {
                            DiffLinePosition::new(path.clone(), DiffSide::Modified, line as usize)
                        }
                        (None, Some(line)) => {
                            DiffLinePosition::new(path.clone(), DiffSide::Original, line as usize)
                        }
                        (None, None) => continue,
                    };
                    annotations.push(DiffAnnotation::line(
                        ElementId::from(("discussion", ix)),
                        position,
                    ));
                }
            }
            if let Some(Composer {
                target: ComposerTarget::Line(position),
                ..
            }) = &self.composer
                && position.path() == &path
            {
                annotations.push(DiffAnnotation::line(
                    ElementId::from(("draft", 0usize)),
                    position.clone(),
                ));
            }
        }
        self.annotations = Rc::new(annotations);
    }

    pub fn open_line_composer(
        &mut self,
        range: &DiffLineRange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mr.is_none() {
            return;
        }
        let side = range.end_side();
        let position = DiffLinePosition::new(range.path().clone(), side, range.end());
        self.open_composer(ComposerTarget::Line(position), window, cx);
    }

    pub fn open_composer(
        &mut self,
        target: ComposerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = tr().comment_placeholder.clone();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 16)
                .placeholder(placeholder)
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        let submit = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                this.submit_composer(window, cx);
            }
        });
        self.composer = Some(Composer {
            target,
            input,
            _submit: submit,
        });
        self.rebuild_annotations();
        cx.notify();
    }

    pub fn cancel_composer(&mut self, cx: &mut Context<Self>) {
        self.composer = None;
        self.rebuild_annotations();
        cx.notify();
    }

    pub fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(composer), Some(ctx), Some(review)) =
            (self.composer.as_ref(), self.mr.clone(), self.review.clone())
        else {
            return;
        };
        let body = composer.input.read(cx).value().trim().to_string();
        let composer_id: EntityId = composer.input.entity_id();
        if body.is_empty() || self.busy == Some(Busy::Posting) {
            return;
        }
        let request = match &composer.target {
            ComposerTarget::Reply(id) => PostRequest::Reply(id.clone()),
            ComposerTarget::General => PostRequest::New(None),
            ComposerTarget::Line(position) => {
                let Some(file) = review.file_index(position.path()) else {
                    return;
                };
                let file = &review.files[file];
                let side = match position.side() {
                    DiffSide::Original => Side::Old,
                    _ => Side::New,
                };
                let Some(kind) = file.line_map.locate(side, position.line() as u32) else {
                    window.push_notification(
                        Notification::error(tr().comment_line_unavailable.clone()),
                        cx,
                    );
                    return;
                };
                PostRequest::New(Some(gitlab::NewPosition {
                    base_sha: ctx.refs.base_sha.clone(),
                    start_sha: ctx.refs.start_sha.clone(),
                    head_sha: ctx.refs.head_sha.clone(),
                    old_path: file.old_path.to_string(),
                    new_path: file.new_path.to_string(),
                    old_line: kind.old_line(),
                    new_line: kind.new_line(),
                }))
            }
        };
        self.busy = Some(Busy::Posting);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                match request {
                    PostRequest::Reply(id) => ctx
                        .client
                        .reply(&ctx.project, ctx.mr.iid, &id, &body)
                        .await
                        .map(|_| ()),
                    PostRequest::New(position) => ctx
                        .client
                        .create_discussion(&ctx.project, ctx.mr.iid, &body, position.as_ref())
                        .await
                        .map(|_| ()),
                }
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        if this
                            .composer
                            .as_ref()
                            .is_some_and(|c| c.input.entity_id() == composer_id)
                        {
                            this.composer = None;
                        }
                        this.load_discussions(cx);
                    }
                    Err(err) => {
                        log::warn!("posting a comment: {err}");
                        window.push_notification(
                            Notification::error(err.to_string()).title(tr().comment_failed.clone()),
                            cx,
                        );
                    }
                }
                this.rebuild_annotations();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_resolved(
        &mut self,
        discussion: String,
        resolved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .set_resolved(&ctx.project, ctx.mr.iid, &discussion, resolved)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(_) => this.load_discussions(cx),
                Err(err) => window.push_notification(
                    Notification::error(err.to_string()).title(tr().comment_failed.clone()),
                    cx,
                ),
            });
        })
        .detach();
    }

    pub fn general_discussions(&self) -> (Rc<Vec<DiscussionView>>, &[usize]) {
        (self.discussions.clone(), &self.general)
    }

    pub fn discussions(&self) -> Rc<Vec<DiscussionView>> {
        self.discussions.clone()
    }

    pub fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree_visible && self.tree_focus.contains_focused(window, cx) {
            self.focus_diff(window, cx);
        }
        self.tree_visible = !self.tree_visible;
        self.tree_toggles += 1;
        self.tree_closing = !self.tree_visible;
        self._tree_close = if self.tree_closing {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TREE_SLIDE).await;
                let _ = this.update(cx, |this, cx| {
                    this.tree_closing = false;
                    cx.notify();
                });
            })
        } else {
            Task::ready(())
        };
        cx.notify();
    }

    pub fn set_auto_mark(&mut self, on: bool, cx: &mut Context<Self>) {
        update_settings(cx, |s| s.auto_mark_viewed = on);
        cx.notify();
    }

    pub fn resize_tree(&mut self, width: f32, cx: &mut Context<Self>) {
        let (min, max) = TREE_WIDTH_RANGE;
        let width = width.clamp(min, max).round();
        if (width - self.tree_width).abs() < 0.5 {
            return;
        }
        self.tree_width = width;
        let generation = TREE_WIDTH_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(TREE_WIDTH_SAVE_DELAY).await;
            if TREE_WIDTH_GENERATION.load(Ordering::Relaxed) == generation {
                cx.update(|cx| update_settings(cx, |s| s.tree_width = width));
            }
        })
        .detach();
        cx.notify();
    }

    pub fn general_count(&self) -> usize {
        self.general.len()
    }

    pub fn has_mr(&self) -> bool {
        self.mr.is_some()
    }

    pub fn focus_tree(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.tree_focus, cx);
    }

    fn focus_diff(&self, window: &mut Window, cx: &mut App) {
        let handle = self.diff.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

enum PostRequest {
    Reply(String),
    New(Option<gitlab::NewPosition>),
}

impl Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.tree_focus.clone()
    }
}

async fn load_review(
    repo: Repo,
    source: ReviewSource,
    client: Option<gitlab::Client>,
    ignore_whitespace: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<(LoadedReview, Option<MrContext>)> {
    match source {
        ReviewSource::Branches { target, source } => {
            let review = cx
                .background_spawn(async move {
                    let comparison = repo.compare(&target, &source)?;
                    model::build(&repo, comparison, ignore_whitespace)
                })
                .await?;
            Ok((review, None))
        }
        ReviewSource::WorkingTree => {
            let review = cx
                .background_spawn(
                    async move { model::build_working_tree(&repo, ignore_whitespace) },
                )
                .await?;
            Ok((review, None))
        }
        ReviewSource::MergeRequest { project, iid } => {
            let client = client.ok_or_else(|| anyhow!(tr().gitlab_not_configured.to_string()))?;
            let project_ref = gitlab::ProjectRef::Path(project.clone());
            let mr = {
                let client = client.clone();
                let project_ref = project_ref.clone();
                net_runtime::spawn(async move { client.merge_request(&project_ref, iid).await })
                    .await?
            };
            let refs = mr
                .diff_refs
                .clone()
                .ok_or_else(|| anyhow!(tr().mr_not_ready.to_string()))?;
            let missing_remote = {
                let repo = repo.clone();
                let refs = refs.clone();
                let base_url = client.base_url().to_string();
                let project_path = project.clone();
                cx.background_spawn(async move {
                    if repo.has_commit(&refs.head_sha) && repo.has_commit(&refs.base_sha) {
                        Ok(None)
                    } else {
                        model::gitlab_remote(&repo, &base_url, Some(&project_path))
                            .map(|(remote, _)| remote)
                            .or_else(|| repo.upstream_remote())
                            .map(Some)
                            .ok_or_else(|| anyhow!(tr().no_remote_for_mr.to_string()))
                    }
                })
                .await?
            };
            if let Some(remote) = missing_remote {
                let specs = model::mr_refspecs(&mr);
                let fetch_repo = repo.clone();
                net_runtime::spawn(async move { fetch_repo.fetch(&remote, &specs).await }).await?;
            }
            let comparison = model::comparison_for_mr(&mr, &refs);
            let review = {
                let repo = repo.clone();
                cx.background_spawn(
                    async move { model::build(&repo, comparison, ignore_whitespace) },
                )
                .await?
            };
            Ok((
                review,
                Some(MrContext {
                    client,
                    project: project_ref,
                    project_path: project,
                    mr,
                    refs,
                }),
            ))
        }
    }
}
