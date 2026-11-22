use crate::line_map::{LineKind, LineMap, Side};

const SCHEME: &str = "diff:";
const MAX_SCAN_LINES: u32 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLink {
    pub path: String,
    pub range: Option<(Side, u32, u32)>,
}

impl DiffLink {
    pub fn locate(&self, map: &LineMap) -> Option<LineKind> {
        let (side, first, last) = self.range?;
        let last = last.min(first.saturating_add(MAX_SCAN_LINES));
        (first..=last).find_map(|line| map.locate(side, line))
    }
}

pub fn parse(link: &str) -> Option<DiffLink> {
    let head = link.get(..SCHEME.len())?;
    if !head.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let rest = &link[SCHEME.len()..];
    let (path, fragment) = match rest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (rest, None),
    };
    let path = decode(path)?;
    if path.is_empty() {
        return None;
    }
    let range = match fragment {
        Some(fragment) => Some(parse_range(fragment)?),
        None => None,
    };
    Some(DiffLink { path, range })
}

pub fn format(link: &DiffLink) -> String {
    let mut out = String::from(SCHEME);
    encode_into(&link.path, &mut out);
    if let Some((side, first, last)) = link.range {
        let letter = side_letter(side);
        out.push('#');
        out.push(letter);
        out.push_str(&first.to_string());
        if last != first {
            out.push('-');
            out.push(letter);
            out.push_str(&last.to_string());
        }
    }
    out
}

pub fn extract(markdown: &str) -> Vec<DiffLink> {
    let mut links = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if let Some((mark, length)) = fence {
            if closes_fence(trimmed, mark, length) {
                fence = None;
            }
            continue;
        }
        if let Some(opened) = opens_fence(trimmed) {
            fence = Some(opened);
            continue;
        }
        scan_line(line.as_bytes(), &mut links);
    }
    links
}

fn side_letter(side: Side) -> char {
    match side {
        Side::Old => 'L',
        Side::New => 'R',
    }
}

fn letter_side(letter: u8) -> Option<Side> {
    match letter {
        b'L' => Some(Side::Old),
        b'R' => Some(Side::New),
        _ => None,
    }
}

fn parse_line_number(digits: &str) -> Option<u32> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|line| *line > 0)
}

fn parse_range(fragment: &str) -> Option<(Side, u32, u32)> {
    let bytes = fragment.as_bytes();
    let side = letter_side(*bytes.first()?)?;
    let body = &fragment[1..];
    let (first, last) = match body.split_once('-') {
        None => {
            let line = parse_line_number(body)?;
            (line, line)
        }
        Some((first, last)) => {
            let last = match last.as_bytes().first() {
                Some(letter) if letter.is_ascii_alphabetic() => {
                    if letter_side(*letter)? != side {
                        return None;
                    }
                    &last[1..]
                }
                _ => last,
            };
            (parse_line_number(first)?, parse_line_number(last)?)
        }
    };
    Some((side, first.min(last), first.max(last)))
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

fn encode_into(path: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in path.as_bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 15) as usize] as char);
        }
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = hex_value(*bytes.get(i + 1)?)?;
            let low = hex_value(*bytes.get(i + 2)?)?;
            out.push(high << 4 | low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn opens_fence(trimmed: &str) -> Option<(u8, usize)> {
    let mark = *trimmed.as_bytes().first()?;
    if mark != b'`' && mark != b'~' {
        return None;
    }
    let length = trimmed.bytes().take_while(|b| *b == mark).count();
    (length >= 3).then_some((mark, length))
}

fn closes_fence(trimmed: &str, mark: u8, length: usize) -> bool {
    let run = trimmed.bytes().take_while(|b| *b == mark).count();
    run >= length && trimmed[run..].trim().is_empty()
}

fn scan_line(bytes: &[u8], links: &mut Vec<DiffLink>) {
    let mut i = 0;
    let mut open = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => i = skip_code_span(bytes, i),
            b'[' => {
                open += 1;
                i += 1;
            }
            b']' if open > 0 => {
                open -= 1;
                if bytes.get(i + 1) == Some(&b'(') {
                    let (target, next) = destination(bytes, i + 2);
                    links.extend(target.as_deref().and_then(parse));
                    i = next;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

fn backtick_run(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|b| **b == b'`').count()
}

fn skip_code_span(bytes: &[u8], open: usize) -> usize {
    let length = backtick_run(bytes, open);
    let mut j = open + length;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let run = backtick_run(bytes, j);
            if run == length {
                return j + run;
            }
            j += run;
        } else {
            j += 1;
        }
    }
    open + length
}

fn destination(bytes: &[u8], mut i: usize) -> (Option<String>, usize) {
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    let mut out = Vec::new();
    if bytes.get(i) == Some(&b'<') {
        i += 1;
        while let Some(&byte) = bytes.get(i) {
            match byte {
                b'>' => return (String::from_utf8(out).ok(), i + 1),
                b'<' => break,
                b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                    out.push(bytes[i + 1]);
                    i += 2;
                }
                _ => {
                    out.push(byte);
                    i += 1;
                }
            }
        }
        return (None, i);
    }
    let mut depth = 0usize;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b' ' | b'\t' => break,
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            _ => {}
        }
        out.push(byte);
        i += 1;
    }
    (String::from_utf8(out).ok(), i)
}
