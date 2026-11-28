use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, SharedString, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{DiffDecoration, DiffLinePosition, DiffLineRange, DiffSide},
    h_flex,
    input::{Enter, InputEvent, Textarea, TextareaState},
};
use review_core::{LineKind, LineMap, Side};

use super::find_match::{
    FindText, Hit, TextSource, find_hits, hit_lines, query_for, seed_query, start_hit, text_side,
};
use super::{FindInFile, ReviewSource, ReviewView};
use crate::localization::tr;

const LEAD_ROWS: usize = 3;
const FIELD_ROWS: usize = 6;
const SCAN_DELAY: Duration = Duration::from_millis(120);
const INDEX_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct FindSource {
    pub(super) key: String,
    pub(super) path: SharedString,
    pub(super) old: TextSource,
    pub(super) new: TextSource,
    pub(super) map: LineMap,
}

struct FindIndex {
    generation: u64,
    texts: Vec<Option<FindText>>,
}

pub(super) struct FindBar {
    input: Entity<TextareaState>,
    query: String,
    case: bool,
    text: Option<Rc<FindText>>,
    loading: Option<String>,
    hits: Vec<Hit>,
    current: Option<usize>,
    anchor: Option<DiffLinePosition>,
    jump_on_load: bool,
    index: Option<Arc<FindIndex>>,
    indexing: Option<u64>,
    files: usize,
    count: SharedString,
    alert: bool,
    decorations: Rc<Vec<DiffDecoration>>,
    _subscription: Subscription,
    _load: Task<()>,
    _index: Task<()>,
    _scan: Task<()>,
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
        Some(FindSource {
            key: format!(
                "{}\0{}\0{}",
                file.path, file.fingerprint, self.installed_whitespace
            ),
            path: file.path.clone(),
            old: text_side(&file.old_blob),
            new: self.new_text_source(&file.new_blob, file.new_path.as_ref()),
            map: file.shown_map().clone(),
        })
    }

    fn new_text_source(&self, blob: &str, path: &str) -> TextSource {
        match (&self.source, text_side(blob)) {
            (_, TextSource::None) => TextSource::None,
            (ReviewSource::WorkingTree, _) => TextSource::Disk(self.repo.root().join(path)),
            (_, side) => side,
        }
    }

    pub(super) fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() {
            self.close_find(window, cx);
        } else {
            self.open_find(window, cx);
        }
    }

    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seed = {
            let diff = self.diff.read(cx);
            match diff.selected_lines() {
                Some(_) => None,
                None => seed_query(&diff.selected_text(cx)).map(str::to_owned),
            }
        };
        let anchor = seed.as_ref().and(self.hovered_line.clone());
        if let Some(find) = self.find.as_mut() {
            let input = find.input.clone();
            input.update(cx, |state, cx| {
                if let Some(seed) = &seed {
                    state.set_value(seed.clone(), window, cx);
                }
                state.focus(window, cx);
                state.select_all(window, cx);
            });
            if seed.is_some() {
                find.anchor = anchor;
                if find.text.is_none() && find.loading.is_some() {
                    find.jump_on_load = true;
                }
                self.run_find(true, cx);
            }
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, FIELD_ROWS)
                .placeholder(tr().find_in_file_placeholder.clone())
                .submit_on_enter(true)
        });
        if let Some(seed) = &seed {
            input.update(cx, |state, cx| {
                state.set_value(seed.clone(), window, cx);
                state.select_all(window, cx);
            });
        }
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
            case: self.find_case,
            text: None,
            loading: None,
            hits: Vec::new(),
            current: None,
            jump_on_load: seed.is_some(),
            anchor,
            index: None,
            indexing: None,
            files: 0,
            count: SharedString::default(),
            alert: false,
            decorations: Rc::default(),
            _subscription: subscription,
            _load: Task::ready(()),
            _index: Task::ready(()),
            _scan: Task::ready(()),
        });
        self.sync_find(cx);
        self.sync_find_index(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            self.set_find_files(Rc::default());
            self.focus_diff(window, cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_find_case(&mut self, cx: &mut Context<Self>) {
        self.find_case = !self.find_case;
        self.run_find(true, cx);
    }

    fn set_find_files(&mut self, counts: Rc<Vec<usize>>) {
        if self.find_files.is_empty() && counts.is_empty() {
            return;
        }
        self.find_files = counts;
        self.rebuild_rows();
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
                    FindText::new(
                        source.key,
                        source.path,
                        source.map.rows(),
                        [
                            source.map.shown_lines(Side::Old, context, min_folded),
                            source.map.shown_lines(Side::New, context, min_folded),
                        ],
                        source.old.read(&repo),
                        source.new.read(&repo),
                    )
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

    pub(super) fn sync_find_index(&mut self, cx: &mut Context<Self>) {
        let generation = self.install_generation;
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find
            .index
            .as_ref()
            .is_some_and(|index| index.generation == generation)
            || find.indexing == Some(generation)
        {
            return;
        }
        let Some(review) = self.review.clone() else {
            return;
        };
        let sources: Vec<Option<FindSource>> = review
            .files
            .iter()
            .map(|file| {
                (!file.binary).then(|| FindSource {
                    key: String::new(),
                    path: file.path.clone(),
                    old: text_side(&file.old_blob),
                    new: self.new_text_source(&file.new_blob, file.new_path.as_ref()),
                    map: file.shown_map().clone(),
                })
            })
            .collect();
        let Some(find) = self.find.as_mut() else {
            return;
        };
        find.indexing = Some(generation);
        find.files = 0;
        find._scan = Task::ready(());
        let repo = self.repo.clone();
        find._index = cx.spawn(async move |this, cx| {
            let texts = cx
                .background_spawn(async move {
                    let mut used = 0usize;
                    let mut texts = Vec::with_capacity(sources.len());
                    for source in sources {
                        let text = source.filter(|_| used < INDEX_LIMIT).map(|source| {
                            let text = FindText::new(
                                source.key,
                                source.path,
                                source.map.rows(),
                                [Vec::new(), Vec::new()],
                                source.old.read(&repo),
                                source.new.read(&repo),
                            );
                            used += text.size();
                            text
                        });
                        texts.push(text);
                    }
                    if used >= INDEX_LIMIT {
                        log::info!("find: the review's text is too large to search in full");
                    }
                    texts
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(find) = this.find.as_mut() else {
                    return;
                };
                if find.indexing != Some(generation) {
                    return;
                }
                find.indexing = None;
                find.index = Some(Arc::new(FindIndex { generation, texts }));
                this.scan_find_files(cx);
            });
        });
    }

    fn scan_find_files(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let (query, case) = (find.query.clone(), find.case);
        let index = find.index.clone();
        let Some(index) = index.filter(|_| !query.is_empty()) else {
            find._scan = Task::ready(());
            find.files = 0;
            self.set_find_files(Rc::default());
            self.refresh_find_view(cx);
            cx.notify();
            return;
        };
        find._scan = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SCAN_DELAY).await;
            let scanned = query.clone();
            let counts = cx
                .background_spawn(async move {
                    index
                        .texts
                        .iter()
                        .map(|text| {
                            text.as_ref()
                                .map_or(0, |text| find_hits(text, &scanned, case).len())
                        })
                        .collect::<Vec<usize>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(find) = this.find.as_mut() else {
                    return;
                };
                if find.query != query || find.case != case {
                    return;
                }
                let counts = Rc::new(counts);
                find.files = counts.iter().filter(|&&count| count > 0).count();
                this.set_find_files(counts);
                this.refresh_find_view(cx);
                cx.notify();
            });
        });
    }

    fn run_find(&mut self, jump: bool, cx: &mut Context<Self>) {
        let case = self.find_case;
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let query = query_for(&find.input.read(cx).value(), case);
        let changed = query != find.query || case != find.case;
        find.query = query;
        find.case = case;
        let text = find.text.clone();
        if text.is_none() && find.loading.is_some() && jump && changed {
            find.jump_on_load = true;
        }
        find.hits = match &text {
            Some(text) => find_hits(text, &find.query, case),
            None => Vec::new(),
        };
        find.current = match &text {
            Some(text) if jump && !find.hits.is_empty() => {
                let anchor = find
                    .anchor
                    .take()
                    .filter(|anchor| *anchor.path() == text.path);
                let anchor = anchor.and_then(|anchor| {
                    let side = match anchor.side() {
                        DiffSide::Original => Side::Old,
                        DiffSide::Modified => Side::New,
                    };
                    Some((side, u32::try_from(anchor.line()).ok()?))
                });
                let firsts: Vec<usize> = find.hits.iter().map(Hit::first).collect();
                Some(start_hit(&text.rows, &firsts, anchor))
            }
            _ => None,
        };
        if changed {
            self.scan_find_files(cx);
        }
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
            for (k, hit) in find.hits.iter().enumerate() {
                if Some(k) != find.current {
                    hit_lines(text, hit, &mut old, &mut new);
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
            if let Some(hit) = find.current.and_then(|k| find.hits.get(k)) {
                let (mut old, mut new) = (Vec::new(), Vec::new());
                hit_lines(text, hit, &mut old, &mut new);
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
        find.alert = !find.query.is_empty() && find.hits.is_empty() && find.files == 0;
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
        let can_step = !find.hits.is_empty();
        Some(
            h_flex()
                .id("find-bar")
                .absolute()
                .top_2()
                .right(px(18.))
                .w(px(360.))
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
                .capture_action(cx.listener(|this, action: &Enter, _, cx| {
                    if action.shift && !action.secondary {
                        cx.stop_propagation();
                        this.step_find(false, cx);
                    }
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_1()
                        .pl_2()
                        .pr_1()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.input)
                        .bg(theme.background)
                        .child(
                            Icon::new(IconName::Search)
                                .small()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Textarea::new(&find.input).small().appearance(false)),
                        )
                        .when(!find.count.is_empty(), |field| {
                            field.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .whitespace_nowrap()
                                    .text_color(if find.alert {
                                        theme.danger
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(find.count.clone()),
                            )
                        }),
                )
                .child(
                    Button::new("find-case")
                        .ghost()
                        .xsmall()
                        .label("Aa")
                        .selected(self.find_case)
                        .tooltip(s.find_match_case.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_find_case(cx))),
                )
                .child(
                    Button::new("find-previous")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronUp)
                        .disabled(!can_step)
                        .tooltip(s.find_previous.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(false, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .disabled(!can_step)
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
    let ix = find.hits.get(find.current?)?.first();
    let row = *text.rows.get(ix)?;
    let mut anchor = ix;
    if text.is_shown(row) {
        while anchor > 0 && ix - anchor < LEAD_ROWS && text.is_shown(text.rows[anchor - 1]) {
            anchor -= 1;
        }
    }
    Some((text.path.clone(), text.rows[anchor]))
}
