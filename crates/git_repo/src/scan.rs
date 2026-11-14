use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::GitError;

pub const SCAN_DEPTH: usize = 5;

const SKIPPED_DIRS: [&str; 5] = ["node_modules", "target", "vendor", "build", "dist"];

enum DirKind {
    Repository,
    Linked,
    Bare,
    Plain,
}

pub fn find_repositories(root: &Path) -> Result<Vec<PathBuf>, GitError> {
    let root = canonical(root)?;
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if !visited.insert(dir.clone()) {
            continue;
        }
        match dir_kind(&dir) {
            DirKind::Repository => {
                found.push(dir);
                continue;
            }
            DirKind::Linked | DirKind::Bare => continue,
            DirKind::Plain => {}
        }
        if depth >= SCAN_DEPTH {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                log::debug!("scan {}: {err}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = if file_type.is_dir() {
                entry.path()
            } else if file_type.is_symlink() && entry.path().is_dir() {
                match canonical(&entry.path()) {
                    Ok(target) if !dir.starts_with(&target) => target,
                    _ => continue,
                }
            } else {
                continue;
            };
            if !visited.contains(&path) {
                stack.push((path, depth + 1));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn dir_kind(dir: &Path) -> DirKind {
    match fs::metadata(dir.join(".git")) {
        Ok(meta) if meta.is_dir() => DirKind::Repository,
        Ok(_) => DirKind::Linked,
        Err(_) if is_bare(dir) => DirKind::Bare,
        Err(_) => DirKind::Plain,
    }
}

fn is_bare(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

fn canonical(path: &Path) -> Result<PathBuf, GitError> {
    Ok(strip_verbatim(fs::canonicalize(path)?))
}

pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
