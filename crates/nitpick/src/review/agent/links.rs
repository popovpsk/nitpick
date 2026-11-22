use std::rc::Rc;

use ::agent::brief;
use ::agent::prompt::{self, Selection};
use app_db::{AgentKind, AgentRole};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, Window, div,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use review_core::diff_link::{self, DiffLink};
use review_core::{LineKind, Side};

use super::{AgentEvent, AgentPanel};
use crate::localization::tr;
use crate::review::comments::opens_in_browser;
use crate::review::{AgentNextStop, AgentPrevStop};

const SCHEME: &str = "diff:";
const MAX_SELECTION_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkKind {
    Diff,
    Browser,
    Ignored,
}

pub(super) fn classify(url: &str) -> LinkKind {
    if url
        .get(..SCHEME.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(SCHEME))
    {
        LinkKind::Diff
    } else if opens_in_browser(url) {
        LinkKind::Browser
    } else {
        LinkKind::Ignored
    }
}

pub(super) fn resolve_file<'a>(
    files: impl IntoIterator<Item = (&'a str, Option<&'a str>)> + Clone,
    path: &str,
) -> Option<usize> {
    files
        .clone()
        .into_iter()
        .position(|(new, _)| new == path)
        .or_else(|| {
            files
                .into_iter()
                .position(|(_, old)| old.is_some_and(|old| old == path))
        })
}

pub(super) fn stops(markdown: &str, files: &[brief::File]) -> Vec<DiffLink> {
    let known = || {
        files
            .iter()
            .map(|file| (file.path.as_str(), file.old_path.as_deref()))
    };
    let mut stops: Vec<DiffLink> = Vec::new();
    for link in diff_link::extract(markdown) {
        if resolve_file(known(), &link.path).is_none() || stops.last() == Some(&link) {
            continue;
        }
        stops.push(link);
    }
    stops
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) enum Step {
    Next,
    Previous,
}

pub(super) fn step_cursor(cursor: Option<usize>, len: usize, step: Step) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let cursor = cursor.filter(|at| *at < len);
    Some(match (cursor, step) {
        (None, Step::Next) => 0,
        (None, Step::Previous) => len - 1,
        (Some(at), Step::Next) => (at + 1) % len,
        (Some(0), Step::Previous) => len - 1,
        (Some(at), Step::Previous) => at - 1,
    })
}

pub(super) fn cursor_after_click(
    stops: &[DiffLink],
    clicked: &DiffLink,
    cursor: Option<usize>,
) -> Option<usize> {
    let after = cursor.map_or(0, |at| at + 1);
    let found = stops
        .iter()
        .enumerate()
        .skip(after)
        .find(|(_, stop)| *stop == clicked)
        .or_else(|| stops.iter().enumerate().find(|(_, stop)| *stop == clicked));
    found.map(|(ix, _)| ix).or(cursor)
}

