use gpui::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, CursorStyle, DragMoveEvent,
    ElementId, Empty, Entity, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, ease_out_quint, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    diff::{Diff, DiffHoverHighlight},
    h_flex,
    progress::Progress as ProgressBar,
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use ui_resources::Icons;

use super::{
    Busy, CrossHint, FocusDiff, FocusTree, LoadState, NextChange, NextFile, PreviousChange,
    PreviousFile, PullSide, Refresh, ReviewSource, ReviewView, TREE_SLIDE, ToggleTree,
    ToggleViewed, comments,
};
use crate::localization::tr;
use crate::picker::Picker;
use crate::settings_store::{DiffLayout, TreeSide, settings};

impl ReviewView {
    fn render_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let busy = self.busy;
        let chips = match &self.source {
            ReviewSource::Branches { target, source } => Some((
                branch_chip(
                    0,
                    target.clone(),
                    PullSide::Target,
                    self.branch_pickers.picker(PullSide::Target),
                    busy,
                    cx,
                ),
                branch_chip(
                    1,
                    source.clone(),
                    PullSide::Source,
                    self.branch_pickers.picker(PullSide::Source),
                    busy,
                    cx,
                ),
            )),
            ReviewSource::MergeRequest { .. } | ReviewSource::WorkingTree => None,
        };
        let theme = cx.theme();
        let (viewed, total) = self.progress();
        let progress = (total > 0).then(|| {
            let complete = viewed == total;
            h_flex()
                .id("viewed-progress")
                .flex_none()
                .gap_1p5()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(if complete {
                            theme.success
                        } else {
                            theme.muted_foreground
                        })
                        .child(format!("{viewed}/{total}")),
                )
                .child(
                    ProgressBar::new("viewed-progress-bar")
                        .xsmall()
                        .w(px(56.))
                        .value(viewed as f32 * 100. / total as f32)
                        .when(complete, |bar| bar.color(theme.success)),
                )
                .tooltip(move |window, cx| {
                    Tooltip::new(tr().viewed_progress(viewed, total)).build(window, cx)
                })
        });
        let mut left = h_flex().flex_1().min_w_0().gap_2().items_center();
        match (&self.source, &self.mr) {
            (_, Some(ctx)) => {
                let url = ctx.mr.web_url.clone();
                left = left
                    .child(
                        Icon::new(Icons::GitPullRequest)
                            .small()
                            .text_color(theme.primary),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_semibold()
                            .child(format!("!{} {}", ctx.mr.iid, ctx.mr.title)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} → {} · {}",
                                ctx.mr.source_branch, ctx.mr.target_branch, ctx.mr.author.name
                            )),
                    )
                    .child(
                        Button::new("open-mr")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ExternalLink)
                            .tooltip(s.open_in_browser.clone())
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
            }
            (ReviewSource::Branches { .. }, None) => {
                left = left.child(
                    Icon::new(Icons::GitCompare)
                        .small()
                        .text_color(theme.primary),
                );
                if let Some((target_chip, source_chip)) = chips {
                    left = left
                        .child(target_chip)
                        .child(div().text_color(theme.muted_foreground).child("←"))
                        .child(source_chip);
                }
                if let Some(review) = &self.review {
                    let base: String = review.base_sha.chars().take(8).collect();
                    left = left.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(s.merge_base(&base, review.commits.len())),
                    );
                }
            }
            (ReviewSource::WorkingTree, None) => {
                left = left
                    .child(Icon::new(Icons::FilePen).small().text_color(theme.primary))
                    .child(div().font_semibold().child(s.uncommitted_changes.clone()));
                if let Some(review) = &self.review {
                    left = left.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} · {}",
                                review.target,
                                short_sha(&review.base_sha)
                            )),
                    );
                }
            }
            (ReviewSource::MergeRequest { iid, .. }, None) => {
                left = left
                    .child(Icon::new(Icons::GitPullRequest).small())
                    .child(format!("!{iid}"));
            }
        }
        let comments_button =
            self.mr.as_ref().map(|_| {
                let count = self.general_count();
                Button::new("general-comments")
                    .ghost()
                    .small()
                    .icon(Icon::new(Icons::MessageSquareText))
                    .label(count.to_string())
                    .loading(self.discussions_loading)
                    .tooltip(s.general_comments.clone())
                    .on_click(cx.listener(|this, _, window, cx| {
                        comments::open_general_sheet(this, window, cx)
                    }))
            });
        h_flex()
            .flex_none()
            .h(px(40.))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(left)
            .when(
                self.review.as_ref().is_some_and(|r| r.context_limited),
                |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(theme.warning)
                            .child(s.context_limited.clone()),
                    )
                },
            )
            .children(comments_button)
            .children(progress)
            .child(
                Button::new("refresh-review")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .loading(busy == Some(Busy::Refreshing))
                    .tooltip_with_action(s.refresh.clone(), &Refresh, Some("Review"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .into_any_element()
    }

    fn render_file_toolbar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let theme = cx.theme();
        let layout = settings(cx).diff_layout;
        let (position, count) = match (&self.review, self.current_file) {
            (Some(review), Some(current)) => {
                let order = review.tree.file_order();
                (
                    order
                        .iter()
                        .position(|&f| f == current)
                        .map_or(0, |p| p + 1),
                    order.len(),
                )
            }
            (Some(review), None) => (0, review.files.len()),
            _ => (0, 0),
        };
        let current_viewed = self
            .current_file
            .and_then(|f| self.viewed.get(f).copied())
            .unwrap_or(false);
        let auto_mark = settings(cx).auto_mark_viewed;
        h_flex()
            .flex_none()
            .h(px(34.))
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("toggle-tree")
                    .ghost()
                    .xsmall()
                    .icon(if settings(cx).tree_side == TreeSide::Left {
                        IconName::PanelLeft
                    } else {
                        IconName::PanelRight
                    })
                    .selected(self.tree_visible)
                    .tooltip_with_action(s.toggle_tree.clone(), &ToggleTree, Some("Review"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_tree(window, cx))),
            )
            .child(
                Button::new("prev-change")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .tooltip_with_action(s.previous_change.clone(), &PreviousChange, Some("Review"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_change(false, cx))),
            )
            .child(
                Button::new("next-change")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip_with_action(s.next_change.clone(), &NextChange, Some("Review"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_change(true, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{position} / {count}")),
            )
            .children(self.cross_hint.map(|hint| {
                div()
                    .ml_2()
                    .px_1p5()
                    .rounded(theme.radius)
                    .bg(theme.accent)
                    .text_xs()
                    .text_color(theme.accent_foreground)
                    .child(match hint {
                        CrossHint::PressForNextFile => s.press_again_next_file.clone(),
                        CrossHint::PressForPreviousFile => s.press_again_previous_file.clone(),
                        CrossHint::NoNextFile => s.no_next_change.clone(),
                        CrossHint::NoPreviousFile => s.no_previous_change.clone(),
                    })
            }))
            .child(div().flex_1())
            .child(
                Button::new("auto-mark")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::CheckCheck))
                    .selected(auto_mark)
                    .tooltip(if auto_mark {
                        s.auto_mark_on.clone()
                    } else {
                        s.auto_mark_off.clone()
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_auto_mark(!auto_mark, cx)),
                    ),
            )
            .child(
                Button::new("mark-viewed")
                    .small()
                    .when(current_viewed, |b| b.ghost())
                    .when(!current_viewed, |b| b.outline())
                    .icon(Icon::new(if current_viewed {
                        Icons::Undo2
                    } else {
                        Icons::SquareCheck
                    }))
                    .label(if current_viewed {
                        s.mark_unviewed.clone()
                    } else {
                        s.mark_viewed.clone()
                    })
                    .disabled(self.current_file.is_none())
                    .tooltip_with_action(
                        if current_viewed {
                            s.mark_unviewed.clone()
                        } else {
                            s.mark_viewed.clone()
                        },
                        &ToggleViewed,
                        Some("Review"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(file) = this.current_file {
                            this.toggle_file_viewed(file, cx);
                        }
                    })),
            )
            .child(
                ButtonGroup::new("diff-layout")
                    .small()
                    .outline()
                    .child(
                        Button::new("layout-split")
                            .icon(Icon::new(Icons::Columns2))
                            .tooltip(s.split_view.clone())
                            .selected(layout == DiffLayout::Split),
                    )
                    .child(
                        Button::new("layout-unified")
                            .icon(Icon::new(Icons::Rows2))
                            .tooltip(s.unified_view.clone())
                            .selected(layout == DiffLayout::Unified),
                    )
                    .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                        let layout = if clicks.first() == Some(&1) {
                            DiffLayout::Unified
                        } else {
                            DiffLayout::Split
                        };
                        this.set_layout(layout, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_diff(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = self.review.clone() else {
            return div().into_any_element();
        };
        if review.files.is_empty() {
            return empty_state(tr().no_changes.clone(), cx);
        }
        let split = settings(cx).diff_layout == DiffLayout::Split;
        let theme = cx.theme();
        let view = cx.entity();
        let annotations = self.annotations.clone();
        let annotation_view = view.clone();
        let add_view = view.clone();
        let can_comment = self.mr.is_some() && review.comments_aligned;
        let viewed_bg = theme.success.opacity(0.08);
        let current_viewed = self
            .current_file
            .and_then(|f| self.viewed.get(f).copied())
            .unwrap_or(false);
        div()
            .id("diff-area")
            .flex_1()
            .min_h_0()
            .when(current_viewed, |el| el.bg(viewed_bg))
            .on_scroll(cx.listener(|this, _, _, cx| this.note_manual_scroll(cx)))
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.note_manual_scroll(cx)))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let keys = &event.keystroke;
                let plain = !keys.modifiers.alt
                    && !keys.modifiers.control
                    && !keys.modifiers.platform
                    && !keys.modifiers.function;
                if plain
                    && matches!(
                        keys.key.as_str(),
                        "up" | "down" | "pageup" | "pagedown" | "home" | "end"
                    )
                {
                    this.note_manual_scroll(cx);
                }
            }))
            .child(
                Diff::new(&self.diff)
                    .hover_highlight(DiffHoverHighlight::LineNumber)
                    .soft_wrap(split)
                    .annotations(annotations.iter().cloned())
                    .render_annotation(move |annotation, window, cx| {
                        comments::render_annotation(&annotation_view, annotation, window, cx)
                    })
                    .when(can_comment, |diff| {
                        diff.on_add_annotation(move |range, window, cx| {
                            add_view
                                .update(cx, |this, cx| this.open_line_composer(range, window, cx));
                        })
                    }),
            )
            .into_any_element()
    }
}

