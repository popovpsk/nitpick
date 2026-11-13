use std::collections::HashMap;

use crate::command::checked_arg;
use crate::repo::Repo;
use crate::{ChangeStatus, ChangedFile, DiffOptions, GitError};

const DIFF_CONFIG: [&str; 6] = [
    "-c",
    "diff.noprefix=false",
    "-c",
    "diff.mnemonicPrefix=false",
    "-c",
    "diff.relative=false",
];

const PATCH_FLAGS: [&str; 3] = ["--no-color", "--no-ext-diff", "--no-textconv"];

fn option_flags(opts: &DiffOptions) -> Vec<String> {
    let mut flags = vec![if opts.find_renames {
        "-M".to_string()
    } else {
        "--no-renames".to_string()
    }];
    if opts.ignore_whitespace {
        flags.push("-w".to_string());
    }
    flags
}

impl Repo {
    pub fn changed_files(
        &self,
        base: &str,
        head: &str,
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
        args.push(checked_arg(head)?.to_string());
        args.push("--".to_string());
        parse_raw_numstat(&self.git(&args)?)
    }

    pub fn diff(&self, base: &str, head: &str, opts: &DiffOptions) -> Result<String, GitError> {
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
        args.push(checked_arg(head)?.to_string());
        args.push("--".to_string());
        let out = self.git(&args)?;
        Ok(match String::from_utf8(out) {
            Ok(text) => text,
            Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
        })
    }
}

fn status_of(letter: char) -> ChangeStatus {
    match letter {
        'A' => ChangeStatus::Added,
        'D' => ChangeStatus::Deleted,
        'R' => ChangeStatus::Renamed,
        'C' => ChangeStatus::Copied,
        'T' => ChangeStatus::TypeChanged,
        _ => ChangeStatus::Modified,
    }
}

fn count(field: &str) -> Option<u32> {
    field.parse().ok()
}

pub(crate) fn parse_raw_numstat(bytes: &[u8]) -> Result<Vec<ChangedFile>, GitError> {
    let mut tokens = bytes
        .split(|byte| *byte == 0)
        .map(|token| String::from_utf8_lossy(token).into_owned());
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut counts: HashMap<String, (Option<u32>, Option<u32>)> = HashMap::new();
    let missing =
        |what: &str| GitError::Parse(format!("truncated git diff output: missing {what}"));
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            continue;
        }
        if let Some(meta) = token.strip_prefix(':') {
            let fields: Vec<&str> = meta.split(' ').collect();
            let [_, _, old_blob, new_blob, status] = fields[..] else {
                return Err(GitError::Parse(format!(
                    "unexpected raw diff line {token:?}"
                )));
            };
            let status = status_of(status.chars().next().unwrap_or('M'));
            let first = tokens
                .next()
                .filter(|path| !path.is_empty())
                .ok_or_else(|| missing("path"))?;
            let (path, old_path) = match status {
                ChangeStatus::Renamed | ChangeStatus::Copied => {
                    let second = tokens
                        .next()
                        .filter(|path| !path.is_empty())
                        .ok_or_else(|| missing("new path"))?;
                    (second, Some(first))
                }
                _ => (first, None),
            };
            files.push(ChangedFile {
                path,
                old_path,
                status,
                old_blob: old_blob.to_string(),
                new_blob: new_blob.to_string(),
                additions: None,
                deletions: None,
            });
        } else {
            let mut fields = token.splitn(3, '\t');
            let additions = count(fields.next().unwrap_or_default());
            let deletions = count(fields.next().unwrap_or_default());
            let path = match fields.next() {
                Some(path) if !path.is_empty() => path.to_string(),
                _ => {
                    tokens.next().ok_or_else(|| missing("old path"))?;
                    tokens.next().ok_or_else(|| missing("new path"))?
                }
            };
            counts.insert(path, (additions, deletions));
        }
    }
    for file in &mut files {
        if let Some((additions, deletions)) = counts.get(&file.path) {
            file.additions = *additions;
            file.deletions = *deletions;
        }
    }
    Ok(files)
}
