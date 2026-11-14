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

use super::{Busy, ComposerTarget, DiscussionView, NoteView, ReviewView};
use crate::localization::tr;

pub fn render_annotation(
    view: &Entity<ReviewView>,
    annotation: &DiffAnnotation,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match annotation.id() {
        ElementId::NamedInteger(name, ix) if name.as_ref() == "discussion" => {
            let discussions = view.read(cx).discussions();
            match discussions.get(*ix as usize) {
                Some(discussion) => div()
                    .px_3()
                    .py_1p5()
                    .child(render_discussion(view, *ix as usize, discussion, cx))
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
    ix: usize,
    discussion: &DiscussionView,
    cx: &App,
) -> AnyElement {
    let s = tr();
    let theme = cx.theme();
    let resolved = discussion.resolved;
    let replying = matches!(
        &view.read(cx).composer,
        Some(c) if matches!(&c.target, ComposerTarget::Reply(id) if *id == discussion.id)
    );
    let reply_view = view.clone();
    let resolve_view = view.clone();
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
        .children(discussion.notes.iter().map(|note| render_note(note, cx)))
        .when(replying, |el| el.child(render_composer(view, cx)))
        .child(
            h_flex()
                .gap_1()
                .when(!replying, |el| {
                    el.child(
                        Button::new(ElementId::NamedInteger("reply".into(), ix as u64))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::Reply))
                            .label(s.reply.clone())
                            .on_click(move |_, window, cx| {
                                reply_view.update(cx, |this, cx| {
                                    if let Some(id) = this.discussions.get(ix).map(|d| d.id.clone())
                                    {
                                        this.open_composer(ComposerTarget::Reply(id), window, cx)
                                    }
                                });
                            }),
                    )
                })
                .when(discussion.resolvable, |el| {
                    el.child(
                        Button::new(ElementId::NamedInteger("resolve".into(), ix as u64))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::CircleCheckBig))
                            .label(if resolved {
                                s.unresolve.clone()
                            } else {
                                s.resolve.clone()
                            })
                            .on_click(move |_, window, cx| {
                                resolve_view.update(cx, |this, cx| {
                                    if let Some(id) = this.discussions.get(ix).map(|d| d.id.clone())
                                    {
                                        this.set_resolved(id, !resolved, window, cx)
                                    }
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

fn render_note(note: &NoteView, cx: &App) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_baseline()
                .child(div().text_sm().font_semibold().child(note.author.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note.meta.clone()),
                ),
        )
        .child(div().text_sm().child(TextView::markdown(
            ElementId::NamedInteger("note".into(), note.id),
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
                        .disabled(posting)
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
    _this: &mut ReviewView,
    window: &mut Window,
    cx: &mut Context<ReviewView>,
) {
    let view = cx.entity();
    window.open_sheet(cx, move |sheet, _window, cx| {
        let s = tr();
        let review = view.read(cx);
        let composing = matches!(
            review.composer.as_ref().map(|c| &c.target),
            Some(ComposerTarget::General)
        );
        let (discussions, general) = review.general_discussions();
        let empty = general.is_empty();
        let mut list = v_flex().gap_3();
        for &ix in general {
            let Some(discussion) = discussions.get(ix) else {
                continue;
            };
            list = list.child(
                v_flex()
                    .gap_1()
                    .when_some(discussion.location.clone(), |el, location| {
                        el.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(Tag::warning().small().child(s.outdated.clone()))
                                .child(location),
                        )
                    })
                    .child(render_discussion(&view, ix, discussion, cx)),
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
