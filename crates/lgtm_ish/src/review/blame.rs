use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use git_repo::{BlameCommit, BlameLine};
use gpui::{
    AnyElement, App, AppContext as _, Context, Empty, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, Theme,
    button::{Button, ButtonVariants as _},
    diff::{DiffGutterMarker, DiffLinePosition, DiffSide},
    h_flex,
    notification::Notification,
    tooltip::Tooltip,
    v_flex,
};
use review_core::{FileStatus, LineMap, Side};
use ui_resources::Icons;

use super::{LoadedReview, ReviewSource, ReviewView};
use crate::localization::tr;
use crate::settings_store::{SettingsStore, settings, update_settings};

actions!(review, [ToggleBlame]);

const TINT_STRENGTHS: [f32; 2] = [0.28, 0.5];
const BLAME_DELAY: Duration = Duration::from_millis(120);
const TINT_COUNT: usize = 12;

#[derive(Clone, PartialEq, Eq, Hash)]
struct BlameKey {
    rev: Option<String>,
    path: SharedString,
}

struct BlameChip {
    initials: SharedString,
    tint: usize,
    uncommitted: bool,
    author: SharedString,
    time: i64,
    short_id: SharedString,
    summary: SharedString,
}

struct Computed {
    runs: Vec<(usize, usize)>,
    chips: Vec<BlameChip>,
}

pub struct BlameMarks {
    runs: Vec<(usize, usize)>,
    chips: Vec<Rc<BlameChip>>,
    pub markers: Vec<DiffGutterMarker>,
}

struct Shown {
    key: BlameKey,
    fingerprint: String,
    marks: Rc<BlameMarks>,
}

pub(super) struct BlameState {
    enabled: bool,
    review: Weak<LoadedReview>,
    cache: HashMap<BlameKey, Rc<BlameMarks>>,
    failed: HashSet<BlameKey>,
    shown: Option<Shown>,
    pending: Option<BlameKey>,
    failure_reported: bool,
    _task: Task<()>,
    _settings: Subscription,
}

impl BlameState {
    pub(super) fn new(cx: &mut Context<ReviewView>) -> Self {
        Self {
            enabled: settings(cx).show_blame,
            review: Weak::new(),
            cache: HashMap::new(),
            failed: HashSet::new(),
            shown: None,
            pending: None,
            failure_reported: false,
            _task: Task::ready(()),
            _settings: cx
                .observe_global::<SettingsStore>(|this: &mut ReviewView, cx| this.sync_blame(cx)),
        }
    }

    fn forget(&mut self) {
        self.invalidate();
        self.shown = None;
    }

    fn invalidate(&mut self) {
        self.cache.clear();
        self.failed.clear();
        self.pending = None;
        self.failure_reported = false;
        self._task = Task::ready(());
    }
}

