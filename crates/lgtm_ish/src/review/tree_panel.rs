use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, Context, Entity, HighlightStyle, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Window, div, prelude::FluentBuilder as _, px, relative, rems, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    popover::{Popover, PopoverState},
    switch::Switch,
    v_flex,
};
use review_core::{FileStatus, hidden};
use ui_resources::Icons;

use super::{
    CollapseSelected, ExpandSelected, FindFile, HiddenFile, NextFile, OpenSelected, PreviousFile,
    ReviewEvent, ReviewView, SelectNext, SelectPrevious, ToggleViewed, TreeRow, file_actions,
};
use crate::localization::tr;
use crate::settings_store::{
    HideScope, TreeMode, TreeSide, add_hide_pattern, settings, update_settings,
};

pub(super) const ROW_HEIGHT: f32 = 1.625;
const HIDDEN_ROW: f32 = 1.625;

fn view_options_panel(
    view: &Entity<ReviewView>,
    hidden: &Rc<Vec<HiddenFile>>,
    cx: &mut Context<PopoverState>,
) -> AnyElement {
    let s = tr();
    let current = settings(cx);
    let flat = current.tree_mode == TreeMode::List;
    let show_hidden = current.show_hidden_files;
    let compact = current.compact_folders;
    let theme = cx.theme();
    let popover = cx.entity();
    let rows = hidden.len();
    let list_view = view.clone();
    let list_hidden = hidden.clone();
    let settings_view = view.clone();
    let option = |label: SharedString, control: AnyElement| {
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .justify_between()
            .child(div().text_sm().child(label))
            .child(control)
    };
    v_flex()
        .w(px(340.))
        .gap_2()
        .child(option(
            s.tree_view_label.clone(),
            ButtonGroup::new("tree-mode")
                .small()
                .outline()
                .child(
                    Button::new("tree-mode-tree")
                        .icon(Icon::new(Icons::ListTree))
                        .label(s.tree_mode_tree.clone())
                        .selected(!flat),
                )
                .child(
                    Button::new("tree-mode-list")
                        .icon(IconName::Menu)
                        .label(s.tree_mode_list.clone())
                        .selected(flat),
                )
                .on_click(|clicks: &Vec<usize>, _, cx| {
                    let mode = if clicks.first() == Some(&1) {
                        TreeMode::List
                    } else {
                        TreeMode::Tree
                    };
                    update_settings(cx, |s| s.tree_mode = mode);
                })
                .into_any_element(),
        ))
        .child(div().h(px(1.)).w_full().bg(theme.border))
        .child(option(
            s.compact_folders.clone(),
            Switch::new("tree-compact")
                .small()
                .checked(compact && !flat)
                .disabled(flat)
                .on_click(|checked, _, cx| {
                    let checked = *checked;
                    update_settings(cx, move |s| s.compact_folders = checked)
                })
                .into_any_element(),
        ))
        .child(option(
            s.show_hidden_files.clone(),
            Switch::new("tree-show-hidden")
                .small()
                .checked(show_hidden)
                .on_click(|checked, _, cx| {
                    let checked = *checked;
                    update_settings(cx, move |s| s.show_hidden_files = checked)
                })
                .into_any_element(),
        ))
        .when(rows > 0 && !show_hidden, |el| {
            el.child(div().h(px(1.)).w_full().bg(theme.border)).child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(s.hidden_count(rows)),
                    )
                    .child(
                        uniform_list("hidden-files-list", rows, move |range, _, cx| {
                            let hover = cx.theme().list_hover;
                            let muted = cx.theme().muted_foreground;
                            let highlight = HighlightStyle {
                                color: Some(cx.theme().blue),
                                ..Default::default()
                            };
                            range
                                .map(|ix| {
                                    let file = &list_hidden[ix];
                                    let file_ix = file.ix;
                                    let view = list_view.clone();
                                    let popover = popover.clone();
                                    h_flex()
                                        .id(("hidden-file", ix))
                                        .h(rems(HIDDEN_ROW))
                                        .px_1p5()
                                        .gap_2()
                                        .items_center()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(hover))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_sm()
                                                .child(
                                                    StyledText::new(file.path.clone())
                                                        .with_highlights(file.matched.iter().map(
                                                            |range| (range.clone(), highlight),
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(file.rule.clone()),
                                        )
                                        .on_click(move |_, window, cx| {
                                            view.update(cx, |this, cx| this.show_file(file_ix, cx));
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        })
                                })
                                .collect::<Vec<_>>()
                        })
                        .h(rems(rows.min(8) as f32 * HIDDEN_ROW)),
                    ),
            )
        })
        .child(div().h(px(1.)).w_full().bg(theme.border))
        .child(
            h_flex().justify_end().child(
                Button::new("edit-hidden-files")
                    .ghost()
                    .small()
                    .icon(IconName::Settings)
                    .label(s.edit_hidden_files.clone())
                    .on_click(move |_, _, cx| {
                        settings_view.update(cx, |_, cx| cx.emit(ReviewEvent::EditHiddenFiles))
                    }),
            ),
        )
        .into_any_element()
}

