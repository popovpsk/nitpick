use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, ElementId, Entity,
    Focusable as _, HighlightStyle, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Subscription, UniformListScrollHandle, Window, div, ease_out_quint,
    prelude::FluentBuilder as _, px, relative, rems, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use review_core::{FileStatus, highlight_ranges, rank};

use super::ReviewView;
use super::diff_panel::status_icon;
use super::tree_panel::ROW_HEIGHT;
use crate::localization::tr;

const SLIDE: Duration = Duration::from_millis(160);
const FIELD_HEIGHT: f32 = 2.25;

pub(super) struct FileSearch {
    pub(super) input: Entity<InputState>,
    query: SharedString,
    rows: Rc<Vec<SearchRow>>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl FileSearch {
    pub(super) fn is_filtering(&self) -> bool {
        !self.query.is_empty()
    }
}

pub(super) struct SearchRow {
    file: usize,
    name: SharedString,
    dir: SharedString,
    name_matches: Vec<Range<usize>>,
    dir_matches: Vec<Range<usize>>,
    status: FileStatus,
}

impl ReviewView {
    pub(super) fn open_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree_visible {
            self.toggle_tree(window, cx);
        }
        if let Some(search) = &self.file_search {
            let input = search.input.clone();
            input.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            });
            return;
        }
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr().find_file_placeholder.clone()));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.rank_file_search(cx),
                InputEvent::PressEnter { .. } => this.open_search_selection(window, cx),
                _ => {}
            },
        );
        input.update(cx, |state, cx| state.focus(window, cx));
        self.search_opens += 1;
        self.file_search = Some(FileSearch {
            input,
            query: SharedString::default(),
            rows: Rc::default(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn toggle_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_search.is_some() {
            self.close_file_search(window, cx);
        } else {
            self.open_file_search(window, cx);
        }
    }

    pub(super) fn close_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_search.take().is_some() {
            if let Some(row) = self.selected_row {
                self.tree_scroll
                    .scroll_to_item(row, gpui::ScrollStrategy::Center);
            }
            self.focus_main(window, cx);
            cx.notify();
        }
    }

    pub(super) fn file_search_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.file_search
            .as_ref()
            .is_some_and(|search| search.input.focus_handle(cx).is_focused(window))
    }

    pub(super) fn rank_file_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.file_search.as_ref() else {
            return;
        };
        let query: SharedString = search.input.read(cx).value().trim().to_string().into();
        let previous = (query == search.query)
            .then(|| search.rows.get(search.selected).map(|row| row.file))
            .flatten();
        let rows = self.search_rows(&query);
        let selected = previous
            .and_then(|file| rows.iter().position(|row| row.file == file))
            .unwrap_or(0);
        if let Some(search) = self.file_search.as_mut() {
            search.query = query;
            search.rows = Rc::new(rows);
            search.selected = selected;
            search
                .scroll
                .scroll_to_item(selected, gpui::ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn search_rows(&self, query: &str) -> Vec<SearchRow> {
        let Some(review) = self.review.as_ref() else {
            return Vec::new();
        };
        if query.is_empty() {
            return Vec::new();
        }
        let files = self.tree.file_order();
        let items = files.iter().map(|&file| {
            let path = review.files[file].path.as_ref();
            let name = path.rsplit('/').next().unwrap_or(path);
            (name, Some(path))
        });
        rank(query, items)
            .into_iter()
            .map(|ranked| {
                let file = files[ranked.index];
                let path = &review.files[file].path;
                let name_start = path.rfind('/').map_or(0, |slash| slash + 1);
                let name = &path[name_start..];
                let dir = &path[..name_start.saturating_sub(1)];
                let mut name_positions = ranked.label;
                let mut dir_positions = Vec::new();
                for position in ranked.detail {
                    if position >= name_start {
                        name_positions.push(position - name_start);
                    } else {
                        dir_positions.push(position);
                    }
                }
                name_positions.sort_unstable();
                SearchRow {
                    file,
                    name_matches: highlight_ranges(name, &name_positions),
                    dir_matches: highlight_ranges(dir, &dir_positions),
                    name: name.to_string().into(),
                    dir: dir.to_string().into(),
                    status: review.files[file].status,
                }
            })
            .collect()
    }

    fn move_search(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(search) = self.file_search.as_mut() else {
            return;
        };
        if !search.is_filtering() {
            self.move_selection(delta, cx);
            return;
        }
        if search.rows.is_empty() {
            return;
        }
        let last = search.rows.len() as isize - 1;
        let selected = (search.selected as isize + delta).clamp(0, last) as usize;
        search.selected = selected;
        search
            .scroll
            .scroll_to_item(selected, gpui::ScrollStrategy::Nearest);
        let file = search.rows[selected].file;
        self.show_file(file, cx);
    }

    fn open_search_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.file_search.as_ref() else {
            return;
        };
        let file = if search.is_filtering() {
            search.rows.get(search.selected).map(|row| row.file)
        } else {
            self.selected_row
                .and_then(|row| self.rows.get(row))
                .and_then(|row| row.file)
        };
        if let Some(file) = file {
            self.show_file(file, cx);
            self.focus_diff(window, cx);
        }
    }

    fn click_search_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(search) = self.file_search.as_mut() else {
            return;
        };
        let Some(file) = search.rows.get(ix).map(|row| row.file) else {
            return;
        };
        search.selected = ix;
        self.show_file(file, cx);
    }

    pub(super) fn render_search_field(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.file_search.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let count = search.is_filtering().then(|| search.rows.len().to_string());
        Some(
            div()
                .id("file-search-row")
                .overflow_hidden()
                .child(
                    div()
                        .pt_1p5()
                        .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            let keys = &event.keystroke;
                            if keys.modifiers.modified() {
                                return;
                            }
                            match keys.key.as_str() {
                                "escape" => {
                                    cx.stop_propagation();
                                    this.close_file_search(window, cx);
                                }
                                "down" => {
                                    cx.stop_propagation();
                                    this.move_search(1, cx);
                                }
                                "up" => {
                                    cx.stop_propagation();
                                    this.move_search(-1, cx);
                                }
                                _ => {}
                            }
                        }))
                        .child(
                            Input::new(&search.input)
                                .small()
                                .cleanable(true)
                                .prefix(Icon::new(IconName::Search).small().text_color(muted))
                                .when_some(count, |input, count| {
                                    input.suffix(div().text_xs().text_color(muted).child(count))
                                }),
                        ),
                )
                .with_animation(
                    ElementId::NamedInteger("file-search-slide".into(), self.search_opens as u64),
                    Animation::new(SLIDE).with_easing(ease_out_quint()),
                    |el, delta| el.max_h(rems(FIELD_HEIGHT * delta)).opacity(delta),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_search_results(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let results = self.render_search_list(cx)?;
        Some(
            div()
                .id("file-search-results-area")
                .size_full()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(results)
                .into_any_element(),
        )
    }

    fn render_search_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.file_search.as_ref().filter(|s| s.is_filtering())?;
        if search.rows.is_empty() {
            return Some(
                v_flex()
                    .size_full()
                    .pt_6()
                    .items_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr().no_matching_files.clone())
                    .into_any_element(),
            );
        }
        let rows = search.rows.clone();
        let selected = search.selected;
        Some(
            uniform_list(
                "file-search-results",
                rows.len(),
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|ix| this.render_search_row(ix, &rows[ix], selected == ix, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&search.scroll)
            .size_full()
            .into_any_element(),
        )
    }

    fn render_search_row(
        &self,
        ix: usize,
        row: &SearchRow,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let viewed = self.viewed.get(row.file).copied().unwrap_or(false);
        let is_current = self.current_file == Some(row.file);
        let highlight = HighlightStyle {
            color: Some(theme.blue),
            ..Default::default()
        };
        let (icon, color) = status_icon(row.status, cx);
        let hover = theme.list_hover;
        h_flex()
            .id(("file-search-row", ix))
            .h(rems(ROW_HEIGHT))
            .w_full()
            .pl(px(8.))
            .pr_1()
            .gap_1p5()
            .items_center()
            .text_sm()
            .text_color(if viewed {
                theme.muted_foreground
            } else {
                theme.sidebar_foreground
            })
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.list_active))
            .when(is_current && !selected, |el| {
                el.border_l_2().border_color(theme.primary)
            })
            .hover(move |style| style.bg(hover))
            .child(Icon::new(icon).small().text_color(color))
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.7))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(
                        StyledText::new(row.name.clone()).with_highlights(
                            row.name_matches.iter().map(|r| (r.clone(), highlight)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis_start()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        StyledText::new(row.dir.clone()).with_highlights(
                            row.dir_matches.iter().map(|r| (r.clone(), highlight)),
                        ),
                    ),
            )
            .when(viewed, |el| {
                el.child(
                    Icon::new(IconName::Check)
                        .xsmall()
                        .text_color(theme.success),
                )
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.click_search_row(ix, cx)))
    }
}
