use gpui::{
    Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use review_core::FileStatus;
use ui_resources::Icons;

use super::{
    CollapseSelected, ExpandSelected, NextFile, OpenSelected, PreviousFile, ReviewView, SelectNext,
    SelectPrevious, ToggleViewed, TreeRow,
};
use crate::localization::tr;

const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;

impl ReviewView {
    pub(super) fn render_tree(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (_, total) = self.progress();
        let s = tr();
        let theme = cx.theme();
        let header = v_flex()
            .flex_none()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .child(s.files.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(total.to_string()),
                    )
                    .child(
                        Button::new("tree-expand-all")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::ChevronsUpDown))
                            .tooltip(s.expand_all.clone())
                            .on_click(cx.listener(|this, _, _, cx| this.expand_all(cx))),
                    )
                    .child(
                        Button::new("tree-collapse-all")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::ChevronsDownUp))
                            .tooltip(s.collapse_all.clone())
                            .on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx))),
                    ),
            );

        let rows = self.rows.clone();
        let selected = self.selected_row;
        let current = self.current_file;
        let list = uniform_list(
            "review-tree",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|ix| this.render_row(ix, &rows[ix], selected == Some(ix), current, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree_scroll)
        .size_full();

        v_flex()
            .id("review-tree-panel")
            .key_context("ReviewTree")
            .track_focus(&self.tree_focus)
            .size_full()
            .min_w_0()
            .bg(theme.sidebar)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &NextFile, _, cx| this.step_file(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousFile, _, cx| this.step_file(-1, cx)))
            .on_action(cx.listener(|this, _: &CollapseSelected, _, cx| this.collapse_selected(cx)))
            .on_action(cx.listener(|this, _: &ExpandSelected, _, cx| this.expand_selected(cx)))
            .on_action(
                cx.listener(|this, _: &OpenSelected, window, cx| this.open_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleViewed, _, cx| {
                if let Some(row) = this.selected_row {
                    this.toggle_row_viewed(row, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_tree(window, cx)),
            )
            .child(header)
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn render_row(
        &self,
        ix: usize,
        row: &TreeRow,
        selected: bool,
        current: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let complete = row.progress.is_complete();
        let partial = row.progress.is_partial();
        let is_current = row.file.is_some() && row.file == current;
        let text_color = if complete {
            theme.muted_foreground
        } else {
            theme.sidebar_foreground
        };
        let icon = if row.is_dir {
            Icon::new(if row.expanded {
                IconName::FolderOpen
            } else {
                IconName::Folder
            })
            .text_color(if complete {
                theme.success
            } else {
                theme.muted_foreground
            })
        } else {
            let (name, color) = match row.status {
                FileStatus::Added => (Icons::FilePlus, theme.green),
                FileStatus::Deleted => (Icons::FileMinus, theme.red),
                FileStatus::Renamed => (Icons::FileDiff, theme.blue),
                FileStatus::Modified => (Icons::FileDiff, theme.yellow),
            };
            Icon::new(name).text_color(color)
        };
        let chevron = if row.is_dir {
            Icon::new(if row.expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .xsmall()
            .text_color(theme.muted_foreground)
            .into_any_element()
        } else {
            div().w(px(12.)).into_any_element()
        };
        let mark = if complete {
            Some(
                Icon::new(IconName::Check)
                    .xsmall()
                    .text_color(theme.success)
                    .into_any_element(),
            )
        } else if partial && row.is_dir {
            Some(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}/{}", row.progress.viewed, row.progress.total))
                    .into_any_element(),
            )
        } else {
            None
        };
        let comments = (row.comments > 0).then(|| {
            h_flex()
                .gap_0p5()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(Icon::new(Icons::MessageSquare).xsmall())
                .child(row.comments.to_string())
        });
        let background = if selected {
            Some(theme.list_active)
        } else if complete {
            Some(theme.success.opacity(0.06))
        } else {
            None
        };
        let hover = theme.list_hover;
        let toggle_tooltip = if complete {
            tr().mark_unviewed.clone()
        } else {
            tr().mark_viewed.clone()
        };
        h_flex()
            .id(("tree-row", ix))
            .group("tree-row")
            .h(px(ROW_HEIGHT))
            .w_full()
            .pl(px(6. + row.depth as f32 * INDENT))
            .pr_1()
            .gap_1()
            .items_center()
            .text_sm()
            .text_color(text_color)
            .cursor_pointer()
            .when_some(background, |el, bg| el.bg(bg))
            .when(is_current && !selected, |el| {
                el.border_l_2().border_color(theme.primary)
            })
            .hover(move |style| style.bg(hover))
            .child(chevron)
            .child(icon.small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(complete && !row.is_dir, |el| el.line_through())
                    .child(row.name.clone()),
            )
            .children(comments)
            .children(mark)
            .child(
                div()
                    .invisible()
                    .group_hover("tree-row", |style| style.visible())
                    .child(
                        Button::new(("tree-row-viewed", ix))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if complete {
                                Icons::Undo2
                            } else {
                                Icons::SquareCheck
                            }))
                            .tooltip(toggle_tooltip)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_row_viewed(ix, cx);
                            })),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.click_row(ix, cx)))
    }

    fn collapse_selected(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_row else {
            return;
        };
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        if row.is_dir && row.expanded {
            self.toggle_dir(ix, cx);
            return;
        }
        if let Some(parent) = self.rows[..ix]
            .iter()
            .rposition(|r| r.is_dir && r.depth < row.depth)
        {
            self.selected_row = Some(parent);
            self.tree_scroll
                .scroll_to_item(parent, gpui::ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn expand_selected(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_row else {
            return;
        };
        match self.rows.get(ix).map(|r| (r.is_dir, r.expanded)) {
            Some((true, false)) => self.toggle_dir(ix, cx),
            Some((true, true)) => self.move_selection(1, cx),
            _ => {}
        }
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_row else {
            return;
        };
        match self.rows.get(ix).map(|r| (r.is_dir, r.file)) {
            Some((true, _)) => self.toggle_dir(ix, cx),
            Some((false, Some(file))) => {
                self.show_file(file, cx);
                self.focus_diff(window, cx);
            }
            _ => {}
        }
    }
}
