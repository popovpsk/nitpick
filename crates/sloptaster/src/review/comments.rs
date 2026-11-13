use gpui::{
    AnyElement, App, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    diff::DiffAnnotation,
    h_flex,
    input::Textarea,
    tag::Tag,
    text::TextView,
    v_flex,
};
use ui_resources::Icons;

use super::{Busy, ComposerTarget, ReviewView};
use crate::localization::tr;

pub fn render_annotation(
    view: &Entity<ReviewView>,
    annotation: &DiffAnnotation,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match annotation.id() {
        ElementId::NamedInteger(name, ix) if name.as_ref() == "discussion" => {
            let discussion = view.read(cx).discussions.get(*ix as usize).cloned();
            match discussion {
                Some(discussion) => div()
                    .px_3()
                    .py_1p5()
                    .child(render_discussion(view, &discussion, window, cx))
                    .into_any_element(),
                None => div().into_any_element(),
            }
        }
        ElementId::NamedInteger(name, _) if name.as_ref() == "draft" => div()
            .px_3()
            .py_1p5()
            .child(render_composer(view, cx))
            .into_any_element(),
        _ => div().into_any_element(),
    }
}

fn render_discussion(
    view: &Entity<ReviewView>,
    discussion: &gitlab::Discussion,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let s = tr();
    let theme = cx.theme();
    let resolvable = discussion.is_resolvable();
    let resolved = discussion.is_resolved();
    let replying = matches!(
        &view.read(cx).composer,
        Some(c) if matches!(&c.target, ComposerTarget::Reply(id) if *id == discussion.id)
    );
    let notes = discussion
        .notes
        .iter()
        .filter(|n| !n.system)
        .map(|note| render_note(note, cx))
        .collect::<Vec<_>>();
    let id = discussion.id.clone();
    let reply_view = view.clone();
    let resolve_view = view.clone();
    let resolve_id = discussion.id.clone();
    v_flex()
        .max_w(px(820.))
        .gap_2()
        .p_2()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(if resolved {
            theme.border
        } else {
            theme.warning.opacity(0.5)
        })
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .children(notes)
        .when(replying, |el| el.child(render_composer(view, cx)))
        .child(
            h_flex()
                .gap_1()
                .when(!replying, |el| {
                    el.child(
                        Button::new(ElementId::Name(format!("reply-{id}").into()))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::Reply))
                            .label(s.reply.clone())
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                reply_view.update(cx, |this, cx| {
                                    this.open_composer(ComposerTarget::Reply(id), window, cx)
                                });
                            }),
                    )
                })
                .when(resolvable, |el| {
                    el.child(
                        Button::new(ElementId::Name(format!("resolve-{resolve_id}").into()))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::CircleCheckBig))
                            .label(if resolved {
                                s.unresolve.clone()
                            } else {
                                s.resolve.clone()
                            })
                            .on_click(move |_, window, cx| {
                                let id = resolve_id.clone();
                                resolve_view.update(cx, |this, cx| {
                                    this.set_resolved(id, !resolved, window, cx)
                                });
                            }),
                    )
                })
                .when(resolved, |el| {
                    el.child(Tag::success().small().child(s.resolved.clone()))
                }),
        )
        .into_any_element()
}

fn render_note(note: &gitlab::Note, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let time = note
        .created_at
        .get(..16)
        .unwrap_or(&note.created_at)
        .replace('T', " ");
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_baseline()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(note.author.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("@{} · {time}", note.author.username)),
                ),
        )
        .child(div().text_sm().child(TextView::markdown(
            ElementId::Name(format!("note-{}", note.id).into()),
            note.body.clone(),
        )))
        .into_any_element()
}

fn render_composer(view: &Entity<ReviewView>, cx: &App) -> AnyElement {
    let s = tr();
    let this = view.read(cx);
    let Some(composer) = this.composer.as_ref() else {
        return div().into_any_element();
    };
    let posting = this.busy == Some(Busy::Posting);
    let input = composer.input.clone();
    let cancel_view = view.clone();
    let submit_view = view.clone();
    v_flex()
        .max_w(px(820.))
        .gap_2()
        .p_2()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().primary.opacity(0.6))
        .bg(cx.theme().popover)
        .child(Textarea::new(&input))
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(s.comment_hint.clone()),
                )
                .child(
                    Button::new("composer-cancel")
                        .ghost()
                        .small()
                        .label(s.cancel.clone())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |this, cx| this.cancel_composer(cx))
                        }),
                )
                .child(
                    Button::new("composer-submit")
                        .primary()
                        .small()
                        .icon(Icon::new(Icons::Send))
                        .label(s.comment.clone())
                        .loading(posting)
                        .disabled(posting)
                        .on_click(move |_, window, cx| {
                            submit_view.update(cx, |this, cx| this.submit_composer(window, cx))
                        }),
                ),
        )
        .into_any_element()
}

pub fn open_general_sheet(
    this: &mut ReviewView,
    window: &mut Window,
    cx: &mut Context<ReviewView>,
) {
    let view = cx.entity();
    let _ = this;
    window.open_sheet(cx, move |sheet, window, cx| {
        let s = tr();
        let review = view.read(cx);
        let composing = matches!(
            review.composer.as_ref().map(|c| &c.target),
            Some(ComposerTarget::General)
        );
        let discussions: Vec<gitlab::Discussion> = review.general_discussions().cloned().collect();
        let empty = discussions.is_empty();
        let mut list = v_flex().gap_3();
        for discussion in &discussions {
            let outdated = discussion.position().is_some();
            list = list.child(
                v_flex()
                    .gap_1()
                    .when(outdated, |el| {
                        el.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(Tag::warning().small().child(s.outdated.clone()))
                                .children(discussion.position().and_then(|p| {
                                    p.new_path.clone().or_else(|| p.old_path.clone())
                                })),
                        )
                    })
                    .child(render_discussion(&view, discussion, window, cx)),
            );
        }
        let new_view = view.clone();
        sheet
            .title(s.general_comments.clone())
            .size(px(520.))
            .child(
                div()
                    .id("general-comments")
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .gap_3()
                            .pb_4()
                            .when(composing, |el| el.child(render_composer(&view, cx)))
                            .when(!composing, |el| {
                                el.child(
                                    Button::new("new-general-comment")
                                        .outline()
                                        .small()
                                        .icon(Icon::new(Icons::MessageSquarePlus))
                                        .label(s.new_comment.clone())
                                        .on_click(move |_, window, cx| {
                                            new_view.update(cx, |this, cx| {
                                                this.open_composer(
                                                    ComposerTarget::General,
                                                    window,
                                                    cx,
                                                )
                                            })
                                        }),
                                )
                            })
                            .when(empty, |el| {
                                el.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(s.no_comments.clone()),
                                )
                            })
                            .child(list),
                    ),
            )
    });
}
