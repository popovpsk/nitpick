use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent, Context,
    CursorStyle, DragMoveEvent, ElementId, Empty, Entity, FocusHandle, Focusable as _,
    HighlightStyle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Window, canvas, div, ease_out_quint,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _, DropdownButton},
    diff::{
        Diff, DiffChangeIndicator, DiffGutterMarker, DiffHoverHighlight, DiffLinePosition,
        DiffMode, DiffSide,
    },
    h_flex,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    progress::Progress as ProgressBar,
    spinner::Spinner,
    tag::Tag,
    tooltip::Tooltip,
    v_flex,
};
use review_core::{FileStatus, FileTree};
use ui_resources::Icons;

use super::{
    AddComment, Busy, COMMENT_LANE, COMMENT_MARKER, CrossHint, FindFile, FindInFile, FocusDiff,
    FocusTree, LoadState, LoadedReview, NextChange, NextFile, PreviousChange, PreviousFile,
    PullSide, Refresh, ReviewSource, ReviewView, TREE_SLIDE, ToggleBlame, ToggleTree, ToggleViewed,
    comment_list, comments, file_actions,
};
use crate::localization::tr;
use crate::picker::Picker;
use crate::settings_store::{
    DiffLayout, SPLIT_MIN_COLUMN, SPLIT_RATIO_RANGE, TreeSide, WrapLines, fit_split_ratio,
    settings, update_settings,
};

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
        self.refresh_header_text();
        let commits = self.render_commits_button(cx);
        let theme = cx.theme();
        let mut left = h_flex().flex_1().min_w_0().gap_2().items_center();
        match (&self.source, &self.mr) {
            (_, Some(ctx)) => {
                let request = &ctx.request;
                let url = request.web_url.clone();
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
                            .child(format!("{} {}", request.reference(), request.title)),
                    )
                    .map(|el| match request.state {
                        forge::RequestState::Open => el,
                        forge::RequestState::Merged => el.child(
                            Tag::info()
                                .xsmall()
                                .flex_none()
                                .child(s.request_merged.clone()),
                        ),
                        forge::RequestState::Closed => el.child(
                            Tag::danger()
                                .xsmall()
                                .flex_none()
                                .child(s.request_closed.clone()),
                        ),
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} → {} · {}",
                                request.source_branch, request.target_branch, request.author.name
                            )),
                    )
                    .children(commits)
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
                left = left.children(commits);
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
            (ReviewSource::MergeRequest { forge, number, .. }, None) => {
                left = left
                    .child(Icon::new(Icons::GitPullRequest).small())
                    .child(forge.reference(*number));
            }
        }
        let approve_button = self.approval.as_ref().and_then(|approval| {
            let label = approval.label.clone()?;
            let mine = approval.approved_by_me;
            let actionable = approval.can_approve || approval.can_unapprove;
            Some(
                Button::new("approve")
                    .small()
                    .icon(Icon::new(if mine {
                        Icons::CircleCheckBig
                    } else {
                        Icons::ThumbsUp
                    }))
                    .label(label)
                    .map(|button| {
                        if mine {
                            button.success()
                        } else if approval.can_approve {
                            button.outline()
                        } else {
                            button.ghost()
                        }
                    })
                    .loading(self.approval_busy)
                    .tooltip(approval.tooltip.clone())
                    .when(actionable, |button| {
                        button.on_click(
                            cx.listener(|this, _, window, cx| this.toggle_approval(window, cx)),
                        )
                    }),
            )
        });
        let review_button = self.drafts.has_review().then(|| {
            Button::new("pending-review")
                .primary()
                .small()
                .icon(Icon::new(Icons::ClipboardCheck))
                .label(self.drafts.button_label.clone())
                .loading(self.drafts.submitting)
                .tooltip(s.review_tooltip.clone())
                .on_click(cx.listener(|this, _, window, cx| this.open_review_sheet(window, cx)))
        });
        let comments_button = self.mr.as_ref().map(|_| {
            let count = self.comments_count();
            Button::new("general-comments")
                .ghost()
                .small()
                .icon(Icon::new(Icons::MessageSquareText))
                .label(count.to_string())
                .loading(self.discussions_loading)
                .tooltip(tr().comments.clone())
                .on_click(cx.listener(|this, _, window, cx| {
                    comment_list::open_comments_sheet(this, window, cx)
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
            .children(review_button)
            .children(approve_button)
            .children(comments_button)
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

    pub(super) fn render_viewed_progress(
        &mut self,
        fit: ProgressFit,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.refresh_header_text();
        let (progress_label, progress_tooltip) = self
            .header_text
            .as_ref()
            .map(|text| (text.progress.clone(), text.progress_tooltip.clone()))
            .unwrap_or_default();
        let theme = cx.theme();
        let (viewed, total) = self.progress();
        (total > 0).then(|| {
            let complete = viewed == total;
            let view = cx.entity();
            Button::new("viewed-progress")
                .ghost()
                .small()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .when(fit.label, |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(if complete {
                                        theme.success
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(progress_label),
                            )
                        })
                        .child(
                            ProgressBar::new("viewed-progress-bar")
                                .xsmall()
                                .w(fit.bar)
                                .value(viewed as f32 * 100. / total as f32)
                                .when(complete, |bar| bar.color(theme.success)),
                        ),
                )
                .tooltip(progress_tooltip)
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                    let view = view.clone();
                    menu.item(
                        PopupMenuItem::new(tr().reset_progress_ellipsis.clone())
                            .icon(Icon::new(Icons::RotateCcw))
                            .on_click(move |_, window, cx| {
                                confirm_reset_progress(&view, window, cx)
                            }),
                    )
                })
                .into_any_element()
        })
    }

    fn refresh_header_text(&mut self) {
        let (viewed, total) = self.progress();
        let commits = self
            .review
            .as_ref()
            .map_or(0, |review| review.commits.len());
        let review = self.review.as_ref().map_or(std::ptr::null(), Rc::as_ptr);
        let key = (viewed, total, commits, review, ui_resources::i18n::active());
        if self
            .header_text
            .as_ref()
            .is_some_and(|text| text.key == key)
        {
            return;
        }
        let s = tr();
        let commits_tooltip = match (&self.source, &self.review) {
            (ReviewSource::Branches { .. }, Some(review)) => s
                .merge_base(short_sha(&review.merge_base_sha), commits)
                .into(),
            _ => s.pick_commit.clone(),
        };
        self.header_text = Some(HeaderText {
            key,
            progress: format!("{viewed}/{total}").into(),
            progress_tooltip: s.viewed_progress(viewed, total).into(),
            commits: s.commits_count(commits).into(),
            commits_tooltip,
        });
    }

    fn render_commits_button(&self, cx: &App) -> Option<AnyElement> {
        let review = self.review.as_ref()?;
        if review.commits.is_empty() && review.commit.is_none() {
            return None;
        }
        let picker = &self.commit_picker;
        let text = self.header_text.as_ref()?;
        let label: SharedString = match picker.label() {
            Some(label) => label.clone(),
            None => text.commits.clone(),
        };
        let tooltip: SharedString = match picker.tooltip() {
            Some(tooltip) => tooltip.clone(),
            None => text.commits_tooltip.clone(),
        };
        let button = Button::new("pick-commit")
            .ghost()
            .xsmall()
            .icon(Icon::new(Icons::GitCommitHorizontal))
            .label(label)
            .selected(review.commit.is_some())
            .tooltip(tooltip);
        Some(
            div()
                .flex_none()
                .child(
                    picker
                        .picker()
                        .read(cx)
                        .popover("commit-picker", button, cx),
                )
                .into_any_element(),
        )
    }

    fn refresh_file_label(&mut self) {
        let (Some(review), Some(file)) = (&self.review, self.current_file) else {
            self.file_label = None;
            return;
        };
        if self.file_label.as_ref().is_some_and(|label| {
            label.file == file
                && Rc::ptr_eq(&label.review, review)
                && Arc::ptr_eq(&label.tree, &self.tree)
                && label.nav_generation == self.nav_generation
        }) {
            return;
        }
        let Some(shown) = review.files.get(file) else {
            self.file_label = None;
            return;
        };
        let order = &self.nav_order;
        let position = order.iter().position(|&f| f == file).map_or(0, |p| p + 1);
        let name_start = shown.path.rfind('/').map_or(0, |slash| slash + 1);
        self.file_label = Some(FileLabel {
            review: review.clone(),
            tree: self.tree.clone(),
            nav_generation: self.nav_generation,
            file,
            path: shown.path.clone(),
            name_start,
            status: shown.status,
            additions: (shown.additions > 0).then(|| format!("+{}", shown.additions).into()),
            deletions: (shown.deletions > 0).then(|| format!("\u{2212}{}", shown.deletions).into()),
            position: format!("{position} / {}", order.len()).into(),
            renamed_from: (shown.status == FileStatus::Renamed && shown.old_path != shown.path)
                .then(|| shown.old_path.clone()),
            has_previous: self.visible_neighbour(file, false).is_some(),
            has_next: self.visible_neighbour(file, true).is_some(),
        });
    }

    fn render_file_label(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let label = self.file_label.as_ref()?;
        let s = tr();
        let theme = cx.theme();
        let (icon, color) = status_icon(label.status, cx);
        let tag = match label.status {
            FileStatus::Added => Some((s.file_added_tag.clone(), theme.green)),
            FileStatus::Deleted => Some((s.file_deleted_tag.clone(), theme.red)),
            FileStatus::Renamed => label
                .renamed_from
                .as_ref()
                .map(|_| (s.file_renamed_tag.clone(), theme.blue)),
            FileStatus::Modified => None,
        };
        let dir = (label.name_start > 0).then(|| {
            (
                0..label.name_start,
                HighlightStyle {
                    color: Some(theme.muted_foreground),
                    ..Default::default()
                },
            )
        });
        let full_path = label.path.clone();
        let renamed_from = label.renamed_from.clone();
        let view = cx.entity();
        let open_view = view.clone();
        Some(
            h_flex()
                .min_w_0()
                .gap_1p5()
                .items_center()
                .child(Icon::new(icon).small().text_color(color))
                .child(
                    div()
                        .id("file-path")
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_start()
                        .text_sm()
                        .font_medium()
                        .text_color(theme.foreground)
                        .child(StyledText::new(label.path.clone()).with_highlights(dir))
                        .tooltip(move |window, cx| {
                            Tooltip::new(full_path.clone()).build(window, cx)
                        })
                        .context_menu(move |menu, _, cx| {
                            file_actions::shown_file_menu(menu, &view, cx)
                        }),
                )
                .children(label.additions.clone().map(|text| {
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.success)
                        .child(text)
                }))
                .children(label.deletions.clone().map(|text| {
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(text)
                }))
                .children(tag.map(|(text, color)| {
                    div()
                        .id("file-tag")
                        .flex_none()
                        .px_1()
                        .rounded(theme.radius)
                        .bg(color.opacity(0.12))
                        .text_xs()
                        .text_color(color)
                        .child(text)
                        .when_some(renamed_from, |el, from| {
                            el.tooltip(move |window, cx| {
                                Tooltip::new(tr().renamed_from(&from)).build(window, cx)
                            })
                        })
                }))
                .child({
                    let view = open_view;
                    Button::new("open-file")
                        .ghost()
                        .xsmall()
                        .flex_none()
                        .icon(Icon::new(Icons::SquareArrowOutUpRight))
                        .tooltip(tr().open_file.clone())
                        .dropdown_menu(move |menu, _, cx| {
                            file_actions::open_file_menu(menu, &view, cx)
                        })
                })
                .into_any_element(),
        )
    }

    fn render_file_toolbar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_file_label();
        let s = tr();
        let layout = settings(cx).diff_layout;
        let any_file = !self.nav_order.is_empty();
        let (has_previous, has_next) = self
            .file_label
            .as_ref()
            .map_or((any_file, any_file), |label| {
                (label.has_previous, label.has_next)
            });
        let position = self.file_label.as_ref().map(|label| label.position.clone());
        let file_label = self.render_file_label(cx);
        let since_toggle = self.render_since_toggle(cx);
        let current_viewed = self
            .current_file
            .and_then(|f| self.viewed.get(f).copied())
            .unwrap_or(false);
        let tree_left = settings(cx).tree_side == TreeSide::Left;
        let toggle_tree = Button::new("toggle-tree")
            .ghost()
            .xsmall()
            .icon(if tree_left {
                IconName::PanelLeft
            } else {
                IconName::PanelRight
            })
            .selected(self.tree_visible)
            .tooltip_with_action(s.toggle_tree.clone(), &ToggleTree, Some("Review"))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_tree(window, cx)));
        let (toggle_left, toggle_right) = if tree_left {
            (Some(toggle_tree), None)
        } else {
            (None, Some(toggle_tree))
        };
        let theme = cx.theme();
        h_flex()
            .flex_none()
            .h(rems(TOOLBAR_HEIGHT_REMS))
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .children(toggle_left)
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
                Button::new("prev-file")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowLeft)
                    .disabled(!has_previous)
                    .tooltip_with_action(s.cmd_previous_file.clone(), &PreviousFile, Some("Review"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_file(-1, cx))),
            )
            .child(
                Button::new("next-file")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowRight)
                    .disabled(!has_next)
                    .tooltip_with_action(s.cmd_next_file.clone(), &NextFile, Some("Review"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_file(1, cx))),
            )
            .children(position.map(|position| {
                div()
                    .flex_none()
                    .px_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(position)
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .ml_1()
                    .gap_2()
                    .items_center()
                    .children(file_label)
                    .children(since_toggle)
                    .children(self.cross_hint.map(|hint| {
                        div()
                            .flex_none()
                            .px_1p5()
                            .rounded(theme.radius)
                            .bg(theme.accent)
                            .text_xs()
                            .text_color(theme.accent_foreground)
                            .child(match hint {
                                CrossHint::PressForNextFile => s.press_again_next_file.clone(),
                                CrossHint::PressForPreviousFile => {
                                    s.press_again_previous_file.clone()
                                }
                                CrossHint::NoNextFile => s.no_next_change.clone(),
                                CrossHint::NoPreviousFile => s.no_previous_change.clone(),
                            })
                    })),
            )
            .child(
                DropdownButton::new("viewed")
                    .small()
                    .map(|button| {
                        if current_viewed {
                            button.success()
                        } else {
                            button.outline()
                        }
                    })
                    .button(
                        Button::new("mark-viewed")
                            .disabled(self.current_file.is_none())
                            .icon(Icon::new(if current_viewed {
                                Icons::SquareCheck
                            } else {
                                Icons::Square
                            }))
                            .label(s.viewed_short.clone())
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
                    .menu_accessibility_label(s.viewed_options.clone())
                    .dropdown_menu(move |menu, _, cx| {
                        let s = tr();
                        let current = settings(cx);
                        let (auto_mark, advance) =
                            (current.auto_mark_viewed, current.advance_on_viewed);
                        menu.item(
                            PopupMenuItem::new(s.auto_mark_viewed.clone())
                                .checked(auto_mark)
                                .on_click(move |_, _, cx| {
                                    update_settings(cx, |s| s.auto_mark_viewed = !auto_mark)
                                }),
                        )
                        .item(
                            PopupMenuItem::new(s.advance_on_viewed.clone())
                                .checked(advance)
                                .on_click(move |_, _, cx| {
                                    update_settings(cx, |s| s.advance_on_viewed = !advance)
                                }),
                        )
                    }),
            )
            .child(self.render_blame_toggle(cx))
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
            .children(toggle_right)
            .into_any_element()
    }

    fn render_diff(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = self.review.clone() else {
            return div().into_any_element();
        };
        let diff_focus = self.diff.read(cx).focus_handle(cx);
        if review.files.is_empty() {
            let text = match self.source {
                ReviewSource::Branches { .. } => tr().no_changes.clone(),
                ReviewSource::MergeRequest { .. } => tr().no_changes_request.clone(),
                ReviewSource::WorkingTree => tr().no_changes_worktree.clone(),
            };
            return focus_holder(&diff_focus, empty_state(text, cx));
        }
        if self.nav_order.is_empty() && self.current_file.is_none() {
            return focus_holder(&diff_focus, empty_state(tr().all_files_hidden.clone(), cx));
        }
        let placeholder =
            match self.render_image_preview(shown_file(&review, self.current_file), cx) {
                Some(preview) => Some(preview),
                None if self.interdiff.as_ref().is_some_and(|d| d.is_empty()) => {
                    Some(empty_state(tr().no_changes_since_viewed.clone(), cx))
                }
                None => None,
            };
        let wrap = match settings(cx).wrap_lines {
            WrapLines::SplitOnly => settings(cx).diff_layout == DiffLayout::Split,
            WrapLines::Always => true,
            WrapLines::Never => false,
        };
        let theme = cx.theme();
        let view = cx.entity();
        let annotations = self.annotations.clone();
        let annotation_view = view.clone();
        let measure_view = view.clone();
        let can_comment = self.can_comment_lines();
        let hover_view = view.clone();
        let viewed_bg = theme.success.opacity(0.08);
        let current_viewed = self
            .current_file
            .and_then(|f| self.viewed.get(f).copied())
            .unwrap_or(false);
        let shown = self.current_file.and_then(|f| review.files.get(f));
        let whole_file = self.interdiff.is_none()
            && shown.is_some_and(|f| matches!(f.status, FileStatus::Added | FileStatus::Deleted));
        let body_width = self.split_width - 2. * DIFF_BORDER;
        let ratio = if self.split_width > 0. {
            fit_split_ratio(self.split_ratio, body_width)
        } else {
            self.split_ratio
                .clamp(SPLIT_RATIO_RANGE.0, SPLIT_RATIO_RANGE.1)
        };
        let split_mode = self.diff.read(cx).mode() == DiffMode::Split;
        let comment_lane = shown.filter(|_| can_comment).map(|file| {
            DiffGutterMarker::new(
                COMMENT_LANE,
                DiffLinePosition::new(file.path.clone(), DiffSide::Modified, 0),
            )
        });
        let comment_marker = self.comment_marker().filter(|_| can_comment).cloned();
        let marker_view = view.clone();
        let resizable = split_mode
            && shown.is_some_and(|f| !f.binary && f.additions + f.deletions > 0)
            && body_width >= 2. * SPLIT_MIN_COLUMN;
        let divider_center = DIFF_BORDER + body_width * ratio - 0.5;
        let grip = theme.drag_border;
        let dragging = self.split_drag.is_some() && cx.has_active_drag();
        let blame = if self.interdiff.is_some() {
            None
        } else {
            self.blame_marks()
        };
        let find_decorations = self.find.as_ref().map(|find| find.decorations());
        let find_bar = self.render_find_bar(cx);
        div()
            .id("diff-area")
            .relative()
            .flex_1()
            .min_h_0()
            .when(current_viewed, |el| el.bg(viewed_bg))
            .on_scroll(cx.listener(|this, _, _, cx| this.note_manual_scroll(cx)))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.leave_diff() {
                    cx.notify();
                }
            }))
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
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SplitResize>, _, cx| {
                    let Some((start_x, start_ratio)) = this.split_drag else {
                        return;
                    };
                    let width = f32::from(event.bounds.size.width) - 2. * DIFF_BORDER;
                    if width < 2. * SPLIT_MIN_COLUMN {
                        return;
                    }
                    let moved = f32::from(event.event.position.x - start_x);
                    let ratio = fit_split_ratio((width * start_ratio + moved) / width, width);
                    this.resize_split(ratio, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_split_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_split_drag(cx)),
            )
            .map(|el| match placeholder {
                Some(placeholder) => el.child(focus_holder(&diff_focus, placeholder)),
                None => el.child(
                    Diff::new(&self.diff)
                        .header_visible(false)
                        .hover_highlight(DiffHoverHighlight::LineNumber)
                        .soft_wrap(wrap)
                        .when(split_mode, |diff| diff.split_ratio(ratio))
                        .change_background(!whole_file)
                        .when(whole_file, |diff| {
                            diff.change_indicator(DiffChangeIndicator::Bars)
                        })
                        .annotations(annotations.iter().cloned())
                        .render_annotation(move |annotation, window, cx| {
                            comments::render_annotation(&annotation_view, annotation, window, cx)
                        })
                        .when(can_comment, |diff| {
                            diff.on_line_hover(move |position, _, cx| {
                                hover_view.update(cx, |this, cx| {
                                    if this.set_hovered_line(position) {
                                        cx.notify();
                                    }
                                })
                            })
                        })
                        .when_some(find_decorations, |diff, decorations| {
                            diff.decorations(decorations.iter().cloned())
                        })
                        .when(blame.is_some() || comment_lane.is_some(), |diff| {
                            let shown_blame = blame.clone();
                            let blame_markers = blame
                                .as_ref()
                                .map(|marks| marks.markers(split_mode))
                                .unwrap_or_default();
                            diff.gutter_markers(
                                blame_markers
                                    .iter()
                                    .cloned()
                                    .chain(comment_marker)
                                    .chain(comment_lane),
                            )
                            .render_gutter_marker(
                                move |marker, window, cx| match (&shown_blame, marker.id()) {
                                    (_, ElementId::Name(name))
                                        if name.as_ref() == COMMENT_MARKER =>
                                    {
                                        let (hot, lane) =
                                            comment_hot_zone(marker, split_mode, window, cx);
                                        comments::render_comment_marker(
                                            &marker_view,
                                            marker,
                                            hot,
                                            lane,
                                            cx,
                                        )
                                    }
                                    (Some(marks), _) => marks.render_chip(marker, window, cx),
                                    (None, _) => Empty.into_any_element(),
                                },
                            )
                        }),
                ),
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        measure_view.update(cx, |this, cx| {
                            if (this.split_width - width).abs() >= 0.5 {
                                this.split_width = width;
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(resizable, |el| {
                el.child(
                    div()
                        .id("split-resize")
                        .group("split-resize")
                        .absolute()
                        .top_0()
                        .h_full()
                        .left(px(divider_center - SPLIT_HANDLE / 2.))
                        .w(px(SPLIT_HANDLE))
                        .flex()
                        .justify_center()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .block_mouse_except_scroll()
                        .child(
                            div()
                                .w(px(SPLIT_GRIP))
                                .h_full()
                                .when(dragging, |el| el.bg(grip))
                                .group_hover("split-resize", move |style| style.bg(grip)),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.split_drag = Some((event.position.x, ratio));
                                cx.notify();
                            }),
                        )
                        .on_drag(SplitResize, |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        })
                        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                            if event.click_count() == 2 {
                                this.resize_split(0.5, cx);
                            }
                        })),
                )
            })
            .children(find_bar)
            .into_any_element()
    }

    fn end_split_drag(&mut self, cx: &mut Context<Self>) {
        if self.split_drag.take().is_some() {
            cx.notify();
        }
    }
}