fn branch_chip(
    ix: usize,
    branch: String,
    side: PullSide,
    picker: &Entity<Picker>,
    busy: Option<Busy>,
    cx: &mut Context<ReviewView>,
) -> AnyElement {
    let s = tr();
    let pull = Button::new(("pull-branch", ix))
        .ghost()
        .xsmall()
        .icon(Icon::new(Icons::ArrowDownToLine))
        .loading(busy == Some(Busy::Pulling(side)))
        .disabled(busy.is_some())
        .tooltip(s.pull_branch(&branch))
        .on_click(cx.listener(move |this, _, window, cx| this.pull(side, window, cx)));
    let name = Button::new(("pick-branch", ix))
        .ghost()
        .small()
        .icon(Icon::new(Icons::GitBranch))
        .label(SharedString::from(branch))
        .tooltip(match side {
            PullSide::Target => s.change_target_branch.clone(),
            PullSide::Source => s.change_source_branch.clone(),
        });
    let name = picker.read(cx).popover(("branch-picker", ix), name, cx);
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .gap_0p5()
        .items_center()
        .pr_0p5()
        .rounded(theme.radius)
        .bg(theme.secondary)
        .child(name)
        .child(pull)
        .into_any_element()
}

fn short_sha(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

fn empty_state(message: SharedString, cx: &mut Context<ReviewView>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(message)
        .into_any_element()
}

#[derive(Clone)]
struct TreeResize;

impl Render for TreeResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl ReviewView {
    fn render_tree_panel(
        &self,
        tree: AnyElement,
        side: TreeSide,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let width = self.tree_width;
        let closing = self.tree_closing;
        let handle = div()
            .id("tree-resize")
            .absolute()
            .top_0()
            .h_full()
            .w(px(5.))
            .when(side == TreeSide::Left, |el| el.right(px(-2.)))
            .when(side == TreeSide::Right, |el| el.left(px(-2.)))
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|el| el.bg(theme.drag_border))
            .on_drag(TreeResize, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            });
        let panel = div()
            .relative()
            .flex_none()
            .h_full()
            .when(side == TreeSide::Left, |el| el.border_r_1())
            .when(side == TreeSide::Right, |el| el.border_l_1())
            .border_color(theme.border)
            .child(
                div()
                    .size_full()
                    .overflow_hidden()
                    .child(div().h_full().w(px(width)).child(tree)),
            )
            .when(!closing, |el| el.child(handle))
            .when(closing, |el| {
                el.child(
                    div()
                        .id("tree-closing")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude(),
                )
            });
        if self.tree_toggles == 0 {
            return panel.w(px(width)).into_any_element();
        }
        panel
            .with_animation(
                ElementId::NamedInteger("tree-slide".into(), self.tree_toggles as u64),
                Animation::new(TREE_SLIDE).with_easing(ease_out_quint()),
                move |el, delta| {
                    let factor = if closing { 1.0 - delta } else { delta };
                    el.w(px(width * factor))
                },
            )
            .into_any_element()
    }
}