impl ReviewView {
    pub fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        update_settings(cx, |s| s.show_blame = !s.show_blame);
    }

    fn sync_blame(&mut self, cx: &mut Context<Self>) {
        let enabled = settings(cx).show_blame;
        if enabled == self.blame.enabled {
            return;
        }
        self.blame.enabled = enabled;
        self.blame.forget();
        if enabled {
            self.request_blame(cx);
        }
        cx.notify();
    }

    pub(super) fn retry_failed_blame(&mut self, cx: &mut Context<Self>) {
        if self.blame.failed.is_empty() {
            return;
        }
        for key in std::mem::take(&mut self.blame.failed) {
            self.blame.cache.remove(&key);
        }
        self.blame.failure_reported = false;
        self.request_blame(cx);
    }

    pub(super) fn request_blame(&mut self, cx: &mut Context<Self>) {
        if !self.blame.enabled {
            return;
        }
        let Some(review) = self.review.clone() else {
            self.blame.forget();
            return;
        };
        if !std::ptr::eq(self.blame.review.as_ptr(), Rc::as_ptr(&review)) {
            self.blame.invalidate();
            self.blame.review = Rc::downgrade(&review);
        }
        let Some(file) = self.current_file.and_then(|ix| review.files.get(ix)) else {
            self.blame.shown = None;
            self.blame.pending = None;
            return;
        };
        if file.binary {
            self.blame.shown = None;
            self.blame.pending = None;
            return;
        }
        let (key, side) = match (file.status, &self.source) {
            (FileStatus::Deleted, _) => (
                BlameKey {
                    rev: Some(review.base_sha.clone()),
                    path: file.old_path.clone(),
                },
                DiffSide::Original,
            ),
            (_, ReviewSource::WorkingTree) => (
                BlameKey {
                    rev: None,
                    path: file.new_path.clone(),
                },
                DiffSide::Modified,
            ),
            _ => (
                BlameKey {
                    rev: Some(review.head_sha.clone()),
                    path: file.new_path.clone(),
                },
                DiffSide::Modified,
            ),
        };
        let fingerprint = file.fingerprint.clone();
        if let Some(marks) = self.blame.cache.get(&key) {
            self.blame.shown = Some(Shown {
                key,
                fingerprint,
                marks: marks.clone(),
            });
            self.blame.pending = None;
            cx.notify();
            return;
        }
        if self.blame.pending.as_ref() == Some(&key) {
            return;
        }
        let diff_path = file.path.clone();
        let keep = self
            .blame
            .shown
            .as_ref()
            .is_some_and(|shown| shown.key == key && shown.fingerprint == fingerprint);
        if !keep {
            let placeholder = Rc::new(BlameMarks::build(
                Computed {
                    runs: Vec::new(),
                    chips: Vec::new(),
                },
                &diff_path,
                side,
            ));
            self.blame.shown = Some(Shown {
                key: key.clone(),
                fingerprint: fingerprint.clone(),
                marks: placeholder,
            });
        }
        self.blame.pending = Some(key.clone());
        let (context, min_folded) = {
            let state = self.diff.read(cx);
            (state.context_lines(), state.min_collapsed_lines())
        };
        let line_map = file.shown_map().clone();
        let repo = self.repo.clone();
        self.blame._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BLAME_DELAY).await;
            let fetch_key = key.clone();
            let result = cx
                .background_spawn(async move {
                    let lines = repo.blame(fetch_key.rev.as_deref(), &fetch_key.path)?;
                    let side = match side {
                        DiffSide::Original => Side::Old,
                        DiffSide::Modified => Side::New,
                    };
                    Ok::<_, git_repo::GitError>(compute(
                        &lines, &line_map, side, context, min_folded,
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.install_blame(key, fingerprint, diff_path, side, result, cx)
            });
        });
        cx.notify();
    }

    fn install_blame(
        &mut self,
        key: BlameKey,
        fingerprint: String,
        diff_path: SharedString,
        side: DiffSide,
        result: Result<Computed, git_repo::GitError>,
        cx: &mut Context<Self>,
    ) {
        let computed = result.unwrap_or_else(|err| {
            log::warn!("blame {:?} {}: {err}", key.rev, key.path);
            self.blame.failed.insert(key.clone());
            if !self.blame.failure_reported {
                self.blame.failure_reported = true;
                self.notify(
                    Notification::error(format!("{}: {err}", tr().blame_failed)),
                    cx,
                );
            }
            Computed {
                runs: Vec::new(),
                chips: Vec::new(),
            }
        });
        let marks = Rc::new(BlameMarks::build(computed, &diff_path, side));
        self.blame.cache.insert(key.clone(), marks.clone());
        if self.blame.pending.as_ref() == Some(&key) {
            self.blame.pending = None;
            self.blame.shown = Some(Shown {
                key,
                fingerprint,
                marks,
            });
            cx.notify();
        }
    }

    pub(super) fn blame_marks(&self) -> Option<Rc<BlameMarks>> {
        if !self.blame.enabled {
            return None;
        }
        self.blame.shown.as_ref().map(|shown| shown.marks.clone())
    }

    pub(super) fn render_blame_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("toggle-blame")
            .ghost()
            .small()
            .icon(Icon::new(Icons::GitCommitHorizontal))
            .selected(self.blame.enabled)
            .tooltip_with_action(tr().toggle_blame.clone(), &ToggleBlame, Some("Review"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_blame(cx)))
            .into_any_element()
    }
}