pub(super) const TOOLBAR_HEIGHT_REMS: f32 = 2.125;
const DIFF_BORDER: f32 = 1.;
const GUTTER_DIGITS: f32 = 5.;
const SPLIT_HANDLE: f32 = 8.;
const SPLIT_GRIP: f32 = 2.;

pub(super) struct HeaderText {
    key: (
        usize,
        usize,
        usize,
        *const LoadedReview,
        ui_resources::i18n::Lang,
    ),
    progress: SharedString,
    progress_tooltip: SharedString,
    commits: SharedString,
    commits_tooltip: SharedString,
}

pub(super) struct FileLabel {
    nav_generation: u64,
    review: Rc<LoadedReview>,
    tree: Arc<FileTree>,
    file: usize,
    path: SharedString,
    name_start: usize,
    status: FileStatus,
    additions: Option<SharedString>,
    deletions: Option<SharedString>,
    position: SharedString,
    renamed_from: Option<SharedString>,
    has_previous: bool,
    has_next: bool,
}

pub(super) fn status_icon(status: FileStatus, cx: &App) -> (Icons, Hsla) {
    let theme = cx.theme();
    match status {
        FileStatus::Added => (Icons::FilePlus, theme.green),
        FileStatus::Deleted => (Icons::FileMinus, theme.red),
        FileStatus::Renamed => (Icons::FileDiff, theme.blue),
        FileStatus::Modified => (Icons::FileDiff, theme.yellow),
    }
}

