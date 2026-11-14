use std::fmt::Write as _;
use std::path::Path;

use crate::command::{checked_arg, text};
use crate::diff::{DIFF_CONFIG, PATCH_FLAGS, option_flags, parse_raw_numstat};
use crate::repo::Repo;
use crate::{ChangeStatus, ChangedFile, DiffOptions, GitError};

const HASH_BATCH: usize = 256;
const UNTRACKED_TEXT_LIMIT: u64 = 16 * 1024 * 1024;
const BINARY_SNIFF: usize = 8000;

struct Untracked {
    path: String,
    mode: &'static str,
    content: UntrackedContent,
}

enum UntrackedContent {
    Text(String),
    Binary,
}

impl Repo {
    pub fn head_or_empty_tree(&self) -> Result<String, GitError> {
        match self.resolve("HEAD") {
            Ok(sha) => Ok(sha),
            Err(_) => Ok(text(&self.git(&[
                "hash-object",
                "-t",
                "tree",
                "--stdin",
            ])?)),
        }
    }

    pub fn untracked_files(&self) -> Result<Vec<String>, GitError> {
        let out = self.git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        Ok(out
            .split(|byte| *byte == 0)
            .filter(|token| !token.is_empty() && !token.ends_with(b"/"))
            .map(|token| String::from_utf8_lossy(token).into_owned())
            .collect())
    }

    pub fn working_tree_changes(
        &self,
        base: &str,
        opts: &DiffOptions,
    ) -> Result<Vec<ChangedFile>, GitError> {
        let mut args: Vec<String> = DIFF_CONFIG.iter().map(|arg| arg.to_string()).collect();
        args.extend(
            ["diff", "--raw", "--numstat", "-z", "--no-abbrev"]
                .into_iter()
                .chain(PATCH_FLAGS)
                .map(str::to_string),
        );
        args.extend(option_flags(opts));
        args.push(checked_arg(base)?.to_string());
        args.push("--".to_string());
        let mut files = parse_raw_numstat(&self.git(&args)?)?;
        let null = null_blob(base);
        let unhashed: Vec<String> = files
            .iter()
            .filter(|f| f.status != ChangeStatus::Deleted && is_null(&f.new_blob))
            .map(|f| f.path.clone())
            .collect();
        let hashes = self.hash_paths(&unhashed)?;
        for file in &mut files {
            if let Some(hash) = hashes.get(&file.path) {
                file.new_blob = hash.clone();
            }
        }
        let untracked = self.read_untracked()?;
        let regular: Vec<String> = untracked
            .iter()
            .filter(|u| u.mode != "120000")
            .map(|u| u.path.clone())
            .collect();
        let hashes = self.hash_paths(&regular)?;
        for entry in untracked {
            let (additions, deletions) = match &entry.content {
                UntrackedContent::Text(content) => (Some(line_count(content) as u32), Some(0)),
                UntrackedContent::Binary => (None, None),
            };
            let new_blob = match (&entry.content, hashes.get(&entry.path)) {
                (_, Some(hash)) => hash.clone(),
                (UntrackedContent::Text(target), None) => format!("link:{target}"),
                (UntrackedContent::Binary, None) => null.clone(),
            };
            files.push(ChangedFile {
                path: entry.path,
                old_path: None,
                status: ChangeStatus::Added,
                old_blob: null.clone(),
                new_blob,
                additions,
                deletions,
            });
        }
        Ok(files)
    }

    pub fn working_tree_diff(&self, base: &str, opts: &DiffOptions) -> Result<String, GitError> {
        let mut args: Vec<String> = DIFF_CONFIG.iter().map(|arg| arg.to_string()).collect();
        args.push("diff".to_string());
        args.push(format!("-U{}", opts.context_lines));
        args.extend(
            PATCH_FLAGS
                .into_iter()
                .chain([
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                    "--full-index",
                    "--submodule=short",
                ])
                .map(str::to_string),
        );
        args.extend(option_flags(opts));
        args.push(checked_arg(base)?.to_string());
        args.push("--".to_string());
        let out = self.git(&args)?;
        let mut patch = match String::from_utf8(out) {
            Ok(text) => text,
            Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
        };
        let untracked = self.read_untracked()?;
        let regular: Vec<String> = untracked
            .iter()
            .filter(|u| u.mode != "120000")
            .map(|u| u.path.clone())
            .collect();
        let hashes = self.hash_paths(&regular)?;
        let null = null_blob(base);
        for entry in &untracked {
            let hash = hashes.get(&entry.path).unwrap_or(&null);
            append_new_file(&mut patch, entry, &null, hash);
        }
        Ok(patch)
    }

