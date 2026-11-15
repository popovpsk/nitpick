use gpui::{App, AppContext as _, Context, Entity, SharedString, Subscription, Window};
use gpui_component::{
    WindowExt as _,
    button::ButtonVariant,
    input::{InputEvent, TextareaState},
    notification::Notification,
};

use super::ReviewView;
use crate::localization::tr;

pub struct NoteEdit {
    pub discussion_id: String,
    pub note_id: u64,
    pub input: Entity<TextareaState>,
    _submit: Subscription,
}

pub struct ApprovalView {
    pub approved_by_me: bool,
    pub can_approve: bool,
    pub can_unapprove: bool,
    pub label: Option<SharedString>,
    pub tooltip: SharedString,
}

impl ApprovalView {
    fn new(approval: &forge::Approval) -> Self {
        let s = tr();
        let names: Vec<&str> = approval
            .approvers
            .iter()
            .map(|user| user.name.as_str())
            .collect();
        let mut tooltip = if names.is_empty() {
            s.no_approvals.to_string()
        } else {
            s.approved_by(&names.join(", "))
        };
        if approval.can_unapprove {
            tooltip.push_str(" · ");
            tooltip.push_str(&s.withdraw_approval);
        }
        Self {
            approved_by_me: approval.approved_by_me,
            can_approve: approval.can_approve,
            can_unapprove: approval.can_unapprove,
            label: approval_label(approval),
            tooltip: tooltip.into(),
        }
    }
}

fn approval_label(approval: &forge::Approval) -> Option<SharedString> {
    let s = tr();
    let count = approval.approvers.len();
    let label = if approval.approved_by_me {
        s.approved.to_string()
    } else if approval.can_approve && count > 0 {
        format!("{} · {count}", s.approve)
    } else if approval.can_approve {
        s.approve.to_string()
    } else if count > 0 {
        count.to_string()
    } else {
        return None;
    };
    Some(label.into())
}

impl ReviewView {
    pub fn is_editing(&self, note_id: u64) -> bool {
        self.editing.as_ref().is_some_and(|e| e.note_id == note_id)
    }

    pub fn start_edit(
        &mut self,
        discussion_id: String,
        note_id: u64,
        body: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 20)
                .default_value(body)
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        let submit = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                this.save_edit(window, cx);
            }
        });
        self.editing = Some(NoteEdit {
            discussion_id,
            note_id,
            input,
            _submit: submit,
        });
        cx.notify();
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    pub fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(edit), Some(ctx)) = (self.editing.as_ref(), self.mr.clone()) else {
            return;
        };
        let body = edit.input.read(cx).value().trim().to_string();
        if body.is_empty() || self.note_busy {
            return;
        }
        let discussion_id = edit.discussion_id.clone();
        let note_id = edit.note_id;
        self.note_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .edit_note(
                        &ctx.project,
                        ctx.request.number,
                        &discussion_id,
                        note_id,
                        &body,
                    )
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.note_busy = false;
                match result {
                    Ok(()) => {
                        if this.is_editing(note_id) {
                            this.editing = None;
                        }
                        this.load_discussions(cx);
                    }
                    Err(err) => {
                        log::warn!("editing a comment: {err}");
                        window.push_notification(
                            Notification::error(err.to_string())
                                .title(tr().comment_edit_failed.clone()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_note(
        &mut self,
        discussion_id: String,
        note_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        self.note_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .delete_note(&ctx.project, ctx.request.number, &discussion_id, note_id)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.note_busy = false;
                match result {
                    Ok(()) => this.load_discussions(cx),
                    Err(err) => {
                        log::warn!("deleting a comment: {err}");
                        window.push_notification(
                            Notification::error(err.to_string())
                                .title(tr().comment_delete_failed.clone()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_approval(&mut self, cx: &mut Context<Self>) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        let me = self.me.clone();
        self._approval = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .approval(&ctx.request, &ctx.project, me.as_deref())
                    .await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.approval_busy = false;
                match result {
                    Ok(approval) => this.approval = Some(ApprovalView::new(&approval)),
                    Err(err) => {
                        log::warn!("approvals: {err}");
                        this.approval = None;
                    }
                }
                cx.notify();
            });
        });
    }

    pub fn toggle_approval(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(approval)) = (self.mr.clone(), self.approval.as_ref()) else {
            return;
        };
        let approve = if approval.approved_by_me {
            if !approval.can_unapprove {
                return;
            }
            false
        } else {
            if !approval.can_approve {
                return;
            }
            true
        };
        if self.approval_busy {
            return;
        }
        if approve && self.github_review_pending() {
            self.open_review_sheet(window, cx);
            if let Some(form) = self.drafts.form.as_mut()
                && form.verdicts.contains(&forge::Verdict::Approve)
            {
                form.verdict = forge::Verdict::Approve;
            }
            return;
        }
        self.approval_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                if approve {
                    ctx.client.approve(&ctx.project, ctx.request.number).await
                } else {
                    ctx.client.unapprove(&ctx.project, ctx.request.number).await
                }
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(err) = result {
                    log::warn!("approval: {err}");
                    window.push_notification(
                        Notification::error(err.to_string()).title(tr().approve_failed.clone()),
                        cx,
                    );
                }
                this.load_approval(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

pub fn confirm_delete(
    view: &Entity<ReviewView>,
    discussion_id: String,
    note_id: u64,
    window: &mut Window,
    cx: &mut App,
) {
    let view = view.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let s = tr();
        let view = view.clone();
        let discussion_id = discussion_id.clone();
        alert
            .title(s.delete_comment_title.clone())
            .description(s.delete_comment_desc.clone())
            .ok_text(s.delete_comment.clone())
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let discussion_id = discussion_id.clone();
                view.update(cx, |this, cx| {
                    this.delete_note(discussion_id, note_id, window, cx)
                });
                true
            })
    });
}
