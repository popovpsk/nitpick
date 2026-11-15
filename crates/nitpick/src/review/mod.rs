pub(crate) mod avatars;
mod blame;
mod branch_picker;
mod comment_list;
mod comments;
mod commit_picker;
mod diff_panel;
mod file_actions;
mod file_search;
mod find_bar;
mod image_preview;
pub mod model;
mod note_actions;
mod since_viewed;
mod tree_panel;
mod viewed;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use forge::ForgeKind;
use git_repo::{PullOutcome, Repo};
use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, ElementId, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, Pixels, SharedString, Subscription, Task,
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
use review_core::{FileStatus, FileTree, HideRules, LineKind, Progress, Side, TreeShape};

use crate::localization::tr;
use crate::settings_store::{
    DiffLayout, SPLIT_RATIO_RANGE, SettingsStore, TREE_WIDTH_RANGE, TreeMode, settings,
    update_settings,
};
pub use avatars::init as init_avatars;
pub use blame::ToggleBlame;
pub use file_actions::{CopyPath, HostOs, OpenInEditor, editor_presets};
pub use model::{LoadedReview, MrContext, ReviewSource};
use viewed::SharedViewed;

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
        FindFile,
        FindInFile,
    ]
);

const TREE_SLIDE: Duration = Duration::from_millis(200);
const TREE_WIDTH_SAVE_DELAY: Duration = Duration::from_millis(400);
const SPLIT_RATIO_SAVE_DELAY: Duration = Duration::from_millis(400);
const AUTO_MARK_DWELL: Duration = Duration::from_millis(500);
const CROSS_HINT_TIMEOUT: Duration = Duration::from_millis(3000);

static TREE_WIDTH_GENERATION: AtomicU64 = AtomicU64::new(0);
static SPLIT_RATIO_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct TreeRow {
    pub node: usize,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub name: SharedString,
    pub path: SharedString,
    pub detail: Option<SharedString>,
    pub muted: bool,
    pub file: Option<usize>,
    pub status: FileStatus,
    pub progress: Progress,
    pub comments: usize,
    pub seen_before: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CrossHint {
    PressForNextFile,
    PressForPreviousFile,
    NoNextFile,
    NoPreviousFile,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct TreeView {
    shape: TreeShape,
    show_hidden: bool,
}

#[derive(Clone)]
pub struct HiddenFile {
    pub ix: usize,
    pub path: SharedString,
    pub rule: SharedString,
    pub matched: Vec<std::ops::Range<usize>>,
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
    pub avatar: Option<SharedString>,
    pub initials: SharedString,
    pub tint: usize,
    pub long: bool,
    pub own: bool,
    pub meta: SharedString,
    pub body: SharedString,
}

#[derive(Clone)]
pub struct DiscussionView {
    pub id: String,
    pub notes: Vec<NoteView>,
    pub can_reply: bool,
    pub resolve_id: Option<String>,
    pub resolved: bool,
    pub position: Option<forge::Position>,
    pub location: Option<SharedString>,
}

const LONG_NOTE_LINES: usize = 10;
const WRAP_CHARS: usize = 90;

fn is_long(body: &str) -> bool {
    body.lines()
        .map(|line| line.chars().count().div_ceil(WRAP_CHARS).max(1))
        .sum::<usize>()
        > LONG_NOTE_LINES
}

impl DiscussionView {
    fn from_forge(discussion: &forge::Discussion, me: Option<&str>) -> Option<Self> {
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
                    avatar: note.author.avatar_url.clone().map(SharedString::from),
                    initials: review_core::blame::initials(&note.author.name).into(),
                    tint: review_core::blame::tint_index(&note.author.username, blame::TINT_COUNT),
                    long: is_long(&note.body),
                    own: me.is_some_and(|me| me == note.author.username),
                    meta: format!("@{} · {time}", note.author.username).into(),
                    body: note.body.clone().into(),
                }
            })
            .collect();
        if notes.is_empty() {
            return None;
        }
        let position = discussion.position.clone();
        let location = position
            .as_ref()
            .and_then(|p| p.new_path.clone().or_else(|| p.old_path.clone()))
            .map(SharedString::from);
        Some(Self {
            id: discussion.id.clone(),
            notes,
            can_reply: discussion.can_reply,
            resolve_id: discussion.resolve_id.clone(),
            resolved: discussion.resolved,
            position,
            location,
        })
    }
}

