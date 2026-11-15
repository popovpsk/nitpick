use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
    rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    diff::{DiffLinePosition, DiffSide},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    notification::Notification,
    sheet::Sheet,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use ui_resources::Icons;

use review_core::FileStatus;

use super::{ComposerTarget, ReviewView, comment_list, comments};
use crate::localization::tr;

#[derive(Clone)]
pub struct DraftView {
    pub id: String,
    pub body: SharedString,
    pub reply_to: Option<String>,
    pub placed: Option<DiffLinePosition>,
    pub path: Option<SharedString>,
    pub status: Option<FileStatus>,
    pub line_label: Option<SharedString>,
    pub context: Option<SharedString>,
    pub outdated: bool,
}

#[derive(Clone, Copy)]
pub struct SheetRow {
    pub draft: usize,
    pub first_in_group: bool,
}

pub struct DraftEdit {
    pub id: String,
    pub input: Entity<TextareaState>,
    _submit: Subscription,
}

pub struct ReviewForm {
    pub summary: Entity<TextareaState>,
    pub verdict: forge::Verdict,
    pub verdicts: Rc<Vec<forge::Verdict>>,
}

#[derive(Default)]
pub struct Drafts {
    pub pending: Option<Rc<forge::PendingReview>>,
    pub list: Rc<Vec<DraftView>>,
    pub sheet: Rc<Vec<SheetRow>>,
    pub button_label: SharedString,
    pub by_file: HashMap<SharedString, Vec<usize>>,
    pub replies: HashMap<String, Vec<usize>>,
    pub edit: Option<DraftEdit>,
    pub busy: bool,
    pub form: Option<ReviewForm>,
    pub submitting: bool,
}