    fn read_untracked(&self) -> Result<Vec<Untracked>, GitError> {
        let mut entries = Vec::new();
        for path in self.untracked_files()? {
            let full = self.root().join(&path);
            let Ok(meta) = std::fs::symlink_metadata(&full) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&full)
                    .map(|t| t.to_string_lossy().into_owned())
                    .unwrap_or_default();
                entries.push(Untracked {
                    path,
                    mode: "120000",
                    content: UntrackedContent::Text(target),
                });
            } else if meta.is_file() {
                entries.push(Untracked {
                    mode: file_mode(&meta),
                    content: read_content(&full, meta.len()),
                    path,
                });
            }
        }
        Ok(entries)
    }

    fn hash_paths(
        &self,
        paths: &[String],
    ) -> Result<std::collections::HashMap<String, String>, GitError> {
        let mut hashes = std::collections::HashMap::with_capacity(paths.len());
        for chunk in paths.chunks(HASH_BATCH) {
            let mut args = vec!["hash-object".to_string(), "--".to_string()];
            args.extend(chunk.iter().cloned());
            let out = self.git(&args)?;
            let text = String::from_utf8_lossy(&out);
            for (path, hash) in chunk.iter().zip(text.lines()) {
                hashes.insert(path.clone(), hash.trim().to_string());
            }
        }
        Ok(hashes)
    }
}

fn null_blob(base: &str) -> String {
    "0".repeat(if base.len() == 64 { 64 } else { 40 })
}

fn is_null(blob: &str) -> bool {
    blob.bytes().all(|b| b == b'0')
}

#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    }
}

#[cfg(not(unix))]
fn file_mode(_: &std::fs::Metadata) -> &'static str {
    "100644"
}

fn read_content(path: &Path, len: u64) -> UntrackedContent {
    if len > UNTRACKED_TEXT_LIMIT {
        return UntrackedContent::Binary;
    }
    match std::fs::read(path) {
        Ok(bytes) if !bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) => {
            UntrackedContent::Text(match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
            })
        }
        _ => UntrackedContent::Binary,
    }
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.matches('\n').count() + usize::from(!content.ends_with('\n'))
    }
}

fn quote_path(prefix: &str, path: &str) -> String {
    let needs = path
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control());
    if !needs {
        return format!("{prefix}{path}");
    }
    let mut out = String::with_capacity(path.len() + prefix.len() + 2);
    out.push('"');
    out.push_str(prefix);
    for c in path.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn append_new_file(patch: &mut String, entry: &Untracked, null: &str, hash: &str) {
    if !patch.is_empty() && !patch.ends_with('\n') {
        patch.push('\n');
    }
    let a = quote_path("a/", &entry.path);
    let b = quote_path("b/", &entry.path);
    let _ = writeln!(patch, "diff --git {a} {b}");
    let _ = writeln!(patch, "new file mode {}", entry.mode);
    let _ = writeln!(patch, "index {null}..{hash}");
    match &entry.content {
        UntrackedContent::Binary => {
            let _ = writeln!(patch, "Binary files /dev/null and {b} differ");
        }
        UntrackedContent::Text(content) if content.is_empty() => {}
        UntrackedContent::Text(content) => {
            let lines = line_count(content);
            let _ = writeln!(patch, "--- /dev/null");
            let _ = writeln!(patch, "+++ {b}");
            if lines == 1 {
                let _ = writeln!(patch, "@@ -0,0 +1 @@");
            } else {
                let _ = writeln!(patch, "@@ -0,0 +1,{lines} @@");
            }
            for line in content.split_inclusive('\n') {
                patch.push('+');
                patch.push_str(line);
            }
            if !content.ends_with('\n') {
                patch.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
}