fn hide_menu(
    menu: PopupMenu,
    view: &Entity<ReviewView>,
    node: usize,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = file_actions::row_menu(menu, view, node, cx);
    let s = tr();
    let review = view.read(cx);
    let root = review.repo_root().to_path_buf();
    let Some(node) = review.tree_node(node) else {
        return menu;
    };
    let here = HideScope::Repository(root);
    let item = |label: String, scope: HideScope, pattern: String| {
        PopupMenuItem::new(label)
            .on_click(move |_, _, cx| add_hide_pattern(cx, scope.clone(), pattern.clone()))
    };
    let menu = if node.is_dir() {
        let name = hidden::dir_name_pattern(&node.path);
        menu.item(item(
            s.hide_folder_here.to_string(),
            here,
            hidden::dir_pattern(&node.path),
        ))
        .item(item(s.hide_everywhere(&name), HideScope::Everywhere, name))
    } else {
        let menu = menu.item(item(
            s.hide_file_here.to_string(),
            here.clone(),
            hidden::file_pattern(&node.path),
        ));
        match hidden::family_pattern(&node.path) {
            Some(family) => menu
                .item(item(s.hide_here(&family), here, family.clone()))
                .item(item(
                    s.hide_everywhere(&family),
                    HideScope::Everywhere,
                    family,
                )),
            None => menu,
        }
    };
    let settings_view = view.clone();
    menu.separator()
        .item(
            PopupMenuItem::new(s.edit_hidden_files.clone()).on_click(move |_, _, cx| {
                settings_view.update(cx, |_, cx| cx.emit(ReviewEvent::EditHiddenFiles))
            }),
        )
}
const INDENT: f32 = 14.;

impl ReviewView {
    pub(super) fn render_tree(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let s = tr();
        let flat = self.tree_view.shape.flat;
        let hidden_count = self.hidden.len();
        let show_hidden = self.tree_view.show_hidden;
        let view_options = {
            let view = cx.entity();
            let hidden = self.hidden.clone();
            Popover::new("tree-view-options")
                .anchor(match settings(cx).tree_side {
                    TreeSide::Left => Anchor::TopLeft,
                    TreeSide::Right => Anchor::TopRight,
                })
                .trigger(
                    Button::new("tree-view-options-button")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(if hidden_count > 0 && !show_hidden {
                            Icons::EyeOff
                        } else {
                            Icons::Eye
                        }))
                        .when(hidden_count > 0, |b| b.label(hidden_count.to_string()))
                        .tooltip(s.tree_view_options.clone()),
                )
                .content(move |_, _, cx| view_options_panel(&view, &hidden, cx))
        };
        let search_field = self.render_search_field(cx);
        let results = self.render_search_results(cx);
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
                        Button::new("tree-find-file")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Search)
                            .selected(self.file_search.is_some())
                            .tooltip_with_action(s.find_file.clone(), &FindFile, Some("Review"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_file_search(window, cx)
                            })),
                    )
                    .when(!flat, |el| {
                        el.child(
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
                        )
                    })
                    .child(view_options),
            )
            .children(search_field);

        let list = match results {
            Some(results) => results,
            None => {
                let rows = self.rows.clone();
                let selected = self.selected_row;
                let current = self.current_file;
                uniform_list(
                    "review-tree",
                    rows.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        range
                            .map(|ix| {
                                this.render_row(ix, &rows[ix], selected == Some(ix), current, cx)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.tree_scroll)
                .size_full()
                .into_any_element()
            }
        };
        let body = div()
            .id("review-tree-body")
            .key_context("ReviewTree")
            .track_focus(&self.tree_focus)
            .flex_1()
            .min_h_0()
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
            .child(list);
        v_flex()
            .id("review-tree-panel")
            .size_full()
            .min_w_0()
            .bg(theme.sidebar)
            .child(header)
            .child(body)
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
        let text_color = if row.muted {
            theme.muted_foreground.opacity(0.6)
        } else if complete {
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
            Icon::new(name).text_color(if row.muted {
                theme.muted_foreground.opacity(0.6)
            } else {
                color
            })
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
        let view = cx.entity();
        let node = row.node;
        let toggle_tooltip = if complete {
            tr().mark_unviewed.clone()
        } else {
            tr().mark_viewed.clone()
        };
        h_flex()
            .id(("tree-row", ix))
            .group("tree-row")
            .h(rems(ROW_HEIGHT))
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
                    .when(row.muted, |el| el.italic())
                    .child(row.name.clone()),
            )
            .children(row.detail.clone().map(|detail| {
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .max_w(relative(0.5))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(detail)
            }))
            .when(row.seen_before, |el| {
                el.child(self.render_seen_badge(ix, cx))
            })
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
            .context_menu(move |menu, _, cx| hide_menu(menu, &view, node, cx))
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