impl Drafts {
    pub fn has_review(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| !p.drafts.is_empty() || p.handle.is_some())
    }

    pub fn edit_input(&self, id: &str) -> Option<Entity<TextareaState>> {
        self.edit
            .as_ref()
            .filter(|edit| edit.id == id)
            .map(|edit| edit.input.clone())
    }

    pub fn replies_to(&self, discussion_id: &str) -> &[usize] {
        self.replies
            .get(discussion_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

impl ReviewView {
    pub(super) fn load_pending(&mut self, cx: &mut Context<Self>) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        if !self.request_open() {
            self._pending = Task::ready(());
            self.drafts.pending = None;
            self.drafts.edit = None;
            self.index_drafts();
            return;
        }
        self._pending = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .pending_review(&ctx.project, ctx.request.number)
                    .await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(pending) => this.drafts.pending = Some(Rc::new(pending)),
                    Err(err) => log::warn!("pending review: {err}"),
                }
                this.index_drafts();
                let edited = this.drafts.edit.as_ref().map(|edit| edit.id.clone());
                if edited.is_some_and(|id| !this.drafts.list.iter().any(|d| d.id == id)) {
                    this.drafts.edit = None;
                }
                this.rebuild_annotations();
                cx.notify();
            });
        });
    }

    pub(super) fn index_drafts(&mut self) {
        self.drafts.by_file.clear();
        self.drafts.replies.clear();
        let Some(pending) = self.drafts.pending.clone() else {
            self.drafts.list = Rc::default();
            self.drafts.sheet = Rc::default();
            return;
        };
        let s = tr();
        let head = self.mr.as_ref().map(|ctx| ctx.refs.head_sha.clone());
        let can_place = self
            .review
            .as_ref()
            .is_some_and(|r| r.comments_aligned && r.commit.is_none());
        let mut rank = Vec::new();
        if let Some(review) = self.review.as_ref() {
            rank = vec![usize::MAX; review.files.len()];
            for (pos, &file) in review.tree.file_order().iter().enumerate() {
                if let Some(slot) = rank.get_mut(file) {
                    *slot = pos;
                }
            }
        }
        let mut list = Vec::with_capacity(pending.drafts.len());
        let mut keys = Vec::with_capacity(pending.drafts.len());
        for (ix, draft) in pending.drafts.iter().enumerate() {
            let placed = draft
                .position
                .as_ref()
                .filter(|pos| can_place && (pos.head_sha.is_none() || pos.head_sha == head))
                .and_then(|pos| self.draft_line(pos));
            let discussion = draft
                .reply_to
                .as_ref()
                .and_then(|id| self.discussions.iter().find(|d| &d.id == id));
            let anchor = match &draft.reply_to {
                Some(_) => discussion.and_then(|d| d.position.as_ref()),
                None => draft.position.as_ref(),
            };
            let raw_path =
                anchor.and_then(|pos| pos.new_path.as_deref().or(pos.old_path.as_deref()));
            let file = raw_path.and_then(|path| self.draft_file(path));
            let path: Option<SharedString> = match file {
                Some(file) => self.review.as_ref().map(|r| r.files[file].path.clone()),
                None => raw_path.map(|path| SharedString::from(path.to_string())),
            };
            let line = anchor.and_then(|pos| pos.new_line.or(pos.old_line));
            let context = draft.reply_to.as_ref().map(|_| match discussion {
                Some(discussion) => discussion.preview.clone(),
                None => s.draft_reply.clone(),
            });
            let outdated = draft.reply_to.is_none()
                && draft.position.is_some()
                && placed.is_none()
                && can_place;
            if let Some(position) = &placed {
                self.drafts
                    .by_file
                    .entry(position.path().clone())
                    .or_default()
                    .push(ix);
            }
            if let Some(id) = &draft.reply_to {
                self.drafts.replies.entry(id.clone()).or_default().push(ix);
            }
            let order = file
                .and_then(|f| rank.get(f).copied())
                .unwrap_or(usize::MAX);
            keys.push((path.is_some(), order, path.clone(), line.unwrap_or(0), ix));
            list.push(DraftView {
                id: draft.id.clone(),
                body: draft.body.clone().into(),
                reply_to: draft.reply_to.clone(),
                placed,
                status: file.and_then(|f| self.review.as_ref().map(|r| r.files[f].status)),
                path,
                line_label: line.map(|line| SharedString::from(format!("L{line}"))),
                context,
                outdated,
            });
        }
        keys.sort();
        let mut previous: Option<&Option<SharedString>> = None;
        let sheet = keys
            .iter()
            .map(|(_, _, path, _, ix)| {
                let first_in_group = previous.is_none_or(|p| p != path);
                previous = Some(path);
                SheetRow {
                    draft: *ix,
                    first_in_group,
                }
            })
            .collect();
        self.drafts.button_label = format!("{} · {}", s.review_button, list.len()).into();
        self.drafts.list = Rc::new(list);
        self.drafts.sheet = Rc::new(sheet);
    }

    fn draft_file(&self, path: &str) -> Option<usize> {
        self.review
            .as_ref()?
            .files
            .iter()
            .position(|f| f.new_path.as_ref() == path || f.old_path.as_ref() == path)
    }

    fn draft_line(&self, pos: &forge::Position) -> Option<DiffLinePosition> {
        let path = pos.new_path.as_deref().or(pos.old_path.as_deref())?;
        let file = self
            .review
            .as_ref()?
            .files
            .iter()
            .find(|f| f.new_path.as_ref() == path || f.old_path.as_ref() == path)?;
        match (pos.new_line, pos.old_line) {
            (Some(line), _) => Some(DiffLinePosition::new(
                file.path.clone(),
                DiffSide::Modified,
                line as usize,
            )),
            (None, Some(line)) => Some(DiffLinePosition::new(
                file.path.clone(),
                DiffSide::Original,
                line as usize,
            )),
            (None, None) => None,
        }
    }

    pub fn can_draft(&self, target: &ComposerTarget) -> bool {
        let (Some(ctx), Some(_)) = (self.mr.as_ref(), self.drafts.pending.as_ref()) else {
            return false;
        };
        match target {
            ComposerTarget::General => ctx.kind().general_drafts(),
            ComposerTarget::Line(_) => true,
            ComposerTarget::Reply(id) => {
                self.discussions.iter().any(|d| &d.id == id && d.can_reply)
            }
        }
    }

    pub fn can_resolve_on_submit(&self, target: &ComposerTarget) -> bool {
        let ComposerTarget::Reply(id) = target else {
            return false;
        };
        self.mr
            .as_ref()
            .is_some_and(|ctx| ctx.kind().resolves_on_submit())
            && self.can_draft(target)
            && self
                .discussions
                .iter()
                .any(|d| &d.id == id && d.resolve_id.is_some() && !d.resolved)
    }

    pub fn submit_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let to_review = self
            .composer
            .as_ref()
            .is_some_and(|c| self.can_draft(&c.target));
        self.submit_composer(to_review, window, cx);
    }

    pub fn toggle_composer_resolve(&mut self, cx: &mut Context<Self>) {
        if let Some(composer) = self.composer.as_mut() {
            composer.resolve = !composer.resolve;
            cx.notify();
        }
    }

    pub(super) fn composer_has_text(&self, cx: &App) -> bool {
        self.composer
            .as_ref()
            .is_some_and(|c| !c.input.read(cx).value().trim().is_empty())
    }

    pub fn has_unsent_comment(&self, cx: &App) -> bool {
        let draft_edited = self.drafts.edit.as_ref().is_some_and(|edit| {
            self.drafts
                .list
                .iter()
                .find(|d| d.id == edit.id)
                .is_none_or(|d| d.body.as_ref() != edit.input.read(cx).value().as_ref())
        });
        let summary = self
            .drafts
            .form
            .as_ref()
            .is_some_and(|form| !form.summary.read(cx).value().trim().is_empty());
        self.composer_has_text(cx) || draft_edited || summary
    }

    pub fn github_review_pending(&self) -> bool {
        self.mr
            .as_ref()
            .is_some_and(|ctx| ctx.kind() == forge::ForgeKind::GitHub)
            && self
                .drafts
                .pending
                .as_ref()
                .is_some_and(|p| p.handle.is_some())
    }

    pub fn direct_blocked(&self, target: &ComposerTarget) -> bool {
        self.github_review_pending() && !matches!(target, ComposerTarget::General)
    }

    pub(super) fn reveal_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composer) = self.composer.as_ref() else {
            return;
        };
        let input = composer.input.clone();
        match composer.target.clone() {
            ComposerTarget::Line(position) => {
                if let Some(file) = self
                    .review
                    .as_ref()
                    .and_then(|r| r.file_index(position.path()))
                {
                    if self.current_file != Some(file) {
                        self.show_file(file, cx);
                    }
                    if !self.content_pending() {
                        self.diff
                            .update(cx, |state, cx| state.scroll_to_line(position, cx));
                    }
                }
            }
            ComposerTarget::Reply(id) => {
                let ix = self.discussions.iter().position(|d| d.id == id);
                let drawn = ix.and_then(|ix| {
                    self.code_threads
                        .iter()
                        .find(|t| t.discussion == ix && t.file.is_some() && t.placed)
                        .map(|_| ix)
                });
                match drawn {
                    Some(ix) => self.reveal_discussion(ix, window, cx),
                    None => {
                        let on_code = ix
                            .is_some_and(|ix| self.code_threads.iter().any(|t| t.discussion == ix));
                        self.comments_tab = if on_code {
                            comment_list::CommentsTab::Code
                        } else {
                            comment_list::CommentsTab::General
                        };
                        comment_list::open_comments_sheet(self, window, cx);
                    }
                }
            }
            ComposerTarget::General => {
                self.comments_tab = comment_list::CommentsTab::General;
                comment_list::open_comments_sheet(self, window, cx);
            }
        }
        input.update(cx, |state, cx| state.focus(window, cx));
        window.push_notification(Notification::info(tr().finish_comment_first.clone()), cx);
        cx.notify();
    }

    pub fn request_cancel_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer_has_text(cx) {
            self.cancel_composer(cx);
            return;
        }
        let view = cx.entity();
        confirm_discard_comment(window, cx, move |_, cx| {
            view.update(cx, |this, cx| this.cancel_composer(cx));
        });
    }

    pub fn is_editing_draft(&self, id: &str) -> bool {
        self.drafts.edit.as_ref().is_some_and(|e| e.id == id)
    }

    pub fn start_draft_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.drafts.list.iter().find(|d| d.id == id) else {
            return;
        };
        let body = draft.body.clone();
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
                this.save_draft_edit(window, cx);
            }
        });
        self.drafts.edit = Some(DraftEdit {
            id,
            input,
            _submit: submit,
        });
        cx.notify();
    }

    pub fn cancel_draft_edit(&mut self, cx: &mut Context<Self>) {
        self.drafts.edit = None;
        cx.notify();
    }

    pub fn save_draft_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(edit), Some(ctx)) = (self.drafts.edit.as_ref(), self.mr.clone()) else {
            return;
        };
        let body = edit.input.read(cx).value().trim().to_string();
        if body.is_empty() || self.drafts.busy {
            return;
        }
        let id = edit.id.clone();
        self.drafts.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let edited = id.clone();
            let result = net_runtime::spawn(async move {
                ctx.client
                    .edit_draft(&ctx.project, ctx.request.number, &edited, &body)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.drafts.busy = false;
                match result {
                    Ok(()) => {
                        if this.is_editing_draft(&id) {
                            this.drafts.edit = None;
                        }
                        this.load_pending(cx);
                    }
                    Err(err) => {
                        log::warn!("editing a draft: {err}");
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

    fn delete_draft(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        self.drafts.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .delete_draft(&ctx.project, ctx.request.number, &id)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.drafts.busy = false;
                if let Err(err) = result {
                    log::warn!("deleting a draft: {err}");
                    window.push_notification(
                        Notification::error(err.to_string())
                            .title(tr().comment_delete_failed.clone()),
                        cx,
                    );
                }
                this.load_pending(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn go_to_draft(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.drafts.list.get(ix).cloned() else {
            return;
        };
        if let Some(position) = draft.placed {
            let Some(file) = self
                .review
                .as_ref()
                .and_then(|r| r.file_index(position.path()))
            else {
                return;
            };
            if self.current_file != Some(file) {
                self.show_file(file, cx);
            }
            self.focus_diff(window, cx);
            if !self.content_pending() {
                self.diff
                    .update(cx, |state, cx| state.scroll_to_line(position, cx));
            }
        } else if let Some(ix) = draft
            .reply_to
            .and_then(|id| self.discussions.iter().position(|d| d.id == id))
        {
            self.reveal_discussion(ix, window, cx);
        }
        cx.notify();
    }

    pub fn verdicts(&self) -> Vec<forge::Verdict> {
        let Some(ctx) = self.mr.as_ref() else {
            return Vec::new();
        };
        let can_approve = self
            .approval
            .as_ref()
            .is_some_and(|a| a.can_approve && !a.approved_by_me);
        ctx.kind()
            .verdicts()
            .iter()
            .copied()
            .filter(|verdict| *verdict == forge::Verdict::Comment || can_approve)
            .collect()
    }

    pub fn open_review_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafts.form.is_none() {
            let placeholder = tr().review_summary_placeholder.clone();
            let summary = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 12)
                    .placeholder(placeholder)
            });
            self.drafts.form = Some(ReviewForm {
                summary,
                verdict: forge::Verdict::Comment,
                verdicts: Rc::default(),
            });
        }
        let verdicts = Rc::new(self.verdicts());
        if let Some(form) = self.drafts.form.as_mut() {
            if !verdicts.contains(&form.verdict) {
                form.verdict = forge::Verdict::Comment;
            }
            form.verdicts = verdicts;
        }
        let view = cx.entity();
        window.open_sheet(cx, move |sheet, _, cx| {
            render_review_sheet(sheet, &view, cx)
        });
    }

    fn set_verdict(&mut self, verdict: forge::Verdict, cx: &mut Context<Self>) {
        if let Some(form) = self.drafts.form.as_mut() {
            form.verdict = verdict;
            cx.notify();
        }
    }

    fn can_submit_review(&self, cx: &App) -> bool {
        let Some(form) = self.drafts.form.as_ref() else {
            return false;
        };
        let summary = !form.summary.read(cx).value().trim().is_empty();
        let github = self
            .mr
            .as_ref()
            .is_some_and(|ctx| ctx.kind() == forge::ForgeKind::GitHub);
        !self.drafts.submitting
            && match form.verdict {
                forge::Verdict::Comment => !self.drafts.list.is_empty() || summary,
                forge::Verdict::Approve => true,
                forge::Verdict::RequestChanges => summary || !github,
            }
    }

    fn submit_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_submit_review(cx) {
            return;
        }
        let (Some(ctx), Some(form)) = (self.mr.clone(), self.drafts.form.as_ref()) else {
            return;
        };
        let pending = self.drafts.pending.as_deref().cloned().unwrap_or_default();
        let body = form.summary.read(cx).value().trim().to_string();
        let verdict = form.verdict;
        self.drafts.submitting = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .submit_review(&ctx.project, ctx.request.number, &pending, &body, verdict)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.drafts.submitting = false;
                match result {
                    Ok(()) => {
                        this.drafts.form = None;
                        window.close_sheet(cx);
                        window.push_notification(
                            Notification::success(tr().review_submitted.clone()),
                            cx,
                        );
                    }
                    Err(err) => {
                        log::warn!("submitting a review: {err}");
                        window.push_notification(
                            Notification::error(err.to_string()).title(tr().review_failed.clone()),
                            cx,
                        );
                    }
                }
                this.load_discussions(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn discard_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(pending)) = (self.mr.clone(), self.drafts.pending.clone()) else {
            return;
        };
        let pending = (*pending).clone();
        self.drafts.submitting = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client
                    .discard_review(&ctx.project, ctx.request.number, &pending)
                    .await
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.drafts.submitting = false;
                match result {
                    Ok(()) => {
                        this.drafts.form = None;
                        window.close_sheet(cx);
                    }
                    Err(err) => {
                        log::warn!("discarding a review: {err}");
                        window.push_notification(
                            Notification::error(err.to_string()).title(tr().discard_failed.clone()),
                            cx,
                        );
                    }
                }
                this.load_pending(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

pub fn confirm_discard_comment(
    window: &mut Window,
    cx: &mut App,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_ok = Rc::new(on_ok);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let s = tr();
        let on_ok = on_ok.clone();
        alert
            .title(s.discard_comment_title.clone())
            .description(s.discard_comment_desc.clone())
            .ok_text(s.discard.clone())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(s.keep_editing.clone())
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                true
            })
    });
}

