use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    tag::Tag,
    text::TextView,
    v_flex,
};
use review_core::FileStatus;
use ui_resources::Icons;

use super::comments::{render_composer, render_discussion};
use super::model::LoadedReview;
use super::{ComposerTarget, DiscussionView, ReviewView};
use crate::localization::tr;

const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentsTab {
    Description,
    #[default]
    Code,
    General,
}

#[derive(Clone)]
pub struct CodeThread {
    pub discussion: usize,
    pub file: Option<usize>,
    pub status: Option<FileStatus>,
    pub path: SharedString,
    pub line_label: Option<SharedString>,
    pub placed: bool,
    pub outdated: bool,
    pub resolved: bool,
    pub first_in_file: bool,
    pub first_in_section: bool,
}

pub struct FilePaths<'a> {
    pub path: &'a SharedString,
    pub old_path: &'a str,
    pub new_path: &'a str,
    pub status: FileStatus,
}

impl<'a> FilePaths<'a> {
    pub fn of(review: &'a LoadedReview) -> Vec<FilePaths<'a>> {
        review
            .files
            .iter()
            .map(|f| FilePaths {
                path: &f.path,
                old_path: &f.old_path,
                new_path: &f.new_path,
                status: f.status,
            })
            .collect()
    }
}

pub fn build_code_threads(
    discussions: &[DiscussionView],
    files: &[FilePaths],
    tree_order: &[usize],
    by_file: &HashMap<SharedString, Vec<usize>>,
    head_sha: Option<&str>,
) -> Vec<CodeThread> {
    let mut rank = vec![usize::MAX; files.len()];
    for (pos, &file) in tree_order.iter().enumerate() {
        if let Some(slot) = rank.get_mut(file) {
            *slot = pos;
        }
    }
    let placed: HashSet<usize> = by_file.values().flatten().copied().collect();
    let mut threads: Vec<(usize, u32, CodeThread)> = discussions
        .iter()
        .enumerate()
        .filter_map(|(ix, discussion)| {
            let position = discussion.position.as_ref()?;
            let path = position
                .new_path
                .as_deref()
                .or(position.old_path.as_deref())?;
            let file = files
                .iter()
                .position(|f| f.new_path == path || f.old_path == path);
            let line = position.new_line.or(position.old_line);
            let thread = CodeThread {
                discussion: ix,
                file,
                status: file.map(|f| files[f].status),
                path: file
                    .map(|f| files[f].path.clone())
                    .unwrap_or_else(|| SharedString::from(path.to_string())),
                line_label: line.map(|line| SharedString::from(format!("L{line}"))),
                placed: placed.contains(&ix),
                outdated: head_sha.is_some_and(|head| {
                    position.head_sha.as_deref().is_some_and(|sha| sha != head)
                }),
                resolved: discussion.resolved,
                first_in_file: false,
                first_in_section: false,
            };
            let order = file
                .and_then(|f| rank.get(f).copied())
                .unwrap_or(usize::MAX);
            Some((order, line.unwrap_or(0), thread))
        })
        .collect();
    threads.sort_by(|a, b| {
        (a.2.resolved, a.0, &a.2.path, a.1, a.2.discussion).cmp(&(
            b.2.resolved,
            b.0,
            &b.2.path,
            b.1,
            b.2.discussion,
        ))
    });
    let mut previous: Option<(bool, SharedString)> = None;
    threads
        .into_iter()
        .map(|(_, _, mut thread)| {
            thread.first_in_section = previous.as_ref().is_none_or(|p| p.0 != thread.resolved);
            thread.first_in_file =
                thread.first_in_section || previous.as_ref().is_some_and(|p| p.1 != thread.path);
            previous = Some((thread.resolved, thread.path.clone()));
            thread
        })
        .collect()
}

pub fn preview(body: &str) -> SharedString {
    let line = body
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '>', '-', '*', ' ']))
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut text: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        text.push('…');
    }
    text.into()
}

