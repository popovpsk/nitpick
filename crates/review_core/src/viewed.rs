use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FORMAT_VERSION: u32 = 1;
const RETENTION_SECS: i64 = 180 * 24 * 60 * 60;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    viewed: BTreeMap<String, BTreeMap<String, i64>>,
}

#[derive(Debug, Clone)]
pub struct ViewedStore {
    path: Option<PathBuf>,
    data: StoreFile,
}

pub fn fingerprint(old_blob: &str, new_blob: &str) -> String {
    format!("{old_blob}..{new_blob}")
}

pub fn fingerprint_blobs(fingerprint: &str) -> Option<(&str, &str)> {
    fingerprint.split_once("..")
}

impl ViewedStore {
    pub fn in_memory() -> Self {
        Self {
            path: None,
            data: StoreFile {
                version: FORMAT_VERSION,
                ..StoreFile::default()
            },
        }
    }

    pub fn file_for(dir: &Path, repo_root: &Path) -> PathBuf {
        let root = repo_root.to_string_lossy();
        let name = repo_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        dir.join(format!("{safe}-{:016x}.json", fnv1a(root.as_bytes())))
    }

    pub fn load(dir: &Path, repo_root: &Path, now: i64) -> Self {
        let path = Self::file_for(dir, repo_root);
        let mut data = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<StoreFile>(&bytes) {
                Ok(data) => data,
                Err(err) => {
                    log::warn!(
                        "viewed store {}: unreadable, starting empty: {err}",
                        path.display()
                    );
                    StoreFile::default()
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => StoreFile::default(),
            Err(err) => {
                log::warn!("viewed store {}: {err}", path.display());
                StoreFile::default()
            }
        };
        data.version = FORMAT_VERSION;
        data.repo = repo_root.to_string_lossy().into_owned();
        data.viewed.retain(|_, marks| {
            marks.retain(|_, at| now - *at < RETENTION_SECS);
            !marks.is_empty()
        });
        Self {
            path: Some(path),
            data,
        }
    }

    pub fn is_viewed(&self, path: &str, fingerprint: &str) -> bool {
        self.data
            .viewed
            .get(path)
            .is_some_and(|marks| marks.contains_key(fingerprint))
    }

    pub fn last_viewed_except(&self, path: &str, fingerprint: &str) -> Option<&str> {
        self.data
            .viewed
            .get(path)?
            .iter()
            .filter(|(mark, _)| mark.as_str() != fingerprint)
            .max_by_key(|(_, at)| **at)
            .map(|(mark, _)| mark.as_str())
    }

    pub fn set_viewed(&mut self, path: &str, fingerprint: &str, viewed: bool, now: i64) {
        if viewed {
            self.data
                .viewed
                .entry(path.to_string())
                .or_default()
                .insert(fingerprint.to_string(), now);
        } else if let Some(marks) = self.data.viewed.get_mut(path) {
            marks.remove(fingerprint);
            if marks.is_empty() {
                self.data.viewed.remove(path);
            }
        }
    }

    pub fn snapshot(&self) -> Option<(PathBuf, Vec<u8>)> {
        let path = self.path.clone()?;
        let bytes = serde_json::to_vec_pretty(&self.data).ok()?;
        Some((path, bytes))
    }

    pub fn save(&self) -> io::Result<()> {
        match self.snapshot() {
            Some((path, bytes)) => write_atomic(&path, &bytes),
            None => Ok(()),
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

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