fn confirm_delete_draft(view: &Entity<ReviewView>, id: String, window: &mut Window, cx: &mut App) {
    let view = view.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let s = tr();
        let view = view.clone();
        let id = id.clone();
        alert
            .title(s.delete_draft_title.clone())
            .description(s.delete_draft_desc.clone())
            .ok_text(s.delete_comment.clone())
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let id = id.clone();
                view.update(cx, |this, cx| this.delete_draft(id, window, cx));
                true
            })
    });
}

fn confirm_discard_review(view: &Entity<ReviewView>, window: &mut Window, cx: &mut App) {
    let view = view.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let s = tr();
        let view = view.clone();
        alert
            .title(s.discard_review_title.clone())
            .description(s.discard_review_desc.clone())
            .ok_text(s.discard_review.clone())
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                view.update(cx, |this, cx| this.discard_review(window, cx));
                true
            })
    });
}

pub fn render_draft(view: &Entity<ReviewView>, ix: usize, cx: &App) -> AnyElement {
    let s = tr();
    let theme = cx.theme();
    let this = view.read(cx);
    let Some(draft) = this.drafts.list.get(ix) else {
        return div().into_any_element();
    };
    let editing = this.drafts.edit_input(&draft.id);
    let header = h_flex()
        .gap_2()
        .items_center()
        .child(Tag::info().xsmall().child(s.draft_tag.clone()))
        .child(div().flex_1())
        .when(editing.is_none(), |el| {
            el.child(draft_actions(view, ix, draft, this.drafts.busy))
        });
    v_flex()
        .max_w(px(820.))
        .gap_1()
        .p_2()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.info.opacity(0.6))
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .child(header)
        .child(draft_body(view, ix, draft, editing, this.drafts.busy))
        .into_any_element()
}

