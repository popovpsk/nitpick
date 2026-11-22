use ::agent::prompt::{self, Selection};
use gpui::{Context, Window};
use review_core::diff_link::DiffLink;
use review_core::{LineKind, Side};

use super::AgentPanel;
use crate::review::comments::opens_in_browser;

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::review) struct SideText {
    pub(in crate::review) text: String,
    pub(in crate::review) numbers: Vec<u32>,
}

impl SideText {
    fn push(&mut self, number: u32, line: &str) {
        if !self.numbers.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(line);
        self.numbers.push(number);
    }
}

fn hunk_start(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let parse = |range: &str| -> Option<(u32, u32)> {
        match range.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((range.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = parse(old)?;
    let (new_start, new_count) = parse(new)?;
    Some((old_start, old_count, new_start, new_count))
}

pub(in crate::review) fn patch_sides(section: &str) -> (SideText, SideText) {
    let mut old = SideText::default();
    let mut new = SideText::default();
    let mut lines = section.split('\n').map(|line| line.trim_end_matches('\r'));
    while let Some(line) = lines.next() {
        let Some((old_start, old_count, new_start, new_count)) = hunk_start(line) else {
            continue;
        };
        let (mut old_no, mut old_left) = (old_start, old_count);
        let (mut new_no, mut new_left) = (new_start, new_count);
        while old_left > 0 || new_left > 0 {
            let Some(line) = lines.next() else {
                break;
            };
            match line.as_bytes().first() {
                Some(b'-') if old_left > 0 => {
                    old.push(old_no, &line[1..]);
                    old_no += 1;
                    old_left -= 1;
                }
                Some(b'+') if new_left > 0 => {
                    new.push(new_no, &line[1..]);
                    new_no += 1;
                    new_left -= 1;
                }
                Some(b'\\') => {}
                Some(b' ') | None if old_left > 0 && new_left > 0 => {
                    let text = line.get(1..).unwrap_or("");
                    old.push(old_no, text);
                    new.push(new_no, text);
                    old_no += 1;
                    new_no += 1;
                    old_left -= 1;
                    new_left -= 1;
                }
                _ => break,
            }
        }
    }
    (old, new)
}

const MAX_CANDIDATES: usize = 1_000_000;

fn candidates(side: Side, shown: &SideText, text: &str) -> Vec<(Side, u32, u32)> {
    let breaks = text.matches('\n').count();
    let mut found = Vec::new();
    let (mut counted_to, mut first) = (0, 0);
    for (offset, _) in shown.text.match_indices(text).take(MAX_CANDIDATES) {
        first += shown.text[counted_to..offset].matches('\n').count();
        counted_to = offset;
        let (Some(&first_no), Some(&last_no)) =
            (shown.numbers.get(first), shown.numbers.get(first + breaks))
        else {
            continue;
        };
        found.push((side, first_no, last_no));
    }
    found
}

pub(in crate::review) fn locate_selection(
    old: &SideText,
    new: &SideText,
    text: &str,
    hovered: Option<(Side, u32)>,
) -> Option<(Side, u32, u32)> {
    let text = text.replace("\r\n", "\n");
    let text = text.trim_matches('\n');
    if text.is_empty() {
        return None;
    }
    let mut found = candidates(Side::New, new, text);
    found.extend(candidates(Side::Old, old, text));
    match hovered {
        Some((side, line)) => found.into_iter().min_by_key(|&(found_side, first, last)| {
            let distance = if line < first {
                first - line
            } else {
                line.saturating_sub(last)
            };
            (found_side != side, distance)
        }),
        None => found.into_iter().next(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) struct AskInputs {
    pub gutter: Option<(usize, Side, u32, Side, u32)>,
    pub has_text: bool,
    pub text_file: Option<usize>,
    pub commit_view: bool,
    pub interdiff: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) enum AskPlan {
    OpenOnly,
    Refuse,
    Attach {
        file: usize,
        range: Option<(Side, u32, u32)>,
    },
    Locate {
        file: usize,
    },
}

pub(in crate::review) fn plan_ask(inputs: &AskInputs) -> AskPlan {
    if let Some((file, side, start, end_side, end)) = inputs.gutter {
        if inputs.commit_view {
            return AskPlan::Refuse;
        }
        let range = selection_span(side, start, end_side, end);
        let linkable = can_link_selection(inputs.commit_view, inputs.interdiff, range.0);
        return AskPlan::Attach {
            file,
            range: linkable.then_some(range),
        };
    }
    let Some(file) = inputs.text_file.filter(|_| inputs.has_text) else {
        return AskPlan::OpenOnly;
    };
    if inputs.commit_view {
        AskPlan::Refuse
    } else if inputs.interdiff {
        AskPlan::Attach { file, range: None }
    } else {
        AskPlan::Locate { file }
    }
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
    pub range: Option<(Side, u32, u32)>,
    pub text: String,
    pub marker: String,
}

impl PendingSelection {
    pub(in crate::review) fn new(
        path: String,
        range: Option<(Side, u32, u32)>,
        text: &str,
    ) -> Self {
        let text = match text.char_indices().nth(MAX_SELECTION_CHARS) {
            Some((end, _)) => text[..end].to_owned(),
            None => text.to_owned(),
        };
        let marker = prompt::selection_link(&Selection {
            path: &path,
            range,
            text: &text,
        });
        Self {
            path,
            range,
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
            range: pending.range,
            text: &pending.text,
        },
    );
    Question { shown, prompt }
}

fn line_break_len(text: &str) -> usize {
    if text.ends_with("\r\n") {
        2
    } else {
        usize::from(text.ends_with('\n'))
    }
}

fn ends_with_blank_line(text: &str) -> bool {
    let last = line_break_len(text);
    last > 0 && line_break_len(&text[..text.len() - last]) > 0
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
                if ends_with_blank_line(&out) {
                    rest = rest
                        .strip_prefix("\r\n")
                        .or_else(|| rest.strip_prefix('\n'))
                        .unwrap_or(rest);
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

impl AgentPanel {
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
        cx.defer_in(window, |this, window, cx| {
            this.input.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.notify();
    }
}