fn compute(
    lines: &[BlameLine],
    line_map: &LineMap,
    side: Side,
    context: Option<usize>,
    min_folded: usize,
) -> Computed {
    let mut chips = Vec::new();
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    let keys: Vec<usize> = lines
        .iter()
        .map(|line| {
            *by_id.entry(line.commit.id.as_str()).or_insert_with(|| {
                chips.push(chip(&line.commit));
                chips.len() - 1
            })
        })
        .collect();
    let starts = line_map.shown_block_starts(side, context, min_folded);
    let runs = review_core::blame::marker_lines(&keys, &starts)
        .into_iter()
        .map(|line| (line as usize, keys[line as usize - 1]))
        .collect();
    Computed { runs, chips }
}

fn chip(commit: &BlameCommit) -> BlameChip {
    BlameChip {
        initials: review_core::blame::initials(&commit.author).into(),
        tint: review_core::blame::tint_index(&commit.id, TINT_COUNT),
        uncommitted: commit.is_uncommitted(),
        author: commit.author.clone().into(),
        time: commit.author_time,
        short_id: commit.id.get(..8).unwrap_or(&commit.id).to_string().into(),
        summary: commit.summary.clone().into(),
    }
}

impl BlameMarks {
    fn build(computed: Computed, path: &SharedString, side: DiffSide) -> Self {
        let chips: Vec<Rc<BlameChip>> = computed.chips.into_iter().map(Rc::new).collect();
        let mut markers: Vec<DiffGutterMarker> = computed
            .runs
            .iter()
            .map(|&(line, chip)| {
                let chip = &chips[chip];
                let label = if chip.uncommitted {
                    tr().blame_uncommitted.clone()
                } else {
                    format!("{} {}", chip.author, chip.short_id).into()
                };
                DiffGutterMarker::new(
                    ("blame", line),
                    DiffLinePosition::new(path.clone(), side, line),
                )
                .with_label(label)
            })
            .collect();
        if markers.is_empty() {
            markers.push(DiffGutterMarker::new(
                "blame-lane",
                DiffLinePosition::new(path.clone(), side, usize::MAX),
            ));
        }
        Self {
            runs: computed.runs,
            chips,
            markers,
        }
    }

    pub fn render_chip(
        &self,
        marker: &DiffGutterMarker,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let line = marker.position().line();
        let Ok(run) = self.runs.binary_search_by_key(&line, |&(start, _)| start) else {
            return Empty.into_any_element();
        };
        let chip = self.chips[self.runs[run].1].clone();
        let theme = cx.theme();
        let row = window.rem_size() * (f32::from(theme.mono_font_size) / 16.) * 1.6;
        let height = row * 0.78;
        let (background, foreground, label) = if chip.uncommitted {
            (theme.muted, theme.muted_foreground, tr().blame_you.clone())
        } else {
            (
                tint(theme, chip.tint),
                theme.foreground,
                chip.initials.clone(),
            )
        };
        div()
            .id(marker.id().clone())
            .h(height)
            .min_w(height)
            .max_w(row)
            .px(row * 0.12)
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded_full()
            .bg(background)
            .text_color(foreground)
            .font_family(theme.font_family.clone())
            .text_size(row * 0.4)
            .line_height(height)
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(label)
            .tooltip(move |window, cx| {
                let chip = chip.clone();
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs() as i64);
                let when: SharedString = tr().relative_time(now - chip.time).into();
                Tooltip::element(move |_, cx| chip_details(&chip, &when, cx)).build(window, cx)
            })
            .into_any_element()
    }
}

fn chip_details(chip: &BlameChip, when: &SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    if chip.uncommitted {
        return div()
            .child(tr().blame_uncommitted.clone())
            .into_any_element();
    }
    v_flex()
        .py_0p5()
        .gap_0p5()
        .max_w(px(420.))
        .child(
            h_flex()
                .gap_2()
                .child(div().font_semibold().child(chip.author.clone()))
                .child(div().text_color(theme.muted_foreground).child(when.clone())),
        )
        .child(
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .flex_none()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(chip.short_id.clone()),
                )
                .child(div().min_w_0().child(chip.summary.clone())),
        )
        .into_any_element()
}

fn tint(theme: &Theme, ix: usize) -> Hsla {
    let hues = [
        theme.blue,
        theme.magenta,
        theme.cyan,
        theme.yellow,
        theme.green,
        theme.red,
    ];
    hues[ix % hues.len()].opacity(TINT_STRENGTHS[(ix / hues.len()) % TINT_STRENGTHS.len()])
}