fn draft_actions(
    view: &Entity<ReviewView>,
    ix: usize,
    draft: &DraftView,
    busy: bool,
) -> AnyElement {
    let s = tr();
    let edit_view = view.clone();
    let delete_view = view.clone();
    let edit_id = draft.id.clone();
    let delete_id = draft.id.clone();
    h_flex()
        .flex_none()
        .gap_0p5()
        .child(
            Button::new(ElementId::NamedInteger("draft-edit".into(), ix as u64))
                .ghost()
                .xsmall()
                .icon(Icon::new(Icons::Pencil))
                .tooltip(s.edit_comment.clone())
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let id = edit_id.clone();
                    edit_view.update(cx, |this, cx| this.start_draft_edit(id, window, cx));
                }),
        )
        .child(
            Button::new(ElementId::NamedInteger("draft-delete".into(), ix as u64))
                .ghost()
                .xsmall()
                .icon(Icon::new(Icons::Trash))
                .tooltip(s.delete_comment.clone())
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    confirm_delete_draft(&delete_view, delete_id.clone(), window, cx)
                }),
        )
        .into_any_element()
}

fn draft_body(
    view: &Entity<ReviewView>,
    ix: usize,
    draft: &DraftView,
    editing: Option<Entity<TextareaState>>,
    busy: bool,
) -> AnyElement {
    let s = tr();
    let Some(input) = editing else {
        return div()
            .text_sm()
            .child(comments::remote_markdown(
                ElementId::NamedInteger("draft-body".into(), ix as u64),
                draft.body.clone(),
            ))
            .into_any_element();
    };
    let cancel_view = view.clone();
    let save_view = view.clone();
    v_flex()
        .gap_2()
        .child(Textarea::new(&input))
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new(ElementId::NamedInteger(
                        "draft-edit-cancel".into(),
                        ix as u64,
                    ))
                    .outline()
                    .small()
                    .label(s.cancel.clone())
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |this, cx| this.cancel_draft_edit(cx))
                    }),
                )
                .child(
                    Button::new(ElementId::NamedInteger("draft-edit-save".into(), ix as u64))
                        .primary()
                        .small()
                        .label(s.save.clone())
                        .loading(busy)
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            save_view.update(cx, |this, cx| this.save_draft_edit(window, cx))
                        }),
                ),
        )
        .into_any_element()
}

