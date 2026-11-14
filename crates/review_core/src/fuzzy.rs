use std::ops::Range;

const SCORE_MATCH: i32 = 16;
const GAP_START: i32 = -3;
const GAP_EXTENSION: i32 = -1;
const BONUS_PATH: i32 = 10;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const BONUS_CONSECUTIVE: i32 = 5;
const FIRST_CHAR_MULTIPLIER: i32 = 2;
const NAME_BONUS: i32 = 2 * SCORE_MATCH;
const NONE: i32 = i32::MIN / 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub score: i32,
    pub label: Vec<usize>,
    pub detail: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Lower,
    Upper,
    Digit,
    Path,
    Separator,
}

fn class(c: char) -> Class {
    match c {
        '/' | '\\' => Class::Path,
        c if c.is_lowercase() => Class::Lower,
        c if c.is_uppercase() => Class::Upper,
        c if c.is_numeric() => Class::Digit,
        c if c.is_alphabetic() => Class::Lower,
        _ => Class::Separator,
    }
}

fn bonus(prev: Class, current: Class) -> i32 {
    match (prev, current) {
        (_, Class::Path) | (Class::Path, _) => BONUS_PATH,
        (_, Class::Separator) | (Class::Separator, _) => BONUS_BOUNDARY,
        (Class::Lower, Class::Upper) => BONUS_CAMEL,
        (Class::Lower | Class::Upper, Class::Digit) => BONUS_CAMEL,
        _ => 0,
    }
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn query_chars(query: &str) -> Vec<char> {
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(fold)
        .collect()
}

pub fn fuzzy_match(query: &str, text: &str) -> Option<FuzzyMatch> {
    match_chars(&query_chars(query), text)
}

fn match_chars(query: &[char], text: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    let mut offsets = Vec::with_capacity(text.len());
    let mut folded = Vec::with_capacity(text.len());
    let mut bonuses = Vec::with_capacity(text.len());
    let mut prev = Class::Path;
    for (offset, c) in text.char_indices() {
        let current = class(c);
        offsets.push(offset);
        folded.push(fold(c));
        bonuses.push(bonus(prev, current));
        prev = current;
    }
    let (m, n) = (query.len(), folded.len());
    if m > n || !is_subsequence(query, &folded) {
        return None;
    }

    let mut score = vec![NONE; m * n];
    let mut run = vec![0; m * n];
    let mut from = vec![usize::MAX; m * n];
    for (i, &q) in query.iter().enumerate() {
        let mut gap = NONE;
        let mut gap_from = usize::MAX;
        for j in i..n {
            if i > 0 && j > i {
                if gap > NONE {
                    gap += GAP_EXTENSION;
                }
                let k = j - 2;
                let candidate = score[(i - 1) * n + k];
                if candidate > NONE && candidate + GAP_START > gap {
                    gap = candidate + GAP_START;
                    gap_from = k;
                }
            }
            if folded[j] != q {
                continue;
            }
            let b = bonuses[j];
            let cell = i * n + j;
            if i == 0 {
                score[cell] = SCORE_MATCH + b * FIRST_CHAR_MULTIPLIER;
                run[cell] = b;
                continue;
            }
            let mut best = NONE;
            if gap > NONE {
                best = gap + SCORE_MATCH + b;
                run[cell] = b;
                from[cell] = gap_from;
            }
            let diagonal = (i - 1) * n + j - 1;
            if score[diagonal] > NONE {
                let carried = if b >= BONUS_BOUNDARY && b > run[diagonal] {
                    b
                } else {
                    run[diagonal]
                };
                let consecutive =
                    score[diagonal] + SCORE_MATCH + b.max(carried).max(BONUS_CONSECUTIVE);
                if consecutive >= best {
                    best = consecutive;
                    run[cell] = carried;
                    from[cell] = j - 1;
                }
            }
            score[cell] = best;
        }
    }

    let last = (m - 1) * n;
    let (end, best) = (m - 1..n)
        .map(|j| (j, score[last + j]))
        .filter(|&(_, s)| s > NONE)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let mut positions = vec![0; m];
    let mut j = end;
    for i in (0..m).rev() {
        positions[i] = offsets[j];
        if i > 0 {
            j = from[i * n + j];
        }
    }
    Some(FuzzyMatch {
        score: best,
        positions,
    })
}

fn is_subsequence(query: &[char], text: &[char]) -> bool {
    let mut rest = text.iter();
    query.iter().all(|q| rest.any(|c| c == q))
}

pub fn rank<'a, I>(query: &str, items: I) -> Vec<Ranked>
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let query = query_chars(query);
    if query.is_empty() {
        return items
            .into_iter()
            .enumerate()
            .map(|(index, _)| Ranked {
                index,
                score: 0,
                label: Vec::new(),
                detail: Vec::new(),
            })
            .collect();
    }
    let mut ranked: Vec<(usize, Ranked)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(index, (label, detail))| {
            let in_label = match_chars(&query, label).map(|m| Ranked {
                index,
                score: m.score + NAME_BONUS,
                label: m.positions,
                detail: Vec::new(),
            });
            let in_detail = detail
                .and_then(|detail| match_chars(&query, detail))
                .map(|m| Ranked {
                    index,
                    score: m.score,
                    label: Vec::new(),
                    detail: m.positions,
                });
            let best = match (in_label, in_detail) {
                (Some(l), Some(d)) => Some(if d.score > l.score { d } else { l }),
                (l, d) => l.or(d),
            };
            best.map(|r| (label.chars().count(), r))
        })
        .collect();
    ranked.sort_by(|(a_len, a), (b_len, b)| {
        b.score
            .cmp(&a.score)
            .then(a_len.cmp(b_len))
            .then(a.index.cmp(&b.index))
    });
    ranked.into_iter().map(|(_, r)| r).collect()
}

pub fn highlight_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &start in positions {
        let Some(c) = text.get(start..).and_then(|rest| rest.chars().next()) else {
            continue;
        };
        let end = start + c.len_utf8();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    ranges
}
