use crate::brief;
use crate::prompt::{
    COMMENTS_HEADING, INSTRUCTIONS_END, LOCAL_UPDATE, OVERVIEW, UPDATED_INSTRUCTIONS, USER_HEADING,
};

const BRIEF_START: &str = "# Review context\n\n";
const BRIEF_LINE: &str = "\n# Review context\n\n";
const COMMENTS_SECTION: &str = "## Review comments";
const NO_COMMENTS: &str = "No review comments.";
const FENCE_OPEN: &str = "<<< untrusted: ";
const FENCE_CLOSE: &str = "<<< end untrusted >>>";
const MORE_THREADS: &str = " more thread(s) not shown: the ";
const HEAD_LINE: &str = "Head commit: ";
const INSTRUCTIONS_TAIL: &str = "Be concise; use Markdown.\n";
const LEGACY_INSTRUCTIONS_END: &str = "\n---\n";
const SELECTED: &str = "Selected: ";
const MOVED_PREFIX: &str = "The review moved from ";
const MOVED_NOTE: &str = " since you last saw it. The review context above is current; if the \
                          change matters for the answer, look at what changed (git diff ";
const CHANGED_NOTE: &str = "The review changed since you last saw it. The review context above is \
                            current; if the change matters for the answer, look at what changed.";
const UPDATE_TAIL: &str = "and update the overview. Start with a short \"What changed since the \
                           last overview\" section.";
const UPDATE_GENERIC: &str = "The review changed since the last overview. Look at what changed and \
                              update the overview. Start with a short \"What changed since the last \
                              overview\" section.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShownKind {
    Overview,
    Update,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub kind: ShownKind,
    pub text: String,
    pub head: Option<String>,
}

pub fn shown(prompt: &str) -> Shown {
    let mut rest = strip_updated_instructions(prompt).unwrap_or(prompt);
    let mut head = None;
    if let Some((after, brief_head)) = strip_brief(rest) {
        rest = strip_head_moved(after);
        head = brief_head;
    } else if let Some(after) = strip_new_comments(rest) {
        rest = after;
    }
    let kind = if rest.trim_end() == OVERVIEW.trim_end() {
        ShownKind::Overview
    } else if is_update_task(rest) {
        ShownKind::Update
    } else {
        ShownKind::Chat
    };
    let text = match kind {
        ShownKind::Chat => without_selection(rest),
        ShownKind::Overview | ShownKind::Update => String::new(),
    };
    Shown { kind, text, head }
}

pub fn after_instructions<'a>(text: &'a str, current: &str) -> &'a str {
    if !current.is_empty()
        && let Some(rest) = text.strip_prefix(current)
    {
        return rest.trim_start_matches('\n');
    }
    let Some(at) = text.find(INSTRUCTIONS_TAIL) else {
        return "";
    };
    let rest = &text[at + INSTRUCTIONS_TAIL.len()..];
    if !rest.starts_with(USER_HEADING) {
        return rest.trim_start_matches('\n');
    }
    match rest.find(BRIEF_LINE) {
        Some(at) => &rest[at + 1..],
        None => "",
    }
}

fn strip_updated_instructions(text: &str) -> Option<&str> {
    let body = text.strip_prefix(UPDATED_INSTRUCTIONS)?;
    if let Some(at) = body.find(INSTRUCTIONS_END) {
        return Some(&body[at + INSTRUCTIONS_END.len()..]);
    }
    let tail = body.find(INSTRUCTIONS_TAIL)? + INSTRUCTIONS_TAIL.len() - 1;
    let at = tail + body[tail..].find(LEGACY_INSTRUCTIONS_END)?;
    Some(&body[at + LEGACY_INSTRUCTIONS_END.len()..])
}

