pub fn marker_lines<K: PartialEq>(keys: &[K], block_starts: &[u32]) -> Vec<u32> {
    let mut starts = block_starts.iter().copied().peekable();
    let mut lines = Vec::new();
    for (ix, key) in keys.iter().enumerate() {
        let line = ix as u32 + 1;
        let mut block_start = false;
        while let Some(start) = starts.next_if(|&start| start <= line) {
            block_start |= start == line;
        }
        if ix == 0 || block_start || keys[ix - 1] != *key {
            lines.push(line);
        }
    }
    lines
}

pub fn initials(name: &str) -> String {
    let name = name.trim();
    let name = match name.split_once('@') {
        Some((local, _)) if !name.contains(char::is_whitespace) => local,
        _ => name,
    };
    let mut firsts = name
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-'))
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    let Some(first) = firsts.next() else {
        return "?".to_string();
    };
    first
        .to_uppercase()
        .chain(firsts.next_back().into_iter().flat_map(char::to_uppercase))
        .collect()
}

pub fn tint_index(commit_id: &str, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let hash = commit_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    (hash % count as u64) as usize
}