pub(super) fn position_label(cursor: Option<usize>, len: usize) -> Option<String> {
    if len == 0 {
        return None;
    }
    let at = cursor
        .filter(|at| *at < len)
        .map_or_else(|| "–".to_owned(), |at| (at + 1).to_string());
    Some(format!("{at} / {len}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) struct Target {
    pub file: usize,
    pub scroll: Option<(Side, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) enum LinkPlan {
    Open(Target),
    NotInReview,
}

pub(in crate::review) fn plan_link<'a>(
    link: &DiffLink,
    files: impl IntoIterator<Item = (&'a str, Option<&'a str>)> + Clone,
    locate: impl FnOnce(usize) -> Option<LineKind>,
) -> LinkPlan {
    let Some(file) = resolve_file(files, &link.path) else {
        return LinkPlan::NotInReview;
    };
    let scroll = link.range.and_then(|(side, _, _)| {
        let kind = locate(file)?;
        let line = match side {
            Side::New => kind.new_line(),
            Side::Old => kind.old_line(),
        }?;
        Some((side, line))
    });
    LinkPlan::Open(Target { file, scroll })
}

pub(in crate::review) fn selection_span(
    side: Side,
    start: u32,
    end_side: Side,
    end: u32,
) -> (Side, u32, u32) {
    if side == end_side {
        (side, start.min(end), start.max(end))
    } else {
        (end_side, end, end)
    }
}

pub(in crate::review) fn can_link_selection(
    commit_view: bool,
    since_mode: bool,
    side: Side,
) -> bool {
    !commit_view && !(since_mode && side == Side::Old)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) struct PendingSelection {
    pub path: String,
    pub side: Side,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub marker: String,
}

impl PendingSelection {
    pub(in crate::review) fn new(
        path: String,
        side: Side,
        start: u32,
        end: u32,
        text: &str,
    ) -> Self {
        let text = match text.char_indices().nth(MAX_SELECTION_CHARS) {
            Some((end, _)) => text[..end].to_owned(),
            None => text.to_owned(),
        };
        let marker = prompt::selection_link(&Selection {
            path: &path,
            side,
            start,
            end,
            text: &text,
        });
        Self {
            path,
            side,
            start,
            end,
            text,
            marker,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Question {
    pub shown: String,
    pub prompt: String,
}

pub(super) fn compose_question(input: &str, pending: Option<&PendingSelection>) -> Question {
    let shown = input.trim().to_owned();
    let Some(pending) = pending.filter(|pending| shown.contains(&pending.marker)) else {
        return Question {
            prompt: shown.clone(),
            shown,
        };
    };
    let question = without_marker(&shown, &pending.marker);
    let prompt = prompt::selection_question(
        &question,
        &Selection {
            path: &pending.path,
            side: pending.side,
            start: pending.start,
            end: pending.end,
            text: &pending.text,
        },
    );
    Question { shown, prompt }
}

pub(super) fn without_marker(text: &str, marker: &str) -> String {
    let horizontal = |c: char| c == ' ' || c == '\t';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at]);
        rest = &rest[at + marker.len()..];
        let kept = out.trim_end_matches(horizontal).len();
        out.truncate(kept);
        rest = rest.trim_start_matches(horizontal);
        let line_start = out.is_empty() || out.ends_with('\n');
        let newline = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'));
        match (line_start, newline) {
            (true, Some(after)) => {
                rest = after;
                if out.ends_with("\n\n") {
                    rest = rest.strip_prefix('\n').unwrap_or(rest);
                }
            }
            (false, _) if !rest.is_empty() && newline.is_none() => out.push(' '),
            _ => {}
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

pub(super) fn input_with_marker(current: &str, old: Option<&str>, marker: &str) -> String {
    if let Some(old) = old.filter(|old| current.contains(old)) {
        return current.replacen(old, marker, 1);
    }
    let mut value = current.to_owned();
    if !value.is_empty() && !value.ends_with(char::is_whitespace) {
        value.push(' ');
    }
    value.push_str(marker);
    value.push(' ');
    value
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) enum StopStep {
    NoOverview,
    NoStops,
    Go(DiffLink),
}

impl AgentPanel {
    fn latest_overview(&self) -> Option<usize> {
        self.messages.iter().rposition(|message| {
            message.role == AgentRole::Assistant && message.kind == AgentKind::Overview
        })
    }

    pub(super) fn refresh_stops(&mut self) {
        let Some(ix) = self.latest_overview() else {
            self.stops_owner = None;
            self.stop_cursor = None;
            self.stops_label = None;
            return;
        };
        let files = self
            .context
            .as_ref()
            .map_or(&[][..], |context| &context.brief.files[..]);
        let found = stops(&self.messages[ix].text, files);
        let owner = self.messages[ix].id;
        if self.stops_owner != Some(owner) {
            self.stops_owner = Some(owner);
            self.stop_cursor = None;
        }
        self.messages[ix].links = Rc::from(found);
        self.sync_stops_label();
    }

    fn sync_stops_label(&mut self) {
        let len = self.current_stops().len();
        self.stop_cursor = self.stop_cursor.filter(|at| *at < len);
        self.stops_label = position_label(self.stop_cursor, len).map(SharedString::from);
    }

    fn current_stops(&self) -> Rc<[DiffLink]> {
        self.latest_overview()
            .map(|ix| self.messages[ix].links.clone())
            .unwrap_or_default()
    }

    pub(in crate::review) fn step_stop(&mut self, step: Step, cx: &mut Context<Self>) -> StopStep {
        if self.latest_overview().is_none() {
            return StopStep::NoOverview;
        }
        let stops = self.current_stops();
        let Some(next) = step_cursor(self.stop_cursor, stops.len(), step) else {
            return StopStep::NoStops;
        };
        self.stop_cursor = Some(next);
        self.sync_stops_label();
        cx.notify();
        StopStep::Go(stops[next].clone())
    }

    pub(in crate::review) fn note_link_clicked(&mut self, link: &DiffLink, cx: &mut Context<Self>) {
        let stops = self.current_stops();
        let cursor = cursor_after_click(&stops, link, self.stop_cursor);
        if cursor != self.stop_cursor {
            self.stop_cursor = cursor;
            self.sync_stops_label();
            cx.notify();
        }
    }

    pub(in crate::review) fn ask(
        &mut self,
        selection: Option<PendingSelection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(cx);
        if let Some(selection) = selection {
            let current = self.input.read(cx).value().to_string();
            let old = self.pending.as_ref().map(|pending| pending.marker.clone());
            let value = input_with_marker(&current, old.as_deref(), &selection.marker);
            let end = value.len();
            self.pending = Some(selection);
            self.input.update(cx, |input, cx| {
                input.replace_all(value, window, cx);
                input.set_selected_range(end..end, cx);
            });
        }
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn render_stops(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label = self.stops_label.clone()?;
        let s = tr();
        let theme = cx.theme();
        Some(
            h_flex()
                .id("agent-stops")
                .flex_none()
                .gap_0p5()
                .items_center()
                .child(
                    Button::new("agent-stop-previous")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronUp)
                        .tooltip_with_action(
                            s.cmd_previous_stop.clone(),
                            &AgentPrevStop,
                            Some("Review"),
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(AgentEvent::Stop(Step::Previous));
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(
                    Button::new("agent-stop-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .tooltip_with_action(
                            s.cmd_next_stop.clone(),
                            &AgentNextStop,
                            Some("Review"),
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(AgentEvent::Stop(Step::Next));
                        })),
                )
                .into_any_element(),
        )
    }
}
