use std::path::Path;
use std::process::Command;

use git_repo::Repo;
use review_core::{FileStatus, LineKind, Side};

use super::{build, gitlab_remote};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn numbered(n: usize, changed: Option<usize>) -> String {
    (1..=n)
        .map(|i| {
            if Some(i) == changed {
                format!("line {i} changed\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "src/a.rs", &numbered(40, None));
    write(root, "old.go", "package old\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["checkout", "-qb", "feature"]);
    write(root, "src/a.rs", &numbered(40, Some(20)));
    write(root, "web/new.ts", "export const x = 1;\n");
    std::fs::remove_file(root.join("old.go")).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "feature"]);
    git(root, &["checkout", "-q", "main"]);
    write(root, "main_only.txt", "target moved on\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "main moves"]);
    let repo = Repo::open(root).unwrap();
    (dir, repo)
}

#[test]
fn review_shows_only_what_the_source_brings() {
    let (dir, repo) = fixture();
    let comparison = repo.compare("main", "feature").unwrap();
    let review = build(&repo, comparison, false).unwrap();
    let mut paths: Vec<&str> = review.files.iter().map(|f| f.path.as_ref()).collect();
    paths.sort();
    assert_eq!(paths, vec!["old.go", "src/a.rs", "web/new.ts"]);
    assert!(review.comments_aligned);
    assert!(!review.context_limited);
    assert_eq!(review.commits.len(), 1);

    let status = |path: &str| review.files[review.file_index(path).unwrap()].status;
    assert_eq!(status("old.go"), FileStatus::Deleted);
    assert_eq!(status("web/new.ts"), FileStatus::Added);
    assert_eq!(status("src/a.rs"), FileStatus::Modified);

    let a = &review.files[review.file_index("src/a.rs").unwrap()];
    let old_blob = git(
        dir.path(),
        &["rev-parse", &format!("{}:src/a.rs", review.base_sha)],
    );
    let new_blob = git(dir.path(), &["rev-parse", "feature:src/a.rs"]);
    assert_eq!(
        a.fingerprint,
        review_core::fingerprint(&old_blob, &new_blob)
    );
    assert_eq!((a.additions, a.deletions), (1, 1));
    assert_eq!(
        a.line_map.locate(Side::New, 20),
        Some(LineKind::Added { new: 20 })
    );
    assert_eq!(
        a.line_map.locate(Side::Old, 20),
        Some(LineKind::Removed { old: 20 })
    );
    assert_eq!(
        a.line_map.locate(Side::New, 1),
        Some(LineKind::Context { old: 1, new: 1 })
    );
    assert_eq!(
        a.line_map.locate(Side::New, 40),
        Some(LineKind::Context { old: 40, new: 40 })
    );

    let order: Vec<&str> = review
        .tree
        .file_order()
        .iter()
        .map(|&ix| review.files[ix].path.as_ref())
        .collect();
    assert_eq!(order, vec!["src/a.rs", "web/new.ts", "old.go"]);
}

#[test]
fn fingerprint_changes_when_the_source_changes_the_file_again() {
    let (dir, repo) = fixture();
    let first = build(&repo, repo.compare("main", "feature").unwrap(), false).unwrap();
    git(dir.path(), &["checkout", "-q", "feature"]);
    write(dir.path(), "src/a.rs", &numbered(41, Some(20)));
    git(dir.path(), &["commit", "-qam", "more"]);
    let second = build(&repo, repo.compare("main", "feature").unwrap(), false).unwrap();
    let fp =
        |r: &super::LoadedReview, p: &str| r.files[r.file_index(p).unwrap()].fingerprint.clone();
    assert_ne!(fp(&first, "src/a.rs"), fp(&second, "src/a.rs"));
    assert_eq!(fp(&first, "web/new.ts"), fp(&second, "web/new.ts"));
}

#[test]
fn gitlab_remote_matches_the_configured_host() {
    let (dir, repo) = fixture();
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/acme/tool.git",
        ],
    );
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@gitlab.example.com:group/sub/tool.git",
        ],
    );
    assert_eq!(
        gitlab_remote(&repo, "https://gitlab.example.com/"),
        Some(("origin".to_string(), "group/sub/tool".to_string()))
    );
    assert_eq!(gitlab_remote(&repo, "https://gitlab.other.org"), None);
}
