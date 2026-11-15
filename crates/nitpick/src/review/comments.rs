use gpui::{
    AnyElement, App, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
    rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    diff::DiffAnnotation,
    h_flex,
    input::Textarea,
    tag::Tag,
    text::TextView,
    v_flex,
};
use ui_resources::Icons;

use super::{Busy, ComposerTarget, DiscussionView, NoteView, ReviewView, avatars, note_actions};
use crate::localization::tr;

const COLLAPSED_REMS: f32 = 14.;

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

pub(super) fn render_discussion(
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
        .children(
            discussion
                .notes
                .iter()
                .map(|note| render_note(view, &discussion.id, note, cx)),
        )
        .when(replying, |el| el.child(render_composer(view, cx)))
        .child(
            h_flex()
                .gap_1()
                .when(!replying && discussion.can_reply, |el| {
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
                .when(discussion.resolve_id.is_some(), |el| {
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
                                    if let Some(id) =
                                        this.discussions.get(ix).and_then(|d| d.resolve_id.clone())
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

fn render_note(
    view: &Entity<ReviewView>,
    discussion_id: &str,
    note: &NoteView,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let this = view.read(cx);
    if let Some(edit) = this.editing.as_ref().filter(|e| e.note_id == note.id) {
        return render_note_editor(view, note, edit.input.clone(), this.note_busy, cx);
    }
    let collapsed = note.long && !this.expanded_notes.contains(&note.id);
    let actions = note.own.then(|| {
        let s = tr();
        let id = note.id;
        let edit_view = view.clone();
        let delete_view = view.clone();
        let edit_discussion = discussion_id.to_string();
        let delete_discussion = discussion_id.to_string();
        let body = note.body.clone();
        h_flex()
            .flex_none()
            .gap_0p5()
            .invisible()
            .group_hover("note", |style| style.visible())
            .child(
                Button::new(ElementId::NamedInteger("note-edit".into(), id))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::Pencil))
                    .tooltip(s.edit_comment.clone())
                    .on_click(move |_, window, cx| {
                        let discussion = edit_discussion.clone();
                        let body = body.clone();
                        edit_view.update(cx, |this, cx| {
                            this.start_edit(discussion, id, body, window, cx)
                        });
                    }),
            )
            .child(
                Button::new(ElementId::NamedInteger("note-delete".into(), id))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::Trash))
                    .tooltip(s.delete_comment.clone())
                    .on_click(move |_, window, cx| {
                        note_actions::confirm_delete(
                            &delete_view,
                            delete_discussion.clone(),
                            id,
                            window,
                            cx,
                        );
                    }),
            )
    });
    let body = div().text_sm().child(TextView::markdown(
        ElementId::NamedInteger("note".into(), note.id),
        note.body.clone(),
    ));
    let body = if collapsed {
        div()
            .relative()
            .max_h(rems(COLLAPSED_REMS))
            .overflow_hidden()
            .child(body)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(32.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(theme.popover.opacity(0.), 0.),
                        linear_color_stop(theme.popover, 1.),
                    )),
            )
            .into_any_element()
    } else {
        body.into_any_element()
    };
    let toggle = note.long.then(|| {
        let toggle_view = view.clone();
        let id = note.id;
        Button::new(ElementId::NamedInteger("note-toggle".into(), id))
            .ghost()
            .xsmall()
            .icon(if collapsed {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            })
            .label(if collapsed {
                tr().show_more.clone()
            } else {
                tr().show_less.clone()
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                toggle_view.update(cx, |this, cx| this.toggle_note(id, cx));
            })
    });
    v_flex()
        .group("note")
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(avatars::render(
                    note.avatar.as_ref(),
                    &note.initials,
                    note.tint,
                    px(20.),
                    cx,
                ))
                .child(div().text_sm().font_semibold().child(note.author.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note.meta.clone()),
                )
                .children(actions),
        )
        .child(body)
        .children(toggle)
        .into_any_element()
}

fn render_note_editor(
    view: &Entity<ReviewView>,
    note: &NoteView,
    input: Entity<gpui_component::input::TextareaState>,
    busy: bool,
    cx: &App,
) -> AnyElement {
    let s = tr();
    let cancel_view = view.clone();
    let save_view = view.clone();
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(avatars::render(
                    note.avatar.as_ref(),
                    &note.initials,
                    note.tint,
                    px(20.),
                    cx,
                ))
                .child(div().text_sm().font_semibold().child(note.author.clone())),
        )
        .child(Textarea::new(&input))
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new(ElementId::NamedInteger("note-edit-cancel".into(), note.id))
                        .ghost()
                        .small()
                        .label(s.cancel.clone())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |this, cx| this.cancel_edit(cx))
                        }),
                )
                .child(
                    Button::new(ElementId::NamedInteger("note-edit-save".into(), note.id))
                        .primary()
                        .small()
                        .label(s.save.clone())
                        .loading(busy)
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            save_view.update(cx, |this, cx| this.save_edit(window, cx))
                        }),
                ),
        )
        .into_any_element()
}

pub(super) fn render_composer(view: &Entity<ReviewView>, cx: &App) -> AnyElement {
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