#[derive(Clone)]
struct SplitResize;

impl Render for SplitResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
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

#[derive(Clone, Copy)]
pub(super) struct ProgressFit {
    pub label: bool,
    pub bar: Pixels,
}

const PROGRESS_BAR_REMS: f32 = 3.5;
const PROGRESS_BAR_MIN_REMS: f32 = 0.75;
const TREE_HEADER_FIXED_REMS: f32 = 7.25;
const PROGRESS_BUTTON_PADDING_REMS: f32 = 1.0;
const PROGRESS_LABEL_CHAR_REMS: f32 = 0.45;
const PROGRESS_LABEL_GAP_REMS: f32 = 0.375;

impl ProgressFit {
    pub(super) fn for_width(width: Pixels, rem: Pixels, label_chars: usize) -> Self {
        let full = rem * PROGRESS_BAR_REMS;
        let room = width - rem * (TREE_HEADER_FIXED_REMS + PROGRESS_BUTTON_PADDING_REMS);
        let label = rem * (PROGRESS_LABEL_CHAR_REMS * label_chars as f32 + PROGRESS_LABEL_GAP_REMS);
        if room - label >= full {
            return Self {
                label: true,
                bar: full,
            };
        }
        Self {
            label: false,
            bar: room.min(full).max(rem * PROGRESS_BAR_MIN_REMS),
        }
    }
}

