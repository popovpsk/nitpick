use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context { old: u32, new: u32 },
    Added { new: u32 },
    Removed { old: u32 },
}

impl LineKind {
    pub fn old_line(self) -> Option<u32> {
        match self {
            LineKind::Context { old, .. } | LineKind::Removed { old } => Some(old),
            LineKind::Added { .. } => None,
        }
    }

    pub fn new_line(self) -> Option<u32> {
        match self {
            LineKind::Context { new, .. } | LineKind::Added { new } => Some(new),
            LineKind::Removed { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub first: LineKind,
    pub anchor: LineKind,
}

const ABSENT: u32 = 0;
const CHANGED: u32 = u32::MAX;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineMap {
    old_to_new: Vec<u32>,
    new_to_old: Vec<u32>,
    changes: Vec<Change>,
}

impl LineMap {
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn locate(&self, side: Side, line: u32) -> Option<LineKind> {
        let (map, ix) = match side {
            Side::Old => (&self.old_to_new, line),
            Side::New => (&self.new_to_old, line),
        };
        let other = *map.get(ix.checked_sub(1)? as usize)?;
        match (side, other) {
            (_, ABSENT) => None,
            (Side::Old, CHANGED) => Some(LineKind::Removed { old: line }),
            (Side::New, CHANGED) => Some(LineKind::Added { new: line }),
            (Side::Old, new) => Some(LineKind::Context { old: line, new }),
            (Side::New, old) => Some(LineKind::Context { old, new: line }),
        }
    }

    pub fn new_line_for_old(&self, old: u32) -> Option<u32> {
        self.locate(Side::Old, old)?.new_line()
    }

    pub fn near_change(&self, line: LineKind, context: u32) -> bool {
        let LineKind::Context { old, new } = line else {
            return true;
        };
        let removed = (old.saturating_sub(context)..=old.saturating_add(context))
            .any(|o| matches!(self.locate(Side::Old, o), Some(LineKind::Removed { .. })));
        removed
            || (new.saturating_sub(context)..=new.saturating_add(context))
                .any(|n| matches!(self.locate(Side::New, n), Some(LineKind::Added { .. })))
    }

    pub fn is_empty(&self) -> bool {
        self.old_to_new.is_empty() && self.new_to_old.is_empty()
    }

    pub fn rows(&self) -> Vec<LineKind> {
        let (old, new) = (&self.old_to_new, &self.new_to_old);
        let mut rows = Vec::with_capacity(old.len().max(new.len()));
        let (mut o, mut n) = (0, 0);
        loop {
            if o < old.len() && old[o] == ABSENT {
                o += 1;
            } else if n < new.len() && new[n] == ABSENT {
                n += 1;
            } else if o < old.len() && old[o] == CHANGED {
                rows.push(LineKind::Removed { old: o as u32 + 1 });
                o += 1;
            } else if n < new.len() && new[n] == CHANGED {
                rows.push(LineKind::Added { new: n as u32 + 1 });
                n += 1;
            } else if o < old.len() && n < new.len() {
                rows.push(LineKind::Context {
                    old: o as u32 + 1,
                    new: n as u32 + 1,
                });
                o += 1;
                n += 1;
            } else if o < old.len() {
                o += 1;
            } else if n < new.len() {
                n += 1;
            } else {
                return rows;
            }
        }
    }

    pub fn shown_block_starts(
        &self,
        side: Side,
        context: Option<usize>,
        min_folded: usize,
    ) -> Vec<u32> {
        let shown = self.shown_lines(side, context, min_folded);
        (0..shown.len())
            .filter(|&ix| shown[ix] && (ix == 0 || !shown[ix - 1]))
            .map(|ix| ix as u32 + 1)
            .collect()
    }

    pub fn shown_lines(&self, side: Side, context: Option<usize>, min_folded: usize) -> Vec<bool> {
        let (own, other) = match side {
            Side::Old => (&self.old_to_new, &self.new_to_old),
            Side::New => (&self.new_to_old, &self.old_to_new),
        };
        let count = own.len();
        let mut shown: Vec<bool> = match context {
            None => own.iter().map(|value| *value != ABSENT).collect(),
            Some(context) => {
                let mut bounds = vec![0isize; count + 1];
                let mut window = |start: usize, end: usize| {
                    let start = start.saturating_sub(context);
                    let end = end.saturating_add(context).min(count);
                    if start < end {
                        bounds[start] += 1;
                        bounds[end] -= 1;
                    }
                };
                let mut ix = 0;
                while ix < count {
                    if own[ix] != CHANGED {
                        ix += 1;
                        continue;
                    }
                    let start = ix;
                    while ix < count && own[ix] == CHANGED {
                        ix += 1;
                    }
                    window(start, ix);
                }
                let mut next = 0;
                for value in other {
                    match *value {
                        ABSENT => {}
                        CHANGED => window(next, next),
                        line => next = line as usize,
                    }
                }
                let mut active = 0;
                own.iter()
                    .zip(&bounds)
                    .map(|(value, bound)| {
                        active += bound;
                        active > 0 && *value != ABSENT
                    })
                    .collect()
            }
        };
        let mut ix = 0;
        while ix < count {
            if shown[ix] || own[ix] == ABSENT {
                ix += 1;
                continue;
            }
            let start = ix;
            while ix < count && !shown[ix] && own[ix] != ABSENT {
                ix += 1;
            }
            if ix - start < min_folded {
                shown[start..ix].fill(true);
            }
        }
        shown
    }

    fn set(map: &mut Vec<u32>, line: u32, value: u32) {
        let ix = line.saturating_sub(1) as usize;
        if map.len() <= ix {
            map.resize(ix + 1, ABSENT);
        }
        map[ix] = value;
    }
}

pub fn file_spans(patch: &str) -> Vec<Range<usize>> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let ends = starts.iter().skip(1).copied().chain([patch.len()]);
    starts
        .iter()
        .zip(ends)
        .map(|(&start, end)| start..end)
        .collect()
}

pub fn line_maps(patch: &str) -> Vec<LineMap> {
    let mut maps = Vec::new();
    let mut current: Option<LineMap> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;
    let mut old_left = 0u32;
    let mut new_left = 0u32;
    let mut in_change = false;
    let mut before: [Option<LineKind>; 2] = [None, None];
    for line in patch.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if old_left > 0 || new_left > 0 {
            let Some(map) = current.as_mut() else {
                old_left = 0;
                new_left = 0;
                continue;
            };
            match line.as_bytes().first() {
                Some(b'+') if new_left > 0 => {
                    if !in_change {
                        start_change(map, LineKind::Added { new: new_line }, &mut before);
                        in_change = true;
                    }
                    LineMap::set(&mut map.new_to_old, new_line, CHANGED);
                    new_line += 1;
                    new_left -= 1;
                }
                Some(b'-') if old_left > 0 => {
                    if !in_change {
                        start_change(map, LineKind::Removed { old: old_line }, &mut before);
                        in_change = true;
                    }
                    LineMap::set(&mut map.old_to_new, old_line, CHANGED);
                    old_line += 1;
                    old_left -= 1;
                }
                Some(b'\\') => {}
                Some(b' ') | None if old_left > 0 && new_left > 0 => {
                    in_change = false;
                    before = [
                        before[1],
                        Some(LineKind::Context {
                            old: old_line,
                            new: new_line,
                        }),
                    ];
                    LineMap::set(&mut map.old_to_new, old_line, new_line);
                    LineMap::set(&mut map.new_to_old, new_line, old_line);
                    old_line += 1;
                    new_line += 1;
                    old_left -= 1;
                    new_left -= 1;
                }
                _ => {
                    old_left = 0;
                    new_left = 0;
                }
            }
            if old_left > 0 || new_left > 0 {
                continue;
            }
            if line.starts_with(['+', '-', ' ', '\\']) || line.is_empty() {
                continue;
            }
        }
        in_change = false;
        before = [None, None];
        if line.starts_with("diff --git ") {
            if let Some(map) = current.take() {
                maps.push(map);
            }
            current = Some(LineMap::default());
        } else if line.starts_with("@@ ")
            && let Some((old, new)) = parse_hunk_header(line)
        {
            if current.is_none() {
                current = Some(LineMap::default());
            }
            (old_line, old_left) = old;
            (new_line, new_left) = new;
            if old_line == 0 {
                old_line = 1;
            }
            if new_line == 0 {
                new_line = 1;
            }
        }
    }
    if let Some(map) = current.take() {
        maps.push(map);
    }
    maps
}

fn start_change(map: &mut LineMap, first: LineKind, before: &mut [Option<LineKind>; 2]) {
    let anchor = before[0].or(before[1]).unwrap_or(first);
    map.changes.push(Change { first, anchor });
    *before = [None, None];
}

fn parse_hunk_header(line: &str) -> Option<((u32, u32), (u32, u32))> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    Some((parse_range(old)?, parse_range(new)?))
}

fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}
