use std::path::PathBuf;
use std::rc::Rc;

use git_repo::Repo;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, SharedString, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{DiffDecoration, DiffLinePosition, DiffLineRange, DiffSide},
    h_flex,
    input::{Input, InputEvent, InputState},
};
use review_core::{LineKind, LineMap, Side};

use super::image_preview::read_regular_file;
use super::since_viewed::is_blob;
use super::{FindInFile, ReviewSource, ReviewView};
use crate::localization::tr;

const TEXT_LIMIT: usize = 16 * 1024 * 1024;
const LEAD_ROWS: usize = 3;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum TextSource {
    None,
    Blob(String),
    Disk(PathBuf),
}

impl TextSource {
    fn read(&self, repo: &Repo) -> Vec<String> {
        let bytes = match self {
            TextSource::None => return Vec::new(),
            TextSource::Blob(id) => repo.blob(id).map_err(|err| err.to_string()),
            TextSource::Disk(path) => {
                read_regular_file(path, TEXT_LIMIT as u64).map_err(|err| err.to_string())
            }
        };
        match bytes {
            Ok(bytes) if bytes.len() <= TEXT_LIMIT => String::from_utf8_lossy(&bytes)
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_lowercase())
                .collect(),
            Ok(_) => Vec::new(),
            Err(err) => {
                log::warn!("find in file: reading the text failed: {err}");
                Vec::new()
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct FindSource {
    pub(super) key: String,
    pub(super) path: SharedString,
    pub(super) old: TextSource,
    pub(super) new: TextSource,
    pub(super) map: LineMap,
}

struct FindText {
    key: String,
    path: SharedString,
    rows: Vec<LineKind>,
    shown: [Vec<bool>; 2],
    old: Vec<String>,
    new: Vec<String>,
}

impl FindText {
    fn line(&self, row: LineKind) -> Option<&str> {
        match row {
            LineKind::Context { new, .. } | LineKind::Added { new } => {
                self.new.get(new as usize - 1).map(String::as_str)
            }
            LineKind::Removed { old } => self.old.get(old as usize - 1).map(String::as_str),
        }
    }

    fn is_shown(&self, row: LineKind) -> bool {
        let (side, line) = match row {
            LineKind::Context { new, .. } | LineKind::Added { new } => (1, new),
            LineKind::Removed { old } => (0, old),
        };
        self.shown[side]
            .get(line as usize - 1)
            .copied()
            .unwrap_or(false)
    }
}

pub(super) struct FindBar {
    input: Entity<InputState>,
    query: String,
    text: Option<Rc<FindText>>,
    loading: Option<String>,
    hits: Vec<usize>,
    current: Option<usize>,
    jump_on_load: bool,
    count: SharedString,
    decorations: Rc<Vec<DiffDecoration>>,
    _subscription: Subscription,
    _load: Task<()>,
}

impl FindBar {
    pub(super) fn decorations(&self) -> Rc<Vec<DiffDecoration>> {
        self.decorations.clone()
    }

    pub(super) fn forget_text(&mut self) {
        self.text = None;
        self.loading = None;
        self._load = Task::ready(());
    }
}

pub(super) fn text_side(blob: &str) -> TextSource {
    if is_blob(blob) {
        TextSource::Blob(blob.to_string())
    } else {
        TextSource::None
    }
}

fn position(path: &SharedString, row: LineKind) -> DiffLinePosition {
    match row {
        LineKind::Context { new, .. } | LineKind::Added { new } => {
            DiffLinePosition::new(path.clone(), DiffSide::Modified, new as usize)
        }
        LineKind::Removed { old } => {
            DiffLinePosition::new(path.clone(), DiffSide::Original, old as usize)
        }
    }
}

fn push_lines(
    out: &mut Vec<DiffDecoration>,
    path: &SharedString,
    side: DiffSide,
    mut lines: Vec<u32>,
    color: Hsla,
) {
    lines.sort_unstable();
    lines.dedup();
    let mut ix = 0;
    while ix < lines.len() {
        let start = lines[ix];
        let mut end = start;
        while ix + 1 < lines.len() && lines[ix + 1] == end + 1 {
            ix += 1;
            end = lines[ix];
        }
        ix += 1;
        out.push(
            DiffDecoration::new(DiffLineRange::new(
                path.clone(),
                side,
                start as usize,
                end as usize,
            ))
            .with_background(color),
        );
    }
}

impl ReviewView {
    pub(super) fn find_source(&self) -> Option<FindSource> {
        if let Some(source) = self.since_viewed_find_source() {
            return Some(source);
        }
        if self.content_pending() {
            return None;
        }
        let review = self.review.as_ref()?;
        let file = review.files.get(self.current_file?)?;
        if file.binary {
            return None;
        }
        let new = match (&self.source, text_side(&file.new_blob)) {
            (_, TextSource::None) => TextSource::None,
            (ReviewSource::WorkingTree, _) => {
                TextSource::Disk(self.repo.root().join(file.new_path.as_ref()))
            }
            (_, side) => side,
        };
        Some(FindSource {
            key: format!(
                "{}\0{}\0{}",
                file.path, file.fingerprint, self.installed_whitespace
            ),
            path: file.path.clone(),
            old: text_side(&file.old_blob),
            new,
            map: file.shown_map().clone(),
        })
    }

    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = &self.find {
            let input = find.input.clone();
            input.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            });
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr().find_in_file_placeholder.clone())
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.run_find(true, cx),
                    InputEvent::PressEnter { shift, .. } => this.step_find(!shift, cx),
                    _ => {}
                },
            );
        input.update(cx, |state, cx| state.focus(window, cx));
        self.find = Some(FindBar {
            input,
            query: String::new(),
            text: None,
            loading: None,
            hits: Vec::new(),
            current: None,
            jump_on_load: false,
            count: SharedString::default(),
            decorations: Rc::default(),
            _subscription: subscription,
            _load: Task::ready(()),
        });
        self.sync_find(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            self.focus_diff(window, cx);
            cx.notify();
        }
    }

    pub(super) fn sync_find(&mut self, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return;
        }
        let source = self.find_source();
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let Some(source) = source else {
            find.text = None;
            find.loading = None;
            find._load = Task::ready(());
            self.run_find(false, cx);
            return;
        };
        if find
            .text
            .as_ref()
            .is_some_and(|text| text.key == source.key)
            || find.loading.as_ref() == Some(&source.key)
        {
            return;
        }
        find.text = None;
        find.loading = Some(source.key.clone());
        let repo = self.repo.clone();
        let (context, min_folded) = {
            let state = self.diff.read(cx);
            (state.context_lines(), state.min_collapsed_lines())
        };
        find._load = cx.spawn(async move |this, cx| {
            let text = cx
                .background_spawn(async move {
                    FindText {
                        rows: source.map.rows(),
                        shown: [
                            source.map.shown_lines(Side::Old, context, min_folded),
                            source.map.shown_lines(Side::New, context, min_folded),
                        ],
                        old: source.old.read(&repo),
                        new: source.new.read(&repo),
                        key: source.key,
                        path: source.path,
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(find) = this.find.as_mut() else {
                    return;
                };
                if find.loading.as_ref() != Some(&text.key) {
                    return;
                }
                find.loading = None;
                find.text = Some(Rc::new(text));
                let jump = std::mem::take(&mut find.jump_on_load);
                this.run_find(jump, cx);
            });
        });
        self.run_find(false, cx);
    }

    fn run_find(&mut self, jump: bool, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let query = find.input.read(cx).value().to_lowercase();
        let changed = query != find.query;
        find.query = query;
        let text = find.text.clone();
        if text.is_none() && find.loading.is_some() && jump && changed {
            find.jump_on_load = true;
        }
        find.hits = match (&text, find.query.is_empty()) {
            (Some(text), false) => text
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    text.line(**row)
                        .is_some_and(|line| line.contains(&find.query))
                })
                .map(|(ix, _)| ix)
                .collect(),
            _ => Vec::new(),
        };
        find.current = (jump && !find.hits.is_empty()).then_some(0);
        self.refresh_find_view(cx);
        if find_jump_target(self).is_some() && jump {
            self.scroll_to_find_hit(cx);
        }
        cx.notify();
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let count = find.hits.len();
        if count == 0 {
            return;
        }
        find.current = Some(match (find.current, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(k), true) => (k + 1) % count,
            (Some(k), false) => (k + count - 1) % count,
        });
        self.refresh_find_view(cx);
        self.scroll_to_find_hit(cx);
        cx.notify();
    }

    fn scroll_to_find_hit(&mut self, cx: &mut Context<Self>) {
        let Some((path, row)) = find_jump_target(self) else {
            return;
        };
        self.diff.update(cx, |state, cx| {
            state.scroll_to_line(position(&path, row), cx)
        });
        self.note_manual_scroll(cx);
    }

    pub(super) fn refresh_find_view(&mut self, cx: &App) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let theme = cx.theme();
        let hit_color = theme.warning.opacity(0.22);
        let current_color = theme.warning.opacity(0.55);
        let mut decorations = Vec::new();
        if let Some(text) = &find.text {
            let mut old = Vec::new();
            let mut new = Vec::new();
            for (k, &ix) in find.hits.iter().enumerate() {
                if Some(k) == find.current {
                    continue;
                }
                match text.rows[ix] {
                    LineKind::Context { old: o, new: n } => {
                        old.push(o);
                        new.push(n);
                    }
                    LineKind::Added { new: n } => new.push(n),
                    LineKind::Removed { old: o } => old.push(o),
                }
            }
            push_lines(
                &mut decorations,
                &text.path,
                DiffSide::Original,
                old,
                hit_color,
            );
            push_lines(
                &mut decorations,
                &text.path,
                DiffSide::Modified,
                new,
                hit_color,
            );
            if let Some(&ix) = find.current.and_then(|k| find.hits.get(k)) {
                let (old, new) = match text.rows[ix] {
                    LineKind::Context { old, new } => (vec![old], vec![new]),
                    LineKind::Added { new } => (Vec::new(), vec![new]),
                    LineKind::Removed { old } => (vec![old], Vec::new()),
                };
                push_lines(
                    &mut decorations,
                    &text.path,
                    DiffSide::Original,
                    old,
                    current_color,
                );
                push_lines(
                    &mut decorations,
                    &text.path,
                    DiffSide::Modified,
                    new,
                    current_color,
                );
            }
        }
        find.decorations = Rc::new(decorations);
        find.count = match (find.query.is_empty(), find.hits.len(), find.current) {
            (true, _, _) => SharedString::default(),
            (false, 0, _) => tr().no_results.clone(),
            (false, total, Some(k)) => format!("{}/{total}", k + 1).into(),
            (false, total, None) => total.to_string().into(),
        };
    }

    pub(super) fn render_find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let s = tr();
        let theme = cx.theme();
        let has_hits = !find.hits.is_empty();
        Some(
            h_flex()
                .id("find-bar")
                .absolute()
                .top_2()
                .right(px(18.))
                .w(px(340.))
                .p_1()
                .gap_1()
                .items_center()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_md()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                        cx.stop_propagation();
                        this.close_find(window, cx);
                    }
                }))
                .child(
                    Input::new(&find.input)
                        .small()
                        .flex_1()
                        .prefix(
                            Icon::new(IconName::Search)
                                .small()
                                .text_color(theme.muted_foreground),
                        )
                        .when(!find.count.is_empty(), |input| {
                            input.suffix(
                                div()
                                    .text_xs()
                                    .whitespace_nowrap()
                                    .text_color(if has_hits {
                                        theme.muted_foreground
                                    } else {
                                        theme.danger
                                    })
                                    .child(find.count.clone()),
                            )
                        }),
                )
                .child(
                    Button::new("find-previous")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronUp)
                        .disabled(!has_hits)
                        .tooltip(s.find_previous.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(false, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .disabled(!has_hits)
                        .tooltip(s.find_next.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(true, cx))),
                )
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip_with_action(s.find_in_file.clone(), &FindInFile, Some("Review"))
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                )
                .into_any_element(),
        )
    }
}

fn find_jump_target(view: &ReviewView) -> Option<(SharedString, LineKind)> {
    let find = view.find.as_ref()?;
    let text = find.text.as_ref()?;
    let &ix = find.hits.get(find.current?)?;
    let mut anchor = ix;
    if text.is_shown(text.rows[ix]) {
        while anchor > 0 && ix - anchor < LEAD_ROWS && text.is_shown(text.rows[anchor - 1]) {
            anchor -= 1;
        }
    }
    Some((text.path.clone(), text.rows[anchor]))
}
