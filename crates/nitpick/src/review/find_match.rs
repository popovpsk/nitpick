use std::path::PathBuf;

use git_repo::Repo;
use gpui::SharedString;
use review_core::{LineKind, Side};

use super::image_preview::read_regular_file;
use super::since_viewed::is_blob;

pub(super) const TEXT_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum TextSource {
    None,
    Blob(String),
    Disk(PathBuf),
}

impl TextSource {
    pub(super) fn read(&self, repo: &Repo) -> Vec<String> {
        let bytes = match self {
            TextSource::None => return Vec::new(),
            TextSource::Blob(id) => repo.blob(id).map_err(|err| err.to_string()),
            TextSource::Disk(path) => {
                read_regular_file(path, TEXT_LIMIT as u64).map_err(|err| err.to_string())
            }
        };
        match bytes {
            Ok(bytes) if bytes.len() <= TEXT_LIMIT => String::from_utf8_lossy(&bytes)
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
                .collect(),
            Ok(_) => Vec::new(),
            Err(err) => {
                log::warn!("find: reading the text failed: {err}");
                Vec::new()
            }
        }
    }
}

pub(super) fn text_side(blob: &str) -> TextSource {
    if is_blob(blob) {
        TextSource::Blob(blob.to_string())
    } else {
        TextSource::None
    }
}

pub(super) struct FindText {
    pub(super) key: String,
    pub(super) path: SharedString,
    pub(super) rows: Vec<LineKind>,
    pub(super) shown: [Vec<bool>; 2],
    old: Vec<String>,
    new: Vec<String>,
    old_lower: Vec<String>,
    new_lower: Vec<String>,
}

impl FindText {
    pub(super) fn new(
        key: String,
        path: SharedString,
        rows: Vec<LineKind>,
        shown: [Vec<bool>; 2],
        old: Vec<String>,
        new: Vec<String>,
    ) -> Self {
        let lower = |lines: &[String]| lines.iter().map(|line| line.to_lowercase()).collect();
        Self {
            key,
            path,
            rows,
            shown,
            old_lower: lower(&old),
            new_lower: lower(&new),
            old,
            new,
        }
    }

    pub(super) fn size(&self) -> usize {
        let sum = |lines: &[String]| lines.iter().map(|line| line.len() + 1).sum::<usize>();
        2 * (sum(&self.old) + sum(&self.new))
    }

    fn sides(&self, case: bool) -> (&[String], &[String]) {
        if case {
            (&self.old, &self.new)
        } else {
            (&self.old_lower, &self.new_lower)
        }
    }

    pub(super) fn is_shown(&self, row: LineKind) -> bool {
        let (side, line) = match row {
            LineKind::Context { new, .. } | LineKind::Added { new } => (1, new),
            LineKind::Removed { old } => (0, old),
        };
        line.checked_sub(1)
            .and_then(|line| self.shown[side].get(line as usize))
            .copied()
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hit {
    pub(super) rows: Vec<usize>,
    pub(super) side: Option<Side>,
}

impl Hit {
    pub(super) fn first(&self) -> usize {
        self.rows.first().copied().unwrap_or(0)
    }
}

pub(super) fn seed_query(selected: &str) -> Option<&str> {
    let text = match selected.strip_suffix('\n') {
        Some(text) => text.strip_suffix('\r').unwrap_or(text),
        None => selected,
    };
    (!text.trim().is_empty()).then_some(text)
}

pub(super) fn query_for(typed: &str, case: bool) -> String {
    if case {
        typed.to_owned()
    } else {
        typed.to_lowercase()
    }
}

fn line_at(lines: &[String], line: Option<u32>) -> Option<&str> {
    let ix = line?.checked_sub(1)? as usize;
    lines.get(ix).map(String::as_str)
}

fn shown_line<'a>(old: &'a [String], new: &'a [String], row: LineKind) -> Option<&'a str> {
    match row {
        LineKind::Context { new: line, .. } | LineKind::Added { new: line } => {
            line_at(new, Some(line))
        }
        LineKind::Removed { old: line } => line_at(old, Some(line)),
    }
}

pub(super) fn find_hits(text: &FindText, query: &str, case: bool) -> Vec<Hit> {
    if query.is_empty() {
        return Vec::new();
    }
    let (old, new) = text.sides(case);
    let parts: Vec<&str> = query
        .split('\n')
        .map(|part| part.strip_suffix('\r').unwrap_or(part))
        .collect();
    if let [part] = parts.as_slice() {
        return text
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| shown_line(old, new, **row).is_some_and(|line| line.contains(part)))
            .map(|(ix, _)| Hit {
                rows: vec![ix],
                side: None,
            })
            .collect();
    }
    let all = reading(&text.rows, |row| shown_line(old, new, row));
    let new_side = reading(&text.rows, |row| line_at(new, row.new_line()));
    let old_side = reading(&text.rows, |row| line_at(old, row.old_line()));
    let mut hits = Vec::new();
    for (sequence, side) in [
        (&all, None),
        (&new_side, Some(Side::New)),
        (&old_side, Some(Side::Old)),
    ] {
        for window in sequence.windows(parts.len()) {
            if spans(window, &parts) {
                hits.push(Hit {
                    rows: window.iter().map(|(ix, _)| *ix).collect(),
                    side,
                });
            }
        }
    }
    hits.sort_by_key(Hit::first);
    hits.dedup_by_key(|hit| hit.first());
    hits
}

fn reading<'a>(
    rows: &[LineKind],
    line: impl Fn(LineKind) -> Option<&'a str>,
) -> Vec<(usize, &'a str)> {
    rows.iter()
        .enumerate()
        .filter_map(|(ix, row)| Some((ix, line(*row)?)))
        .collect()
}

fn spans(lines: &[(usize, &str)], parts: &[&str]) -> bool {
    let last = parts.len() - 1;
    lines[0].1.ends_with(parts[0])
        && lines[last].1.starts_with(parts[last])
        && (1..last).all(|ix| lines[ix].1 == parts[ix])
}

pub(super) fn hit_lines(text: &FindText, hit: &Hit, old: &mut Vec<u32>, new: &mut Vec<u32>) {
    for &ix in &hit.rows {
        let Some(&row) = text.rows.get(ix) else {
            continue;
        };
        match hit.side {
            None => {
                old.extend(row.old_line());
                new.extend(row.new_line());
            }
            Some(Side::New) => new.extend(row.new_line()),
            Some(Side::Old) => old.extend(row.old_line()),
        }
    }
}

pub(super) fn start_hit(rows: &[LineKind], hits: &[usize], anchor: Option<(Side, u32)>) -> usize {
    let at = anchor.and_then(|(side, line)| {
        rows.iter().position(|row| match side {
            Side::Old => row.old_line() == Some(line),
            Side::New => row.new_line() == Some(line),
        })
    });
    at.and_then(|at| hits.iter().position(|&hit| hit >= at))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "find_match_tests.rs"]
mod tests;