fn confirm_reset_progress(view: &Entity<ReviewView>, window: &mut Window, cx: &mut App) {
    let viewed = view.read(cx).viewed_count();
    if viewed == 0 {
        return;
    }
    let view = view.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let s = tr();
        let view = view.clone();
        alert
            .title(s.reset_progress_title.clone())
            .description(s.reset_progress_desc(viewed))
            .ok_text(s.reset_progress.clone())
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                view.update(cx, |this, cx| this.reset_progress(cx));
                true
            })
    });
}

fn short_sha(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

fn shown_file(review: &LoadedReview, file: Option<usize>) -> Option<&super::model::ReviewFile> {
    file.and_then(|ix| review.files.get(ix))
}

fn focus_holder(focus: &FocusHandle, content: AnyElement) -> AnyElement {
    div()
        .id("diff-focus-holder")
        .track_focus(focus)
        .size_full()
        .child(content)
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
            .on_action(cx.listener(|this, _: &FocusTree, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.focus_tree(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusDiff, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.focus_diff(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.refresh(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTree, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.toggle_tree(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FindFile, window, cx| {
                this.open_file_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInFile, window, cx| {
                this.open_find(window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddComment, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.comment_here(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleBlame, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.toggle_blame(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &file_actions::CopyPath, window, cx| {
                if window.has_focused_input(cx) {
                    cx.propagate();
                } else {
                    this.copy_shown_path(cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &file_actions::OpenInEditor, window, cx| {
                    if window.has_focused_input(cx) {
                        cx.propagate();
                    } else {
                        this.open_shown_in_editor(cx);
                    }
                }),
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
        let banner = self.render_push_banner(cx);
        root.child(header)
            .children(banner)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "diff_panel_tests.rs"]
mod tests;

fn comment_hot_zone(
    marker: &DiffGutterMarker,
    split_mode: bool,
    window: &Window,
    cx: &App,
) -> (Pixels, Pixels) {
    let rem = window.rem_size();
    let font = rem * (f32::from(cx.theme().mono_font_size) / 16.);
    let lane = font * 1.6;
    let digits = marker.position().line().max(1).ilog10() as f32 + 1.;
    let gutter = font * 0.7 * digits.max(GUTTER_DIGITS) + rem;
    let gutters = if split_mode { 1. } else { 2. };
    (rem / 2. + gutter * gutters + lane, lane)
}
