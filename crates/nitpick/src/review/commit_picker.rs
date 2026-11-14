use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, Window, px};
use ui_resources::Icons;

use super::{Busy, LoadedReview, ReviewView};
use crate::localization::tr;
use crate::picker::{Picker, PickerEntry, PickerEvent};

const LABEL_SUBJECT_CHARS: usize = 48;

pub struct CommitPicker {
    picker: Entity<Picker>,
    label: Option<SharedString>,
    tooltip: Option<SharedString>,
    shas: Vec<Option<String>>,
    _subscriptions: Vec<Subscription>,
}

impl CommitPicker {
    pub fn new(window: &mut Window, cx: &mut Context<ReviewView>) -> Self {
        let picker = cx.new(|cx| {
            Picker::new(
                Icons::GitCommitHorizontal,
                || tr().search_commits.clone(),
                px(520.),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&picker, window, |this, _, event, window, cx| {
                this.on_commit_picker(event, window, cx)
            }),
            cx.observe(&picker, |_, _, cx| cx.notify()),
        ];
        Self {
            picker,
            label: None,
            tooltip: None,
            shas: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn picker(&self) -> &Entity<Picker> {
        &self.picker
    }

    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    pub fn tooltip(&self) -> Option<&SharedString> {
        self.tooltip.as_ref()
    }

    pub fn set_review(&mut self, review: &LoadedReview) {
        let Some(commit) = &review.commit else {
            self.label = None;
            self.tooltip = None;
            return;
        };
        let short = short_sha(&commit.sha);
        let mut subject: String = commit.subject.chars().take(LABEL_SUBJECT_CHARS).collect();
        if subject.len() < commit.subject.len() {
            subject.push('…');
        }
        self.label = Some(format!("{short} {subject}").into());
        self.tooltip = Some(format!("{} · {}", commit.subject, commit.author).into());
    }
}

impl ReviewView {
    pub(super) fn close_commit_picker(&mut self, cx: &mut Context<Self>) {
        self.commit_picker
            .picker
            .update(cx, |picker, cx| picker.close(cx));
    }

    fn on_commit_picker(
        &mut self,
        event: &PickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PickerEvent::Open => self.install_commit_choices(window, cx),
            PickerEvent::Confirm { entry, .. } => self.pick_commit(*entry, cx),
        }
    }

    fn install_commit_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let s = tr();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let mut entries = Vec::with_capacity(review.commits.len() + 1);
        entries.push(PickerEntry {
            label: s.all_changes.clone(),
            detail: s.commits_count(review.commits.len()).into(),
            search_detail: false,
        });
        entries.extend(review.commits.iter().map(|commit| {
            PickerEntry {
                label: commit.subject.clone().into(),
                detail: format!(
                    "{} · {} · {}",
                    short_sha(&commit.sha),
                    commit.author,
                    s.relative_time(now - commit.time)
                )
                .into(),
                search_detail: true,
            }
        }));
        let checked = review
            .commit_sha()
            .and_then(|sha| review.commits.iter().position(|c| c.sha == sha))
            .map_or(0, |ix| ix + 1);
        self.commit_picker.shas = std::iter::once(None)
            .chain(review.commits.iter().map(|c| Some(c.sha.clone())))
            .collect();
        self.commit_picker.picker.update(cx, |picker, cx| {
            picker.set_entries(entries, Some(checked), window, cx)
        });
    }

    fn pick_commit(&mut self, entry: usize, cx: &mut Context<Self>) {
        let Some(commit) = self.commit_picker.shas.get(entry).cloned() else {
            return;
        };
        if commit == self.commit {
            return;
        }
        self.commit = commit;
        match self.busy {
            None | Some(Busy::Refreshing) => self.reload(cx),
            Some(_) => self.load(cx),
        }
    }
}

fn short_sha(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}
