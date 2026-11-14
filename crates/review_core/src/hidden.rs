use std::ops::Range;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub pattern: String,
    pub message: String,
}

pub struct HideRules {
    matcher: Option<Gitignore>,
}

impl HideRules {
    pub fn new(global: &[String], repository: &[String]) -> (HideRules, Vec<RuleError>) {
        let mut builder = GitignoreBuilder::new("");
        let mut errors = Vec::new();
        let mut any = false;
        for pattern in global.iter().chain(repository) {
            let line = pattern.trim_end();
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            match builder.add_line(None, line) {
                Ok(_) => any = true,
                Err(err) => errors.push(RuleError {
                    pattern: line.to_string(),
                    message: err.to_string(),
                }),
            }
        }
        let matcher = if any {
            match builder.build() {
                Ok(matcher) => Some(matcher),
                Err(err) => {
                    errors.push(RuleError {
                        pattern: String::new(),
                        message: err.to_string(),
                    });
                    None
                }
            }
        } else {
            None
        };
        (HideRules { matcher }, errors)
    }

    pub fn is_empty(&self) -> bool {
        self.matcher.is_none()
    }

    pub fn matching_rule(&self, path: &str) -> Option<String> {
        let matcher = self.matcher.as_ref()?;
        let path = path.trim_start_matches('/');
        if path.is_empty() || std::path::Path::new(path).has_root() {
            return None;
        }
        match matcher.matched_path_or_any_parents(path, false) {
            ignore::Match::Ignore(glob) => Some(glob.original().to_string()),
            _ => None,
        }
    }

    pub fn is_hidden(&self, path: &str) -> bool {
        self.matching_rule(path).is_some()
    }
}

pub fn escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len() + 2);
    for (ix, c) in literal.chars().enumerate() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '{' | '}')
            || (ix == 0 && matches!(c, '!' | '#'))
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn file_pattern(path: &str) -> String {
    format!("/{}", escape(path.trim_matches('/')))
}

pub fn dir_pattern(path: &str) -> String {
    format!("/{}/", escape(path.trim_matches('/')))
}

pub fn dir_name_pattern(path: &str) -> String {
    let name = path
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    format!("{}/", escape(name))
}

pub fn family_pattern(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = name.split_once('.').filter(|(stem, _)| !stem.is_empty())?;
    let suffix = match stem.rfind(['_', '-']) {
        Some(at) if !ext.contains('.') && at > 0 && at + 1 < stem.len() => {
            format!("{}.{ext}", &stem[at..])
        }
        _ => format!(".{ext}"),
    };
    Some(format!("*{}", escape(&suffix)))
}

pub fn merge_patterns(list: &mut Vec<String>, baseline: &[String], typed: &[String]) {
    if list.as_slice() == baseline {
        *list = typed.to_vec();
        return;
    }
    list.retain(|p| typed.contains(p) || !baseline.contains(p));
    for pattern in typed {
        if !list.contains(pattern) {
            list.push(pattern.clone());
        }
    }
}