struct Lines<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn peek(&self) -> Option<&'a str> {
        let rest = self.text.get(self.pos..).filter(|rest| !rest.is_empty())?;
        Some(rest.split_once('\n').map_or(rest, |(line, _)| line))
    }

    fn next(&mut self) -> Option<&'a str> {
        let line = self.peek()?;
        self.pos = (self.pos + line.len() + 1).min(self.text.len());
        Some(line)
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn skip_block(&mut self) -> bool {
        if !self.next().is_some_and(|line| line.starts_with(FENCE_OPEN)) {
            return false;
        }
        while let Some(line) = self.next() {
            if line == FENCE_CLOSE {
                return true;
            }
        }
        false
    }

    fn skip_more_threads(&mut self) {
        if self.peek().is_some_and(is_more_threads) {
            self.next();
        }
    }
}

fn is_more_threads(line: &str) -> bool {
    line.split_once(MORE_THREADS).is_some_and(|(count, tail)| {
        !count.is_empty()
            && count.bytes().all(|b| b.is_ascii_digit())
            && tail.ends_with(" character limit was reached.")
    })
}

fn strip_brief(text: &str) -> Option<(&str, Option<String>)> {
    if !text.starts_with(BRIEF_START) {
        return None;
    }
    let mut lines = Lines::new(text);
    let mut head = None;
    loop {
        let line = lines.peek()?;
        if line.starts_with(FENCE_OPEN) {
            if !lines.skip_block() {
                return None;
            }
            continue;
        }
        lines.next();
        if let Some(sha) = line.strip_prefix(HEAD_LINE)
            && head.is_none()
        {
            head = brief::commit_id(sha).map(str::to_owned);
        }
        if line == COMMENTS_SECTION {
            break;
        }
    }
    if !lines.next()?.is_empty() {
        return None;
    }
    let summary = lines.next()?;
    if summary != NO_COMMENTS {
        if !lines.next()?.is_empty() || !lines.skip_block() {
            return None;
        }
        lines.skip_more_threads();
    }
    let rest = lines.rest();
    Some((rest.strip_prefix('\n').unwrap_or(rest), head))
}

fn strip_new_comments(text: &str) -> Option<&str> {
    let mut lines = Lines::new(text);
    if lines.next()? != COMMENTS_HEADING || !lines.skip_block() {
        return None;
    }
    lines.skip_more_threads();
    let rest = lines.rest();
    Some(rest.strip_prefix('\n').unwrap_or(rest))
}

fn strip_head_moved(text: &str) -> &str {
    let Some((note, message)) = text.split_once("\n\n") else {
        return text;
    };
    let moved = note.strip_prefix(MOVED_PREFIX).is_some_and(|tail| {
        tail.contains(MOVED_NOTE) && tail.ends_with(").") && !tail.contains('\n')
    });
    if moved || note == CHANGED_NOTE {
        message
    } else {
        text
    }
}

fn is_update_task(text: &str) -> bool {
    let text = text.trim_end();
    if text == LOCAL_UPDATE || text == UPDATE_GENERIC {
        return true;
    }
    text.strip_prefix(MOVED_PREFIX)
        .is_some_and(|tail| tail.ends_with(UPDATE_TAIL) && !tail.contains('\n'))
}

fn without_selection(text: &str) -> String {
    let Some(body) = text.strip_suffix('\n') else {
        return text.to_owned();
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let Some((&fence, earlier)) = lines.split_last() else {
        return text.to_owned();
    };
    if fence.len() < 3 || !fence.bytes().all(|b| b == b'`') {
        return text.to_owned();
    }
    let Some(open) = earlier.iter().rposition(|line| *line == fence) else {
        return text.to_owned();
    };
    let Some(link) = open
        .checked_sub(1)
        .and_then(|ix| earlier[ix].strip_prefix(SELECTED))
    else {
        return text.to_owned();
    };
    let question = earlier[..open - 1].join("\n");
    let question = question.trim_end();
    if question.is_empty() {
        link.to_owned()
    } else {
        format!("{question}\n\n{link}")
    }
}