pub enum ReviewEvent {
    Changed,
    Open(ReviewSource),
    OpenSettings,
    EditHiddenFiles,
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
    general_only: Rc<Vec<usize>>,
    code_threads: Rc<Vec<comment_list::CodeThread>>,
    comments_tab: comment_list::CommentsTab,
    comment_tab_labels: (SharedString, SharedString),
    expanded_notes: HashSet<u64>,
    open_thread: Option<usize>,
    me: Option<String>,
    editing: Option<note_actions::NoteEdit>,
    note_busy: bool,
    approval: Option<note_actions::ApprovalView>,
    approval_busy: bool,
    _approval: Task<()>,
    discussions_loading: bool,
    viewed: Vec<bool>,
    tree: Arc<FileTree>,
    hidden: Rc<Vec<HiddenFile>>,
    hidden_mask: Vec<bool>,
    nav_order: Vec<usize>,
    nav_generation: u64,
    tree_view: TreeView,
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
    split_ratio: f32,
    split_width: f32,
    split_drag: Option<(Pixels, f32)>,
    file_label: Option<diff_panel::FileLabel>,
    header_text: Option<diff_panel::HeaderText>,
    file_search: Option<file_search::FileSearch>,
    find: Option<find_bar::FindBar>,
    seen_before: Vec<Option<String>>,
    since_mode: bool,
    interdiff: Option<model::Interdiff>,
    interdiff_loading: Option<String>,
    blob_known: HashMap<String, bool>,
    blob_checking: bool,
    _blob_check: Task<()>,
    pending_jump: Option<bool>,
    _interdiff: Task<()>,
    search_opens: usize,
    busy: Option<Busy>,
    _tree_close: Task<()>,
    branch_pickers: branch_picker::BranchPickers,
    commit: Option<String>,
    commit_picker: commit_picker::CommitPicker,
    blame: blame::BlameState,
    image_preview: image_preview::ImagePreview,
    web: file_actions::WebLink,
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
        let context = context_lines(cx);
        let diff = cx.new(|cx| {
            DiffState::new([], cx)
                .with_mode(mode)
                .with_context_lines(context)
        });
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
        let commit_picker = commit_picker::CommitPicker::new(window, cx);
        let mut this = Self {
            repo,
            source,
            load_state: LoadState::Loading,
            review: None,
            mr: None,
            discussions: Rc::default(),
            by_file: HashMap::new(),
            general: Vec::new(),
            general_only: Rc::default(),
            code_threads: Rc::default(),
            comments_tab: comment_list::CommentsTab::default(),
            comment_tab_labels: (SharedString::default(), SharedString::default()),
            expanded_notes: HashSet::new(),
            open_thread: None,
            me: None,
            editing: None,
            note_busy: false,
            approval: None,
            approval_busy: false,
            _approval: Task::ready(()),
            discussions_loading: false,
            viewed: Vec::new(),
            tree: Arc::default(),
            hidden: Rc::default(),
            hidden_mask: Vec::new(),
            nav_order: Vec::new(),
            nav_generation: 0,
            tree_view: TreeView::default(),
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
            split_ratio: settings(cx)
                .split_ratio
                .clamp(SPLIT_RATIO_RANGE.0, SPLIT_RATIO_RANGE.1),
            split_width: 0.,
            split_drag: None,
            file_label: None,
            header_text: None,
            file_search: None,
            find: None,
            seen_before: Vec::new(),
            since_mode: false,
            interdiff: None,
            interdiff_loading: None,
            blob_known: HashMap::new(),
            blob_checking: false,
            _blob_check: Task::ready(()),
            pending_jump: None,
            _interdiff: Task::ready(()),
            search_opens: 0,
            busy: None,
            _tree_close: Task::ready(()),
            branch_pickers,
            commit: None,
            commit_picker,
            blame: blame::BlameState::new(cx),
            image_preview: image_preview::ImagePreview::new(cx),
            web: file_actions::WebLink::default(),
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
            (_, Some(mr)) => format!("{} {}", mr.request.reference(), mr.request.title).into(),
            (source, None) => source.label(),
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        let order = &self.nav_order;
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
                        matched: review_core::hidden::matched_ranges(&rule, &file.path),
                        rule: rule.into(),
                    })
                })
                .collect()
        };
        let changed = hidden
            .iter()
            .map(|h| h.ix)
            .ne(self.hidden.iter().map(|h| h.ix));
        let mut mask = vec![false; review.files.len()];
        for file in &hidden {
            mask[file.ix] = true;
        }
        self.hidden_mask = mask;
        self.hidden = Rc::new(hidden);
        self.rebuild_tree(cx);
        changed
    }

    fn tree_view(cx: &App) -> TreeView {
        let current = settings(cx);
        TreeView {
            shape: TreeShape {
                flat: current.tree_mode == TreeMode::List,
                compact: current.compact_folders,
            },
            show_hidden: current.show_hidden_files,
        }
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let view = Self::tree_view(cx);
        self.tree_view = view;
        let mask = &self.hidden_mask;
        let no_hidden = !mask.iter().any(|h| *h);
        self.tree = if view.shape == TreeShape::default() && (no_hidden || view.show_hidden) {
            review.tree.clone()
        } else {
            Arc::new(FileTree::build_shaped(
                &model::tree_files(&review.files),
                |ix| view.show_hidden || !mask.get(ix).copied().unwrap_or(false),
                view.shape,
            ))
        };
        self.nav_generation += 1;
        self.nav_order = self
            .tree
            .file_order()
            .iter()
            .copied()
            .filter(|&f| self.is_tracked(f))
            .collect();
        self.rank_file_search(cx);
    }

    pub fn is_tracked(&self, file: usize) -> bool {
        !self.hidden_mask.get(file).copied().unwrap_or(false)
    }

    pub fn nav_len(&self) -> usize {
        self.nav_order.len()
    }

    pub fn nav_position(&self, file: usize) -> Option<usize> {
        self.nav_order.iter().position(|&f| f == file)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let source = self.source.clone();
        let commit = self.commit.clone();
        let client = match &source {
            ReviewSource::MergeRequest { forge, .. } => settings(cx).forge_client(*forge),
            _ => None,
        };
        let ignore_whitespace = settings(cx).ignore_whitespace;
        self.loaded_whitespace = ignore_whitespace;
        if self.review.is_none() {
            self.load_state = LoadState::Loading;
        }
        self._load = cx.spawn(async move |this, cx| {
            let result = load_review(repo, source, commit, client, ignore_whitespace, cx).await;
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
                let switched = self
                    .review
                    .as_deref()
                    .is_some_and(|old| old.commit_sha() != review.commit_sha());
                let unchanged = self.mr.is_none()
                    && mr.is_none()
                    && !switched
                    && self.review.as_deref().is_some_and(|old| {
                        same_shape(old)
                            && old.base_sha == review.base_sha
                            && old.head_sha == review.head_sha
                            && old.merge_base_sha == review.merge_base_sha
                            && old.commits == review.commits
                            && old.files.len() == review.files.len()
                            && old.files.iter().zip(&review.files).all(|(a, b)| {
                                a.path == b.path
                                    && a.fingerprint == b.fingerprint
                                    && a.status == b.status
                            })
                    });
                if unchanged {
                    self.retry_failed_blame(cx);
                    cx.notify();
                    return;
                }
                let previous = self.current_file.filter(|_| !switched).and_then(|ix| {
                    let old = self.review.as_deref()?;
                    let file = &old.files[ix];
                    same_shape(old).then(|| (file.path.clone(), file.fingerprint.clone()))
                });
                let kept_path = self
                    .current_file
                    .filter(|_| !switched)
                    .and_then(|ix| self.review.as_ref().map(|r| r.files[ix].path.clone()));
                let store = self.viewed_store.read(cx).store();
                self.viewed = review
                    .files
                    .iter()
                    .map(|f| store.is_viewed(&f.path, &f.fingerprint))
                    .collect();
                self.commit = review.commit_sha().map(str::to_string);
                self.commit_picker.set_review(&review);
                let review = Rc::new(review);
                self.review = Some(review.clone());
                self.refresh_seen_before(cx);
                self.apply_hide_rules(cx);
                self.installed_whitespace = whitespace;
                self.mr = mr;
                self.refresh_web(cx);
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
                    if let Some(interdiff) = self.interdiff.as_mut() {
                        interdiff.file = ix;
                    }
                    self.sync_since_viewed(cx);
                    self.rebuild_annotations();
                } else {
                    let target = kept_path
                        .and_then(|path| review.file_index(&path))
                        .or_else(|| self.first_unviewed())
                        .or_else(|| self.nav_order.first().copied());
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
                self.request_blame(cx);
                self.sync_find(cx);
            }
            Err(err) => {
                log::warn!("review load failed: {err:#}");
                let message: SharedString = format!("{err:#}").into();
                self.commit = self
                    .review
                    .as_deref()
                    .and_then(LoadedReview::commit_sha)
                    .map(str::to_string);
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
        let seen_changed = self.refresh_seen_before(cx);
        if viewed != self.viewed || seen_changed {
            self.viewed = viewed;
            self.rebuild_rows();
            self.sync_since_viewed(cx);
            cx.emit(ReviewEvent::Changed);
            cx.notify();
        }
    }

    fn diff_mode(&self, cx: &App) -> DiffMode {
        let whole_file = self
            .current_file
            .and_then(|f| self.review.as_ref()?.files.get(f))
            .is_some_and(|f| matches!(f.status, FileStatus::Added | FileStatus::Deleted))
            && self
                .current_file
                .and_then(|f| self.interdiff_key(f))
                .is_none();
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
        let context = context_lines(cx);
        if self.diff.read(cx).context_lines() != context {
            self.diff
                .update(cx, |state, cx| state.set_context_lines(context, cx));
            self.relayout_blame(cx);
            if let Some(find) = self.find.as_mut() {
                find.forget_text();
            }
            self.sync_find(cx);
        }
        self.refresh_find_view(cx);
        let patterns_changed = settings(cx).hidden_files != self.hide_patterns.0
            || settings(cx).repo_hidden(self.repo.root()) != self.hide_patterns.1.as_slice();
        if !patterns_changed && Self::tree_view(cx) != self.tree_view {
            self.rebuild_tree(cx);
            self.rebuild_rows();
            if let Some(row) = self.selected_row {
                self.tree_scroll
                    .scroll_to_item(row, gpui::ScrollStrategy::Center);
            }
            cx.emit(ReviewEvent::Changed);
        }
        if patterns_changed && self.apply_hide_rules(cx) {
            self.rebuild_rows();
            let current_hidden = self.current_file.is_some_and(|f| !self.is_tracked(f));
            if current_hidden || self.current_file.is_none() {
                let next = self
                    .current_file
                    .and_then(|f| self.next_unviewed_after(f))
                    .or_else(|| {
                        self.current_file
                            .and_then(|f| self.visible_neighbour(f, true))
                    })
                    .or_else(|| self.first_unviewed())
                    .or_else(|| self.nav_order.first().copied());
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
        self.nav_order.iter().copied().find(|&ix| !self.viewed[ix])
    }

    fn rebuild_rows(&mut self) {
        let Some(review) = self.review.as_ref() else {
            self.rows = Rc::default();
            return;
        };
        let mask = &self.hidden_mask;
        let progress = self
            .tree
            .progress_tracked(&self.viewed, |f| !mask.get(f).copied().unwrap_or(false));
        let flat = self.tree_view.shape.flat;
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
                    path: node.path.clone().into(),
                    detail: flat
                        .then(|| node.path.rsplit_once('/').map(|(dir, _)| dir))
                        .flatten()
                        .map(|dir| SharedString::from(dir.to_string())),
                    muted: match file {
                        Some(f) => mask.get(f).copied().unwrap_or(false),
                        None => progress[row.node].total == 0,
                    },
                    file,
                    status: file.map_or(FileStatus::Modified, |f| review.files[f].status),
                    progress: match file {
                        Some(f) if mask.get(f).copied().unwrap_or(false) => Progress {
                            viewed: usize::from(self.viewed.get(f).copied().unwrap_or(false)),
                            total: 1,
                        },
                        _ => progress[row.node],
                    },
                    comments: match file {
                        Some(f) => self.by_file.get(&review.files[f].path).map_or(0, Vec::len),
                        None if !row.expanded && !self.by_file.is_empty() => self
                            .tree
                            .files_under(row.node)
                            .iter()
                            .filter_map(|&f| self.by_file.get(&review.files[f].path))
                            .map(Vec::len)
                            .sum(),
                        None => 0,
                    },
                    seen_before: file.is_some_and(|f| {
                        !self.viewed.get(f).copied().unwrap_or(false) && self.seen_before(f)
                    }),
                }
            })
            .collect();
        let selected_path = self
            .selected_row
            .and_then(|r| self.rows.get(r))
            .map(|r| r.path.clone());
        self.selected_row = self
            .current_file
            .and_then(|current| rows.iter().position(|r| r.file == Some(current)))
            .or_else(|| {
                let path = selected_path?;
                rows.iter().position(|r| r.path == path)
            });
        self.rows = Rc::new(rows);
    }

    pub fn show_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        if ix >= review.files.len() {
            return;
        }
        if self.current_file != Some(ix) {
            if let Some(left) = self.current_file
                && settings(cx).auto_mark_viewed
            {
                let dwelled = self
                    .shown_at
                    .is_some_and(|at| at.elapsed() >= AUTO_MARK_DWELL);
                let tracked = self.is_tracked(left);
                let unviewed = !self.viewed.get(left).copied().unwrap_or(true);
                let kept = review.files.get(left).is_some_and(|f| {
                    self.kept_unviewed
                        .contains(&(f.path.clone(), f.fingerprint.clone()))
                });
                log::debug!(
                    "auto-mark {left}: dwelled {dwelled}, tracked {tracked}, unviewed {unviewed}, kept unviewed {kept}"
                );
                if dwelled && tracked && unviewed && !kept {
                    self.set_viewed(&[left], true, cx);
                }
            }
            self.shown_at = Some(Instant::now());
            self.change_cursor = None;
            self.change_scrolled = false;
            self.cross_hint = None;
            self.current_file = Some(ix);
            self.pending_jump = None;
            self.install_content(ix, cx);
            if settings(cx).show_whole_file {
                if self.content_pending() {
                    self.pending_jump = Some(true);
                } else if !self.shown_changes(ix).is_empty() {
                    self.scroll_to_change(ix, 0, cx);
                }
            }
            self.request_blame(cx);
            self.sync_find(cx);
            self.request_image_preview(cx);
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
        if self.review.is_none() {
            return;
        }
        let Some(current) = self.current_file else {
            self.step_file(if forward { 1 } else { -1 }, cx);
            return;
        };
        if self.content_pending() {
            return;
        }
        let count = self.shown_changes(current).len();
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
        if self.review.is_none() {
            return;
        }
        let Some(current) = self.current_file else {
            return;
        };
        let Some(next) = self.visible_neighbour(current, forward) else {
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
        if self.content_pending() {
            self.pending_jump = Some(forward);
            return;
        }
        let count = self.shown_changes(next).len();
        if count > 0 {
            self.scroll_to_change(next, if forward { 0 } else { count - 1 }, cx);
        }
    }

    fn scroll_to_change(&mut self, file: usize, change: usize, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(target) = self.shown_changes(file).get(change).copied() else {
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
        let order = &self.nav_order;
        let target = match self.current_file {
            Some(current) => self.visible_neighbour(current, delta > 0),
            None if delta > 0 => order.first().copied(),
            None => order.last().copied(),
        };
        if let Some(ix) = target {
            self.show_file(ix, cx);
        }
    }

    fn visible_neighbour(&self, from: usize, forward: bool) -> Option<usize> {
        let order = &self.nav_order;
        if let Some(pos) = order.iter().position(|&f| f == from) {
            return if forward {
                order.get(pos + 1).copied()
            } else {
                pos.checked_sub(1).map(|p| order[p])
            };
        }
        let review = self.review.as_ref()?;
        let sorted;
        let all: &[usize] = if self.tree.file_node(from).is_some() {
            self.tree.file_order()
        } else if self.tree_view.shape.flat {
            sorted = {
                let mut ixs: Vec<usize> = (0..review.files.len()).collect();
                ixs.sort_by(|&a, &b| {
                    let (a, b) = (&review.files[a].path, &review.files[b].path);
                    (a.to_lowercase(), a).cmp(&(b.to_lowercase(), b))
                });
                ixs
            };
            &sorted
        } else {
            review.tree.file_order()
        };
        let pos = all.iter().position(|&f| f == from)?;
        let visible = |f: &&usize| self.is_tracked(**f) && self.tree.file_node(**f).is_some();
        if forward {
            all[pos + 1..].iter().find(visible).copied()
        } else {
            all[..pos].iter().rev().find(visible).copied()
        }
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
        let selected = self
            .selected_row
            .and_then(|row| self.rows.get(row))
            .map(|row| row.node);
        let folded =
            viewed && settings(cx).collapse_viewed_folders && self.collapse_finished_folders(files);
        self.rebuild_rows();
        if folded {
            self.selected_row = selected.and_then(|node| self.visible_row_of(node));
        }
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    fn collapse_finished_folders(&mut self, files: &[usize]) -> bool {
        let mask = &self.hidden_mask;
        let progress = self
            .tree
            .progress_tracked(&self.viewed, |f| !mask.get(f).copied().unwrap_or(false));
        let folders = self.tree.completed_folders(files, &progress);
        let folded = !folders.is_empty();
        self.collapsed.extend(folders);
        folded
    }

    fn visible_row_of(&self, node: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.node == node).or_else(|| {
            let shown = self
                .tree
                .ancestors(node)
                .filter(|a| self.collapsed.contains(&a.path))
                .last()?;
            self.rows.iter().position(|r| r.path.as_ref() == shown.path)
        })
    }

    pub fn viewed_count(&self) -> usize {
        self.progress().0
    }

    pub fn reset_progress(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let files: Vec<usize> = self
            .nav_order
            .iter()
            .copied()
            .filter(|&f| self.viewed[f])
            .collect();
        if files.is_empty() {
            return;
        }
        self.set_viewed(&files, false, cx);
        for &ix in &files {
            let file = &review.files[ix];
            self.kept_unviewed
                .remove(&(file.path.clone(), file.fingerprint.clone()));
        }
        self.shown_at = Some(Instant::now());
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
        let order = &self.nav_order;
        let (after, before) = match order.iter().position(|&x| x == ix) {
            Some(pos) => (&order[pos + 1..], &order[..pos]),
            None => match self.visible_neighbour(ix, true) {
                Some(next) => {
                    let pos = order.iter().position(|&x| x == next)?;
                    (&order[pos..], &order[..pos])
                }
                None => (&order[..], &order[..0]),
            },
        };
        after
            .iter()
            .chain(before)
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
            let mut files = self.tree.files_under(row.node);
            files.retain(|&f| self.is_tracked(f));
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
        self.load_approval(cx);
        let known_me = self.me.clone();
        self._discussions = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move {
                let me = match known_me {
                    Some(me) => Some(me),
                    None => ctx
                        .client
                        .current_user()
                        .await
                        .inspect_err(|err| log::warn!("current user: {err}"))
                        .ok()
                        .map(|user| user.username),
                };
                let discussions = ctx
                    .client
                    .discussions(&ctx.project, ctx.request.number)
                    .await;
                discussions.map(|list| (list, me))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.discussions_loading = false;
                match result {
                    Ok((discussions, me)) => {
                        if me.is_some() {
                            this.me = me;
                        }
                        let edited = this.editing.as_ref().map(|e| e.note_id);
                        if edited.is_some_and(|id| {
                            !discussions
                                .iter()
                                .flat_map(|d| &d.notes)
                                .any(|n| n.id == id)
                        }) {
                            this.editing = None;
                        }
                        this.open_thread = None;
                        let me = this.me.clone();
                        this.discussions = Rc::new(
                            discussions
                                .iter()
                                .filter_map(|d| DiscussionView::from_forge(d, me.as_deref()))
                                .collect(),
                        );
                        let urls: Vec<SharedString> = this
                            .discussions
                            .iter()
                            .flat_map(|d| d.notes.iter().filter_map(|n| n.avatar.clone()))
                            .collect();
                        avatars::request(urls, cx);
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
            self.code_threads = Rc::default();
            self.general_only = Rc::default();
            return;
        };
        let can_place = review.comments_aligned && review.commit.is_none();
        for (ix, discussion) in self.discussions.iter().enumerate() {
            let placed = discussion.position.as_ref().and_then(|pos| {
                if !can_place
                    || pos.head_sha.as_deref() != Some(ctx.refs.head_sha.as_str())
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
                Some(path) => self.by_file.entry(path).or_default().push(ix),
                None => self.general.push(ix),
            }
        }
        let files = comment_list::FilePaths::of(review);
        self.code_threads = Rc::new(comment_list::build_code_threads(
            &self.discussions,
            &files,
            review.tree.file_order(),
            &self.by_file,
            Some(ctx.refs.head_sha.as_str()),
        ));
        self.general_only = Rc::new(
            self.discussions
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.position
                        .as_ref()
                        .is_none_or(|p| p.new_path.is_none() && p.old_path.is_none())
                })
                .map(|(ix, _)| ix)
                .collect(),
        );
        let s = tr();
        self.comment_tab_labels = (
            format!("{} · {}", s.comments_code, self.code_threads.len()).into(),
            format!("{} · {}", s.comments_general, self.general_only.len()).into(),
        );
    }

    pub fn reveal_discussion(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self
            .code_threads
            .iter()
            .find(|thread| thread.discussion == ix)
            .cloned()
        else {
            return;
        };
        let Some(file) = thread.file else {
            return;
        };
        if self.tree.file_node(file).is_none() {
            return;
        }
        if self.current_file != Some(file) {
            self.show_file(file, cx);
        }
        self.focus_diff(window, cx);
        let position = thread.placed.then(|| {
            let pos = self.discussions.get(ix)?.position.as_ref()?;
            let path = self.review.as_ref()?.files.get(file)?.path.clone();
            match (pos.new_line, pos.old_line) {
                (Some(line), _) => Some(DiffLinePosition::new(
                    path,
                    DiffSide::Modified,
                    line as usize,
                )),
                (None, Some(line)) => Some(DiffLinePosition::new(
                    path,
                    DiffSide::Original,
                    line as usize,
                )),
                (None, None) => None,
            }
        });
        if let Some(Some(position)) = position
            && !self.content_pending()
        {
            self.change_cursor = None;
            self.diff
                .update(cx, |state, cx| state.scroll_to_line(position, cx));
        }
        cx.notify();
    }

    pub fn toggle_open_thread(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.open_thread = if self.open_thread == Some(ix) {
            None
        } else {
            Some(ix)
        };
        cx.notify();
    }

    pub fn toggle_note(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.expanded_notes.remove(&id) {
            self.expanded_notes.insert(id);
        }
        cx.notify();
    }

    fn rebuild_annotations(&mut self) {
        let mut annotations = Vec::new();
        let other_content = self.interdiff.is_some() || self.interdiff_loading.is_some();
        if let (Some(review), Some(file), false) =
            (self.review.as_ref(), self.current_file, other_content)
        {
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
                && review.commit.is_none()
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
        if self.mr.is_none() || self.review.as_ref().is_none_or(|r| r.commit.is_some()) {
            return;
        }
        let side = range.end_side();
        let position = DiffLinePosition::new(range.path().clone(), side, range.end());
        if !self.line_commentable(&position) {
            window.push_notification(
                Notification::error(tr().comment_line_unavailable.clone()),
                cx,
            );
            return;
        }
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
            ComposerTarget::Line(_) if review.commit.is_some() => {
                window.push_notification(
                    Notification::error(tr().comment_line_unavailable.clone()),
                    cx,
                );
                return;
            }
            ComposerTarget::Line(position) => {
                let Some(file) = review.file_index(position.path()) else {
                    return;
                };
                let file = &review.files[file];
                let side = match position.side() {
                    DiffSide::Original => Side::Old,
                    _ => Side::New,
                };
                let Some(kind) =
                    file.line_map
                        .locate(side, position.line() as u32)
                        .filter(|&kind| {
                            ctx.kind()
                                .comment_context()
                                .is_none_or(|context| file.line_map.near_change(kind, context))
                        })
                else {
                    window.push_notification(
                        Notification::error(tr().comment_line_unavailable.clone()),
                        cx,
                    );
                    return;
                };
                PostRequest::New(Some(forge::NewPosition {
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
                    PostRequest::Reply(id) => {
                        ctx.client
                            .reply(&ctx.project, ctx.request.number, &id, &body)
                            .await
                    }
                    PostRequest::New(position) => {
                        ctx.client
                            .comment(&ctx.project, ctx.request.number, &body, position.as_ref())
                            .await
                    }
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
        resolve_id: String,
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
                    .set_resolved(&ctx.project, ctx.request.number, &resolve_id, resolved)
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

    pub fn discussions(&self) -> Rc<Vec<DiscussionView>> {
        self.discussions.clone()
    }

    pub fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree_visible
            && (self.tree_focus.contains_focused(window, cx)
                || self.file_search_focused(window, cx))
        {
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

    pub fn resize_split(&mut self, ratio: f32, cx: &mut Context<Self>) {
        let (min, max) = SPLIT_RATIO_RANGE;
        let ratio = (ratio.clamp(min, max) * 1000.).round() / 1000.;
        if (ratio - self.split_ratio).abs() < 0.001 {
            return;
        }
        self.split_ratio = ratio;
        let generation = SPLIT_RATIO_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SPLIT_RATIO_SAVE_DELAY).await;
            if SPLIT_RATIO_GENERATION.load(Ordering::Relaxed) == generation {
                cx.update(|cx| update_settings(cx, |s| s.split_ratio = ratio));
            }
        })
        .detach();
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

    pub fn comments_count(&self) -> usize {
        self.code_threads.len() + self.general_only.len()
    }

    pub fn has_mr(&self) -> bool {
        self.mr.is_some()
    }

    fn line_commentable(&self, position: &DiffLinePosition) -> bool {
        let Some(context) = self
            .mr
            .as_ref()
            .and_then(|ctx| ctx.kind().comment_context())
        else {
            return true;
        };
        let Some(file) = self
            .review
            .as_ref()
            .and_then(|review| review.files.get(review.file_index(position.path())?))
        else {
            return false;
        };
        let side = match position.side() {
            DiffSide::Original => Side::Old,
            _ => Side::New,
        };
        file.line_map
            .locate(side, position.line() as u32)
            .is_some_and(|kind| file.line_map.near_change(kind, context))
    }

    pub fn comments_title(&self) -> SharedString {
        match self.mr.as_ref().map(MrContext::kind) {
            Some(ForgeKind::GitHub) => tr().pr_comments.clone(),
            _ => tr().general_comments.clone(),
        }
    }

    pub fn focus_tree(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.tree_focus, cx);
    }

    pub fn focus_main(&self, window: &mut Window, cx: &mut App) {
        if self.tree_visible {
            self.focus_tree(window, cx);
        } else {
            self.focus_diff(window, cx);
        }
    }

    fn focus_diff(&self, window: &mut Window, cx: &mut App) {
        let handle = self.diff.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

const FOLD_CONTEXT: usize = 3;

fn context_lines(cx: &App) -> Option<usize> {
    (!settings(cx).show_whole_file).then_some(FOLD_CONTEXT)
}

enum PostRequest {
    Reply(String),
    New(Option<forge::NewPosition>),
}

impl Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.tree_focus.clone()
    }
}

async fn load_review(
    repo: Repo,
    source: ReviewSource,
    commit: Option<String>,
    client: Option<forge::Client>,
    ignore_whitespace: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<(LoadedReview, Option<MrContext>)> {
    match source {
        ReviewSource::Branches { target, source } => {
            let review = cx
                .background_spawn(async move {
                    let comparison = repo.compare(&target, &source)?;
                    model::build(&repo, comparison, commit.as_deref(), ignore_whitespace)
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
        ReviewSource::MergeRequest {
            forge: kind,
            project,
            number,
        } => {
            let (not_configured, no_remote) = match kind {
                ForgeKind::GitLab => (&tr().gitlab_not_configured, &tr().no_remote_for_mr),
                ForgeKind::GitHub => (&tr().github_not_configured, &tr().no_remote_for_pr),
            };
            let no_remote = no_remote.to_string();
            let client = client.ok_or_else(|| anyhow!(not_configured.to_string()))?;
            let request = {
                let client = client.clone();
                let project = project.clone();
                net_runtime::spawn(async move { client.request(&project, number).await }).await?
            };
            let diff_refs = request.diff_refs.clone().ok_or_else(|| {
                anyhow!(match kind {
                    ForgeKind::GitLab => tr().mr_not_ready.to_string(),
                    ForgeKind::GitHub => tr().pr_not_ready.to_string(),
                })
            })?;
            let missing_remote = {
                let repo = repo.clone();
                let needed = diff_refs.needed_commits().map(str::to_string);
                let forges = vec![(kind, client.base_url().to_string())];
                let project = project.clone();
                cx.background_spawn(async move {
                    if needed.iter().all(|sha| repo.has_commit(sha)) {
                        Ok(None)
                    } else {
                        model::forge_remote(&repo, &forges, Some(&project))
                            .map(|found| found.remote)
                            .or_else(|| repo.upstream_remote())
                            .map(Some)
                            .ok_or_else(|| anyhow!(no_remote))
                    }
                })
                .await?
            };
            if let Some(remote) = missing_remote {
                let specs = request.refspecs();
                let fetch_repo = repo.clone();
                net_runtime::spawn(async move { fetch_repo.fetch(&remote, &specs).await }).await?;
            }
            let (review, refs) = {
                let repo = repo.clone();
                let request = request.clone();
                cx.background_spawn(async move {
                    let refs = model::resolve_mr_refs(&repo, &diff_refs)?;
                    let comparison = model::comparison_for_mr(&request, &refs);
                    let review =
                        model::build(&repo, comparison, commit.as_deref(), ignore_whitespace)?;
                    anyhow::Ok((review, refs))
                })
                .await?
            };
            Ok((
                review,
                Some(MrContext {
                    client,
                    project,
                    request,
                    refs,
                }),
            ))
        }
    }
}
