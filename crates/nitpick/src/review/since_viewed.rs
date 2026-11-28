use gpui::{
    AnyElement, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
};
use gpui_component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{DiffFile, DiffLinePosition, DiffSide},
    notification::Notification,
    tooltip::Tooltip,
};
use review_core::{Change, Side, fingerprint_blobs};
use ui_resources::Icons;

use super::find_bar::FindSource;
use super::find_match::TextSource;
use super::model::{self, Interdiff};
use super::{ReviewSource, ReviewView};
use crate::localization::tr;

pub(super) fn is_blob(id: &str) -> bool {
    id.len() >= 7 && id.bytes().all(|b| b.is_ascii_hexdigit()) && !id.bytes().all(|b| b == b'0')
}

impl ReviewView {
    pub(super) fn refresh_seen_before(&mut self, cx: &mut Context<Self>) -> bool {
        let mut unknown: Vec<String> = Vec::new();
        let seen: Vec<Option<String>> = match self.review.as_ref() {
            Some(review) if self.source != ReviewSource::WorkingTree => {
                let store = self.viewed_store.read(cx).store();
                review
                    .files
                    .iter()
                    .map(|file| {
                        if file.binary || !is_blob(&file.new_blob) {
                            return None;
                        }
                        for mark in store.viewed_versions_except(&file.path, &file.fingerprint) {
                            let Some((_, new)) = fingerprint_blobs(mark) else {
                                continue;
                            };
                            if !is_blob(new) || new == file.new_blob {
                                continue;
                            }
                            match self.blob_known.get(new) {
                                Some(true) => return Some(new.to_string()),
                                Some(false) => continue,
                                None => {
                                    unknown.push(new.to_string());
                                    return None;
                                }
                            }
                        }
                        None
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        if !unknown.is_empty() {
            self.check_blobs(unknown, cx);
        }
        let changed = seen != self.seen_before;
        self.seen_before = seen;
        changed
    }

    fn check_blobs(&mut self, mut ids: Vec<String>, cx: &mut Context<Self>) {
        if self.blob_checking {
            return;
        }
        ids.sort_unstable();
        ids.dedup();
        self.blob_checking = true;
        let repo = self.repo.clone();
        self._blob_check = cx.spawn(async move |this, cx| {
            let known: Vec<(String, bool)> = cx
                .background_spawn(async move {
                    let exists = repo.has_blobs(&ids);
                    ids.into_iter().zip(exists).collect()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.blob_checking = false;
                this.blob_known.extend(known);
                if this.refresh_seen_before(cx) {
                    this.rebuild_rows();
                    this.sync_since_viewed(cx);
                    cx.notify();
                }
            });
        });
    }

    pub(super) fn seen_before(&self, file: usize) -> bool {
        self.seen_before.get(file).is_some_and(Option::is_some)
    }

    pub(super) fn interdiff_key(&self, file: usize) -> Option<String> {
        if !self.since_mode {
            return None;
        }
        let previous = self.seen_before.get(file)?.as_ref()?;
        let shown = self.review.as_ref()?.files.get(file)?;
        Some(format!(
            "{}\0{}\0{previous}\0{}",
            shown.path, shown.fingerprint, self.installed_whitespace
        ))
    }

    pub(super) fn install_content(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(shown) = review.files.get(file) else {
            return;
        };
        let Some(key) = self.interdiff_key(file) else {
            self.interdiff = None;
            self.interdiff_loading = None;
            self.pending_jump = None;
            self.pending_scroll = None;
            let diff_file = review.diff_file(file);
            self.sync_diff_mode(cx);
            self.set_diff_files(diff_file.into_iter().collect(), cx);
            return;
        };
        if let Some(interdiff) = self.interdiff.as_mut().filter(|d| d.key == key) {
            interdiff.file = file;
            let files: Vec<DiffFile> = self
                .interdiff
                .as_ref()
                .and_then(|d| d.diff.clone())
                .into_iter()
                .collect();
            self.sync_diff_mode(cx);
            self.set_diff_files(files, cx);
            return;
        }
        self.interdiff = None;
        self.sync_diff_mode(cx);
        self.diff.update(cx, |state, cx| state.set_files([], cx));
        if self.interdiff_loading.as_ref() == Some(&key) {
            return;
        }
        let Some(previous) = self.seen_before.get(file).cloned().flatten() else {
            return;
        };
        let repo = self.repo.clone();
        let path = shown.path.to_string();
        let current = shown.new_blob.clone();
        let whitespace = self.installed_whitespace;
        self.interdiff_loading = Some(key.clone());
        self._interdiff = cx.spawn(async move |this, cx| {
            let task_key = key.clone();
            let result = cx
                .background_spawn(async move {
                    model::interdiff(
                        &repo, file, task_key, &path, &previous, &current, whitespace,
                    )
                    .map_err(|err| (previous, err))
                })
                .await;
            let _ = this.update(cx, |this, cx| this.install_interdiff(key, result, cx));
        });
    }

    fn install_interdiff(
        &mut self,
        key: String,
        result: Result<Interdiff, (String, anyhow::Error)>,
        cx: &mut Context<Self>,
    ) {
        if self.interdiff_loading.as_ref() != Some(&key) {
            return;
        }
        self.interdiff_loading = None;
        match result {
            Ok(mut interdiff) => {
                let Some(file) = self.current_file else {
                    return;
                };
                if self.interdiff_key(file).as_ref() != Some(&key) {
                    return;
                }
                interdiff.file = file;
                let files: Vec<DiffFile> = interdiff.diff.clone().into_iter().collect();
                self.interdiff = Some(interdiff);
                self.set_diff_files(files, cx);
                self.rebuild_annotations();
                self.sync_find(cx);
                let scroll = self.pending_scroll.take();
                if let Some(forward) = self.pending_jump.take()
                    && scroll.is_none()
                {
                    let count = self.shown_changes(file).len();
                    if count > 0 {
                        self.scroll_to_change(file, if forward { 0 } else { count - 1 }, cx);
                    }
                }
                if let Some(position) = scroll {
                    self.diff
                        .update(cx, |state, cx| state.scroll_to_line(position, cx));
                }
            }
            Err((previous, err)) => {
                log::warn!("comparing with the version viewed before: {err:#}");
                self.blob_known.insert(previous, false);
                self.refresh_seen_before(cx);
                self.rebuild_rows();
                self.notify(
                    Notification::error(format!("{err:#}")).title(tr().since_viewed_failed.clone()),
                    cx,
                );
                if let Some(file) = self.current_file {
                    let scroll = self.pending_scroll.take();
                    self.install_content(file, cx);
                    self.rebuild_annotations();
                    self.sync_find(cx);
                    match scroll {
                        Some(position) if self.content_pending() => {
                            self.pending_scroll = Some(position);
                        }
                        Some(position) => self
                            .diff
                            .update(cx, |state, cx| state.scroll_to_line(position, cx)),
                        None => {}
                    }
                }
            }
        }
        cx.notify();
    }

    pub(super) fn sync_since_viewed(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.current_file else {
            return;
        };
        let wanted = self.interdiff_key(file);
        let installed = self
            .interdiff
            .as_ref()
            .map(|d| &d.key)
            .or(self.interdiff_loading.as_ref());
        if wanted.as_ref() != installed {
            self.install_content(file, cx);
            self.rebuild_annotations();
            self.sync_find(cx);
        }
    }

    pub(super) fn toggle_since_mode(&mut self, cx: &mut Context<Self>) {
        self.since_mode = !self.since_mode;
        self.change_cursor = None;
        self.change_scrolled = false;
        self.pending_jump = None;
        self.pending_scroll = None;
        self.sync_since_viewed(cx);
        cx.notify();
    }

    pub(super) fn shown_position(&self, position: &DiffLinePosition) -> Option<DiffLinePosition> {
        let review = self.review.as_ref()?;
        let file = review.file_index(position.path())?;
        if self.interdiff_key(file).is_none() || matches!(position.side(), DiffSide::Modified) {
            return Some(position.clone());
        }
        let line = review.files[file]
            .line_map
            .new_line_for_old(position.line() as u32)?;
        Some(DiffLinePosition::new(
            position.path().clone(),
            DiffSide::Modified,
            line as usize,
        ))
    }

    pub(super) fn review_position(&self, shown: &DiffLinePosition) -> Option<DiffLinePosition> {
        let review = self.review.as_ref()?;
        let file = review.file_index(shown.path())?;
        if self.interdiff_key(file).is_none() {
            return Some(shown.clone());
        }
        let line = match shown.side() {
            DiffSide::Modified => shown.line() as u32,
            DiffSide::Original => self
                .interdiff
                .as_ref()
                .filter(|d| d.file == file)?
                .map
                .new_line_for_old(shown.line() as u32)?,
        };
        review.files[file].line_map.locate(Side::New, line)?;
        Some(DiffLinePosition::new(
            shown.path().clone(),
            DiffSide::Modified,
            line as usize,
        ))
    }

    pub(super) fn reveal_line(
        &mut self,
        file: usize,
        position: DiffLinePosition,
        cx: &mut Context<Self>,
    ) {
        let empty = self
            .interdiff
            .as_ref()
            .is_some_and(|d| d.file == file && d.is_empty());
        if self.interdiff_key(file).is_some() && (empty || self.shown_position(&position).is_none())
        {
            self.toggle_since_mode(cx);
        }
        if self.current_file != Some(file) {
            self.show_file(file, cx);
        }
        let Some(shown) = self.shown_position(&position) else {
            return;
        };
        self.change_cursor = None;
        if self.content_pending() {
            self.pending_scroll = Some(shown);
        } else {
            self.diff
                .update(cx, |state, cx| state.scroll_to_line(shown, cx));
        }
    }

    pub(super) fn shown_changes(&self, file: usize) -> &[Change] {
        match &self.interdiff {
            Some(interdiff) if interdiff.file == file && self.current_file == Some(file) => {
                &interdiff.changes
            }
            _ => self
                .review
                .as_ref()
                .and_then(|review| review.files.get(file))
                .map_or(&[], |f| f.changes.as_slice()),
        }
    }

    pub(super) fn content_pending(&self) -> bool {
        self.interdiff_loading.is_some()
    }

    pub(super) fn since_viewed_find_source(&self) -> Option<FindSource> {
        let interdiff = self.interdiff.as_ref()?;
        let review = self.review.as_ref()?;
        let file = review.files.get(interdiff.file)?;
        Some(FindSource {
            key: format!("since\0{}", interdiff.key),
            path: file.path.clone(),
            old: TextSource::Blob(interdiff.previous.clone()),
            new: TextSource::Blob(file.new_blob.clone()),
            map: interdiff.map.clone(),
        })
    }

    pub(super) fn render_since_toggle(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let file = self.current_file?;
        if !self.seen_before(file) {
            return None;
        }
        let s = tr();
        Some(
            Button::new("since-viewed")
                .ghost()
                .xsmall()
                .icon(Icon::new(Icons::FileClock))
                .label(s.since_viewed_label.clone())
                .selected(self.since_mode)
                .tooltip(if self.since_mode {
                    s.since_viewed_on.clone()
                } else {
                    s.since_viewed_off.clone()
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_since_mode(cx)))
                .into_any_element(),
        )
    }

    pub(super) fn render_seen_badge(&self, ix: usize, cx: &App) -> AnyElement {
        div()
            .id(("tree-seen-before", ix))
            .flex_none()
            .child(
                Icon::new(Icons::FileClock)
                    .xsmall()
                    .text_color(cx.theme().blue),
            )
            .tooltip(|window, cx| Tooltip::new(tr().seen_before_tooltip.clone()).build(window, cx))
            .into_any_element()
    }
}