pub fn open_comments_sheet(
    this: &mut ReviewView,
    window: &mut Window,
    cx: &mut Context<ReviewView>,
) {
    if this.code_threads.is_empty() {
        if !this.general_only.is_empty() {
            this.comments_tab = CommentsTab::General;
        } else if this.description.is_some() {
            this.comments_tab = CommentsTab::Description;
        }
    }
    let view = cx.entity();
    window.open_sheet(cx, move |sheet, _window, cx| {
        let review = view.read(cx);
        let title = tr().comments.clone();
        let description = review.description.clone();
        let tabs_shown: &[CommentsTab] = if description.is_some() {
            &[
                CommentsTab::Description,
                CommentsTab::Code,
                CommentsTab::General,
            ]
        } else {
            &[CommentsTab::Code, CommentsTab::General]
        };
        let tab = match review.comments_tab {
            CommentsTab::Description if description.is_none() => CommentsTab::Code,
            tab => tab,
        };
        let tabs = tabs_shown
            .iter()
            .fold(
                TabBar::new("comments-tabs").segmented().small(),
                |bar, shown| {
                    bar.child(Tab::new().label(match shown {
                        CommentsTab::Description => tr().mr_description.clone(),
                        CommentsTab::Code => review.comment_tab_labels.0.clone(),
                        CommentsTab::General => review.comment_tab_labels.1.clone(),
                    }))
                },
            )
            .selected_index(tabs_shown.iter().position(|t| *t == tab).unwrap_or(0))
            .on_click({
                let view = view.clone();
                move |ix, _, cx| {
                    view.update(cx, |this, cx| {
                        if let Some(tab) = tabs_shown.get(*ix) {
                            this.comments_tab = *tab;
                        }
                        cx.notify();
                    })
                }
            });
        let body = match (tab, description) {
            (CommentsTab::Description, Some(text)) => div()
                .text_sm()
                .pb_4()
                .child(TextView::markdown("mr-description", text))
                .into_any_element(),
            (CommentsTab::General, _) => render_general_list(&view, cx),
            _ => render_code_list(&view, cx),
        };
        sheet.title(title).size(px(560.)).child(
            v_flex().size_full().gap_3().child(tabs).child(
                div()
                    .id("comments-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            ),
        )
    });
}

fn render_code_list(view: &Entity<ReviewView>, cx: &App) -> AnyElement {
    let s = tr();
    let review = view.read(cx);
    let threads = review.code_threads.clone();
    let discussions = review.discussions.clone();
    let (open_label, resolved_label) = review.code_section_labels.clone();
    if threads.is_empty() {
        return v_flex()
            .gap_1()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(s.no_code_comments.clone())
            .child(div().text_xs().child(s.line_comment_hint.clone()))
            .into_any_element();
    }
    let theme = cx.theme();
    let mut list = v_flex().gap_2().pb_4();
    for (row, thread) in threads.iter().enumerate() {
        let Some(discussion) = discussions.get(thread.discussion) else {
            continue;
        };
        if thread.first_in_section {
            list = list.child(
                div()
                    .when(row > 0, |el| el.pt_3())
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(if thread.resolved {
                        resolved_label.clone()
                    } else {
                        open_label.clone()
                    }),
            );
        }
        if thread.first_in_file {
            list = list.child(
                h_flex()
                    .pt_1()
                    .gap_1p5()
                    .text_sm()
                    .child(
                        Icon::new(status_icon(thread.status))
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(thread.path.clone()),
                    ),
            );
        }
        let ix = thread.discussion;
        let jump_view = view.clone();
        list = list.child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1p5()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .when_some(thread.line_label.clone(), |el, line| el.child(line))
                        .when(thread.outdated, |el| {
                            el.child(Tag::warning().xsmall().child(s.outdated.clone()))
                        })
                        .child(div().flex_1())
                        .when(thread.file.is_some(), |el| {
                            el.child(
                                Button::new(ElementId::NamedInteger(
                                    "go-to-code".into(),
                                    ix as u64,
                                ))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(Icons::ArrowUpRight))
                                .label(s.go_to_code.clone())
                                .on_click(move |_, window, cx| {
                                    window.close_sheet(cx);
                                    jump_view.update(cx, |this, cx| {
                                        this.reveal_discussion(ix, window, cx)
                                    });
                                }),
                            )
                        }),
                )
                .child(render_discussion(view, ix, discussion, cx)),
        );
    }
    list.into_any_element()
}

fn status_icon(status: Option<FileStatus>) -> Icons {
    match status {
        Some(FileStatus::Added) => Icons::FilePlus,
        Some(FileStatus::Deleted) => Icons::FileMinus,
        _ => Icons::FileDiff,
    }
}

fn render_general_list(view: &Entity<ReviewView>, cx: &App) -> AnyElement {
    let s = tr();
    let review = view.read(cx);
    let composing = matches!(
        review.composer.as_ref().map(|c| &c.target),
        Some(ComposerTarget::General)
    );
    let discussions = review.discussions.clone();
    let general = review.general_only.clone();
    let new_view = view.clone();
    v_flex()
        .gap_3()
        .pb_4()
        .when(composing, |el| el.child(render_composer(view, cx)))
        .when(!composing, |el| {
            el.child(
                Button::new("new-general-comment")
                    .outline()
                    .small()
                    .icon(Icon::new(Icons::MessageSquarePlus))
                    .label(s.new_comment.clone())
                    .on_click(move |_, window, cx| {
                        new_view.update(cx, |this, cx| {
                            this.open_composer(ComposerTarget::General, window, cx)
                        })
                    }),
            )
        })
        .when(general.is_empty(), |el| {
            el.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(s.no_comments.clone()),
            )
        })
        .children(general.iter().filter_map(|&ix| {
            discussions
                .get(ix)
                .map(|discussion| render_discussion(view, ix, discussion, cx))
        }))
        .into_any_element()
}

#[cfg(test)]
#[path = "comment_list_tests.rs"]
mod tests;