pub fn matched_ranges(pattern: &str, path: &str) -> Vec<Range<usize>> {
    let pattern = pattern.trim_end();
    if pattern.starts_with('!') || pattern.starts_with('#') {
        return Vec::new();
    }
    let dir_only = pattern.ends_with('/') && !pattern.ends_with("\\/");
    let pattern = pattern
        .strip_suffix('/')
        .filter(|_| dir_only)
        .unwrap_or(pattern);
    let anchored = pattern.starts_with('/') || pattern.contains('/');
    let pattern = pattern.trim_start_matches('/');
    let segments: Vec<&str> = pattern.split('/').collect();
    let mut components = Vec::new();
    let mut start = 0;
    for part in path.split('/') {
        components.push((start, part));
        start += part.len() + 1;
    }
    let last = components.len().saturating_sub(1);
    let mut positions = Vec::new();
    if anchored {
        if let Some(found) = match_anchored(&segments, &components, dir_only) {
            positions = found;
        }
    } else {
        let candidates = components
            .iter()
            .enumerate()
            .rev()
            .filter(|(ix, _)| !dir_only || *ix < last);
        for (_, (offset, component)) in candidates {
            if let Some(found) = match_segment(segments[0], component) {
                positions = found.into_iter().map(|p| p + offset).collect();
                break;
            }
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for at in positions {
        let len = path[at..].chars().next().map_or(1, char::len_utf8);
        match ranges.last_mut() {
            Some(range) if range.end == at => range.end = at + len,
            _ => ranges.push(at..at + len),
        }
    }
    ranges
}

fn match_anchored(
    segments: &[&str],
    components: &[(usize, &str)],
    dir_only: bool,
) -> Option<Vec<usize>> {
    match segments.split_first() {
        None => Some(Vec::new()),
        Some((&"**", rest)) => (0..=components.len())
            .find_map(|skip| match_anchored(rest, &components[skip..], dir_only)),
        Some((segment, rest)) => {
            let ((offset, component), remaining) = components.split_first()?;
            if rest.is_empty() && dir_only && remaining.is_empty() {
                return None;
            }
            let found = match_segment(segment, component)?;
            let mut tail = match_anchored(rest, remaining, dir_only)?;
            let mut out: Vec<usize> = found.into_iter().map(|p| p + offset).collect();
            out.append(&mut tail);
            Some(out)
        }
    }
}

fn match_segment(segment: &str, text: &str) -> Option<Vec<usize>> {
    let pattern: Vec<char> = segment.chars().collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut literal = Vec::new();
    glob(&pattern, 0, &chars, 0, &mut literal).then_some(literal)
}

fn glob(
    pattern: &[char],
    pi: usize,
    text: &[(usize, char)],
    ti: usize,
    literal: &mut Vec<usize>,
) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }
    match pattern[pi] {
        '*' => {
            let next = pattern[pi..].iter().take_while(|c| **c == '*').count();
            (ti..=text.len()).any(|skip| {
                let mark = literal.len();
                glob(pattern, pi + next, text, skip, literal) || {
                    literal.truncate(mark);
                    false
                }
            })
        }
        '?' => ti < text.len() && glob(pattern, pi + 1, text, ti + 1, literal),
        '[' => {
            let Some(&(_, c)) = text.get(ti) else {
                return false;
            };
            match class(pattern, pi, c) {
                Some((true, end)) => glob(pattern, end, text, ti + 1, literal),
                Some((false, _)) => false,
                None => literal_step('[', pattern, pi, text, ti, literal),
            }
        }
        '\\' if pi + 1 < pattern.len() => {
            let c = pattern[pi + 1];
            match text.get(ti) {
                Some(&(at, t)) if t == c => {
                    literal.push(at);
                    glob(pattern, pi + 2, text, ti + 1, literal) || {
                        literal.pop();
                        false
                    }
                }
                _ => false,
            }
        }
        c => literal_step(c, pattern, pi, text, ti, literal),
    }
}

fn literal_step(
    c: char,
    pattern: &[char],
    pi: usize,
    text: &[(usize, char)],
    ti: usize,
    literal: &mut Vec<usize>,
) -> bool {
    match text.get(ti) {
        Some(&(at, t)) if t == c => {
            literal.push(at);
            glob(pattern, pi + 1, text, ti + 1, literal) || {
                literal.pop();
                false
            }
        }
        _ => false,
    }
}

fn class(pattern: &[char], pi: usize, c: char) -> Option<(bool, usize)> {
    let mut i = pi + 1;
    let negated = matches!(pattern.get(i), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < pattern.len() {
        let current = pattern[i];
        if current == ']' && !first {
            return Some((matched != negated, i + 1));
        }
        first = false;
        if pattern.get(i + 1) == Some(&'-') && pattern.get(i + 2).is_some_and(|e| *e != ']') {
            let end = pattern[i + 2];
            matched |= current <= c && c <= end;
            i += 3;
        } else {
            matched |= current == c;
            i += 1;
        }
    }
    None
}