impl Render for ReviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex()
            .id("review")
            .key_context("Review")
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &NextFile, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.step_file(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousFile, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.step_file(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextChange, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.step_change(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousChange, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.step_change(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleViewed, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else if let Some(file) = this.current_file {
                    this.toggle_file_viewed(file, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusTree, window, cx| this.focus_tree(window, cx)))
            .on_action(cx.listener(|this, _: &FocusDiff, window, cx| this.focus_diff(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleTree, window, cx| this.toggle_tree(window, cx)),
            );
        match &self.load_state {
            LoadState::Loading => {
                return root
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(Spinner::new().large())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr().loading_review.clone()),
                    )
                    .into_any_element();
            }
            LoadState::Failed(message) => {
                let message = message.clone();
                return root
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .p_8()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .large()
                            .text_color(cx.theme().danger),
                    )
                    .child(div().max_w(px(640.)).text_sm().child(message))
                    .child(
                        Button::new("retry-review")
                            .label(tr().retry.clone())
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }
        let header = self.render_header(cx);
        let tree = self.render_tree(window, cx).into_any_element();
        let diff = v_flex()
            .size_full()
            .min_w_0()
            .child(self.render_file_toolbar(cx))
            .child(self.render_diff(cx))
            .into_any_element();
        let side = settings(cx).tree_side;
        let tree_panel = (self.tree_visible || self.tree_closing)
            .then(|| self.render_tree_panel(tree, side, cx));
        let body = h_flex()
            .id("review-body")
            .size_full()
            .min_h_0()
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<TreeResize>, _, cx| {
                    let x = event.event.position.x;
                    let width = match side {
                        TreeSide::Left => x - event.bounds.left(),
                        TreeSide::Right => event.bounds.right() - x,
                    };
                    this.resize_tree(f32::from(width), cx);
                }),
            );
        let diff = div().flex_1().min_w_0().h_full().child(diff);
        let body = match side {
            TreeSide::Left => body.children(tree_panel).child(diff),
            TreeSide::Right => body.child(diff).children(tree_panel),
        };
        root.child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