fn render_sheet_draft(view: &Entity<ReviewView>, row: SheetRow, cx: &App) -> Option<AnyElement> {
    let s = tr();
    let theme = cx.theme();
    let this = view.read(cx);
    let ix = row.draft;
    let draft = this.drafts.list.get(ix)?;
    let busy = this.drafts.busy || this.drafts.submitting;
    let editing = this.drafts.edit_input(&draft.id);
    let reachable = draft.placed.is_some() || draft.reply_to.is_some();
    let group = row.first_in_group.then(|| {
        h_flex()
            .pt_1()
            .gap_1p5()
            .text_sm()
            .child(
                Icon::new(match &draft.path {
                    Some(_) => comment_list::status_icon(draft.status),
                    None => Icons::MessageSquare,
                })
                .small()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(
                        draft
                            .path
                            .clone()
                            .unwrap_or_else(|| s.comments_general.clone()),
                    ),
            )
    });
    let go_view = view.clone();
    let header = h_flex()
        .gap_1p5()
        .items_center()
        .text_xs()
        .text_color(theme.muted_foreground)
        .when_some(draft.line_label.clone(), |el, line| el.child(line))
        .when(draft.reply_to.is_some(), |el| {
            el.child(Icon::new(Icons::Reply).xsmall())
                .child(s.draft_reply.clone())
        })
        .when(draft.outdated, |el| {
            el.child(Tag::warning().xsmall().child(s.outdated.clone()))
        })
        .child(div().flex_1())
        .when(reachable && editing.is_none(), |el| {
            el.child(
                Button::new(ElementId::NamedInteger("review-draft-go".into(), ix as u64))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::ArrowUpRight))
                    .label(s.go_to_code.clone())
                    .on_click(move |_, window, cx| {
                        window.close_sheet(cx);
                        go_view.update(cx, |this, cx| this.go_to_draft(ix, window, cx));
                    }),
            )
        })
        .when(editing.is_none(), |el| {
            el.child(draft_actions(view, ix, draft, busy))
        });
    let context = draft.context.clone().map(|context| {
        div()
            .pl_2()
            .border_l_2()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .child(context)
    });
    Some(
        v_flex()
            .gap_1()
            .children(group)
            .child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.info.opacity(0.6))
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .child(header)
                    .children(context)
                    .child(draft_body(view, ix, draft, editing, busy)),
            )
            .into_any_element(),
    )
}

