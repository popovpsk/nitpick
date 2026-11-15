use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ViewedStore {
    viewed: HashMap<String, HashMap<String, i64>>,
    into: HashMap<String, HashSet<(String, String)>>,
}

pub fn fingerprint(old_blob: &str, new_blob: &str) -> String {
    format!("{old_blob}..{new_blob}")
}

pub fn fingerprint_blobs(fingerprint: &str) -> Option<(&str, &str)> {
    fingerprint.split_once("..")
}

impl ViewedStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_marks<'a>(marks: impl IntoIterator<Item = (&'a str, &'a str, i64)>) -> Self {
        let mut store = Self::new();
        for (path, fingerprint, at) in marks {
            store.set_viewed(path, fingerprint, true, at);
        }
        store
    }

    pub fn is_viewed(&self, path: &str, fingerprint: &str) -> bool {
        self.is_viewed_exactly(path, fingerprint)
            || fingerprint_blobs(fingerprint).is_some_and(|(old, new)| self.chained(old, new))
    }

    fn chained(&self, old: &str, new: &str) -> bool {
        if old == new {
            return false;
        }
        let mut seen: HashSet<&str> = HashSet::from([new]);
        let mut queue = vec![new];
        while let Some(blob) = queue.pop() {
            for (from, _) in self.into.get(blob).into_iter().flatten() {
                if blob == new && from == old {
                    continue;
                }
                if from == old {
                    return true;
                }
                if seen.insert(from) {
                    queue.push(from);
                }
            }
        }
        false
    }

    pub fn covering_marks<'a>(&'a self, path: &str, fingerprint: &'a str) -> Vec<(String, String)> {
        let mut marks = Vec::new();
        if self.is_viewed_exactly(path, fingerprint) {
            marks.push((path.to_string(), fingerprint.to_string()));
        }
        let Some((old, new)) = fingerprint_blobs(fingerprint) else {
            return marks;
        };
        if !self.chained(old, new) {
            return marks;
        }
        let edges = self
            .viewed
            .iter()
            .flat_map(|(p, fps)| fps.keys().map(move |fp| (p.as_str(), fp.as_str())))
            .filter_map(|(p, fp)| fingerprint_blobs(fp).map(|(a, b)| (p, fp, a, b)))
            .filter(|&(p, _, a, b)| (!(a == old && b == new) || p == path) && b != old && a != new)
            .collect::<Vec<_>>();
        let mut forward: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut backward: HashMap<&str, Vec<&str>> = HashMap::new();
        for &(_, _, a, b) in &edges {
            forward.entry(a).or_default().push(b);
            backward.entry(b).or_default().push(a);
        }
        let reach = |start: &'a str, next: &HashMap<&'a str, Vec<&'a str>>| {
            let mut seen: HashSet<&str> = HashSet::from([start]);
            let mut queue = vec![start];
            while let Some(blob) = queue.pop() {
                for &to in next.get(blob).into_iter().flatten() {
                    if seen.insert(to) {
                        queue.push(to);
                    }
                }
            }
            seen
        };
        let after_old = reach(old, &forward);
        let before_new = reach(new, &backward);
        for &(p, fp, a, b) in &edges {
            if after_old.contains(a) && before_new.contains(b) && !(p == path && fp == fingerprint)
            {
                marks.push((p.to_string(), fp.to_string()));
            }
        }
        marks
    }

    fn is_viewed_exactly(&self, path: &str, fingerprint: &str) -> bool {
        self.viewed
            .get(path)
            .is_some_and(|marks| marks.contains_key(fingerprint))
    }

    pub fn viewed_versions_except(&self, path: &str, fingerprint: &str) -> Vec<&str> {
        let mut versions: Vec<(&str, i64)> = self
            .viewed
            .get(path)
            .into_iter()
            .flatten()
            .filter(|(mark, _)| mark.as_str() != fingerprint)
            .map(|(mark, at)| (mark.as_str(), *at))
            .collect();
        versions.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        versions.into_iter().map(|(mark, _)| mark).collect()
    }

    pub fn set_viewed(&mut self, path: &str, fingerprint: &str, viewed: bool, now: i64) {
        let edge = fingerprint_blobs(fingerprint)
            .map(|(old, new)| (new.to_string(), (old.to_string(), path.to_string())));
        if viewed {
            self.viewed
                .entry(path.to_string())
                .or_default()
                .insert(fingerprint.to_string(), now);
            if let Some((new, from)) = edge {
                self.into.entry(new).or_default().insert(from);
            }
        } else if let Some(marks) = self.viewed.get_mut(path) {
            marks.remove(fingerprint);
            if marks.is_empty() {
                self.viewed.remove(path);
            }
            if let Some((new, from)) = edge
                && let Some(sources) = self.into.get_mut(&new)
            {
                sources.remove(&from);
                if sources.is_empty() {
                    self.into.remove(&new);
                }
            }
        }
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
