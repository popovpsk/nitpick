use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffHoverHighlight},
    h_flex, h_resizable, resizable_panel,
    spinner::Spinner,
    v_flex,
};
use ui_resources::Icons;

use super::{
    Busy, ComposerTarget, FocusDiff, FocusTree, LoadState, NextFile, PreviousFile, PullSide,
    Refresh, ReviewSource, ReviewView, ToggleTree, ToggleViewed, comments,
};
use crate::localization::tr;
use crate::settings_store::{DiffLayout, TreeSide, settings};

impl ReviewView {
    fn render_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let busy = self.busy;
        let chips = match &self.source {
            ReviewSource::Branches { target, source } => Some((
                branch_chip("target", target.clone(), PullSide::Target, busy, cx),
                branch_chip("source", source.clone(), PullSide::Source, busy, cx),
            )),
            ReviewSource::MergeRequest { .. } => None,
        };
        let theme = cx.theme();
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
            .child(
                Button::new("refresh-review")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .loading(busy == Some(Busy::Refreshing))
                    .tooltip(s.refresh.clone())
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
                    .tooltip(s.toggle_tree.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_tree(cx))),
            )
            .child(
                Button::new("prev-file")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .tooltip(s.previous_file.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.step_file(-1, cx))),
            )
            .child(
                Button::new("next-file")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip(s.next_file.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.step_file(1, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{position} / {count}")),
            )
            .child(div().flex_1())
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
        let theme = cx.theme();
        let Some(review) = self.review.clone() else {
            return div().into_any_element();
        };
        if review.files.is_empty() {
            return empty_state(tr().no_changes.clone(), cx);
        }
        let view = cx.entity();
        let annotations = self.annotations.clone();
        let header_view = view.clone();
        let annotation_view = view.clone();
        let add_view = view.clone();
        let can_comment = self.mr.is_some() && review.comments_aligned;
        let viewed_bg = theme.success.opacity(0.08);
        let current_viewed = self
            .current_file
            .and_then(|f| self.viewed.get(f).copied())
            .unwrap_or(false);
        div()
            .flex_1()
            .min_h_0()
            .when(current_viewed, |el| el.bg(viewed_bg))
            .child(
                Diff::new(&self.diff)
                    .hover_highlight(DiffHoverHighlight::LineNumber)
                    .render_header_suffix(move |_, _, cx| {
                        let this = header_view.read(cx);
                        let viewed = this
                            .current_file
                            .and_then(|f| this.viewed.get(f).copied())
                            .unwrap_or(false);
                        let view = header_view.clone();
                        Checkbox::new("header-viewed")
                            .label(tr().viewed.clone())
                            .checked(viewed)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    if let Some(file) = this.current_file {
                                        this.toggle_file_viewed(file, cx);
                                    }
                                })
                            })
                    })
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
    id: &'static str,
    branch: String,
    side: PullSide,
    busy: Option<Busy>,
    cx: &mut Context<ReviewView>,
) -> AnyElement {
    let s = tr();
    let pull = Button::new(ElementId::Name(format!("pull-{id}").into()))
        .ghost()
        .xsmall()
        .icon(Icon::new(Icons::ArrowDownToLine))
        .loading(busy == Some(Busy::Pulling(side)))
        .disabled(busy.is_some())
        .tooltip(s.pull_branch(&branch))
        .on_click(cx.listener(move |this, _, window, cx| this.pull(side, window, cx)));
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .gap_0p5()
        .items_center()
        .px_1p5()
        .rounded(theme.radius)
        .bg(theme.secondary)
        .child(
            Icon::new(Icons::GitBranch)
                .xsmall()
                .text_color(theme.muted_foreground),
        )
        .child(div().text_sm().child(SharedString::from(branch)))
        .child(pull)
        .into_any_element()
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
            .on_action(cx.listener(|this, _: &ToggleViewed, _, cx| {
                if let Some(file) = this.current_file {
                    this.toggle_file_viewed(file, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusTree, window, cx| this.focus_tree(window, cx)))
            .on_action(cx.listener(|this, _: &FocusDiff, window, cx| this.focus_diff(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ToggleTree, _, cx| this.toggle_tree(cx)));
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
        let tree_panel = resizable_panel()
            .size(px(300.))
            .size_range(px(160.)..px(720.))
            .visible(self.tree_visible)
            .child(tree);
        let diff_panel = resizable_panel().child(diff);
        let body = match settings(cx).tree_side {
            TreeSide::Left => h_resizable("review-split")
                .child(tree_panel)
                .child(diff_panel),
            TreeSide::Right => h_resizable("review-split-right")
                .child(diff_panel)
                .child(tree_panel),
        };
        let _ = ComposerTarget::General;
        root.child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
