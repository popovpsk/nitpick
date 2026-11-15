use std::collections::HashMap;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ViewedStore {
    viewed: HashMap<String, HashMap<String, i64>>,
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
        if viewed {
            self.viewed
                .entry(path.to_string())
                .or_default()
                .insert(fingerprint.to_string(), now);
        } else if let Some(marks) = self.viewed.get_mut(path) {
            marks.remove(fingerprint);
            if marks.is_empty() {
                self.viewed.remove(path);
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