fn render_review_sheet(sheet: Sheet, view: &Entity<ReviewView>, cx: &App) -> Sheet {
    let s = tr();
    let theme = cx.theme();
    let this = view.read(cx);
    let Some(form) = this.drafts.form.as_ref() else {
        return sheet;
    };
    let rows = this.drafts.sheet.clone();
    let busy = this.drafts.busy || this.drafts.submitting;
    let list = if rows.is_empty() {
        div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(s.review_empty.clone())
            .into_any_element()
    } else {
        v_flex()
            .gap_2()
            .children(
                rows.iter()
                    .filter_map(|row| render_sheet_draft(view, *row, cx)),
            )
            .into_any_element()
    };
    let verdicts = form.verdicts.clone();
    let verdict = form.verdict;
    let verdict_bar = (verdicts.len() > 1).then(|| {
        let verdict_view = view.clone();
        let choices = verdicts.clone();
        verdicts
            .iter()
            .fold(
                TabBar::new("review-verdict").segmented().small(),
                |bar, choice| {
                    bar.child(Tab::new().label(match choice {
                        forge::Verdict::Comment => s.verdict_comment.clone(),
                        forge::Verdict::Approve => s.verdict_approve.clone(),
                        forge::Verdict::RequestChanges => s.verdict_request_changes.clone(),
                    }))
                },
            )
            .selected_index(verdicts.iter().position(|v| *v == verdict).unwrap_or(0))
            .on_click(move |ix, _, cx| {
                if let Some(choice) = choices.get(*ix).copied() {
                    verdict_view.update(cx, |this, cx| this.set_verdict(choice, cx));
                }
            })
    });
    let can_submit = this.can_submit_review(cx);
    let has_review = this.drafts.has_review();
    let submit_view = view.clone();
    let discard_view = view.clone();
    let footer = h_flex()
        .w_full()
        .gap_2()
        .justify_between()
        .child(
            Button::new("review-discard")
                .ghost()
                .small()
                .label(s.discard_review.clone())
                .disabled(busy || !has_review)
                .on_click(move |_, window, cx| confirm_discard_review(&discard_view, window, cx)),
        )
        .child(
            Button::new("review-submit")
                .primary()
                .small()
                .icon(Icon::new(Icons::Send))
                .label(s.submit_review.clone())
                .loading(this.drafts.submitting)
                .disabled(!can_submit || busy)
                .on_click(move |_, window, cx| {
                    submit_view.update(cx, |this, cx| this.submit_review(window, cx))
                }),
        );
    sheet
        .title(s.review_title.clone())
        .size(px(480.))
        .child(
            v_flex().gap_4().pb_4().child(list).child(
                v_flex()
                    .gap_2()
                    .child(Textarea::new(&form.summary).min_h(rems(4.)))
                    .children(verdict_bar),
            ),
        )
        .footer(footer)
}
