use std::path::Path;
use std::process::Command;

use git_repo::Repo;
use gpui_component::Rope;
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
use review_core::{Change, FileStatus, LineKind, Side};
use rstest::rstest;

use super::{build, build_working_tree, gitlab_remote};

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
    let review = build(&repo, comparison, None, false).unwrap();
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
fn one_commit_shows_only_that_commit_and_keeps_the_range() {
    let (dir, repo) = fixture();
    let root = dir.path();
    git(root, &["checkout", "-q", "feature"]);
    let first = git(root, &["rev-parse", "HEAD"]);
    write(root, "src/a.rs", &numbered(41, Some(20)));
    git(root, &["commit", "-qam", "second"]);
    let second = git(root, &["rev-parse", "HEAD"]);
    write(root, "docs/b.md", "b\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "third"]);
    let range = repo.compare("main", "feature").unwrap();
    let merge_base = range.base_sha.clone();

    let review = build(&repo, range.clone(), Some(&second), false).unwrap();
    let paths: Vec<&str> = review.files.iter().map(|f| f.path.as_ref()).collect();
    assert_eq!(paths, vec!["src/a.rs"]);
    assert_eq!(review.base_sha, first);
    assert_eq!(review.head_sha, second);
    assert_eq!(review.merge_base_sha, merge_base);
    assert_eq!(review.commit_sha(), Some(second.as_str()));
    assert_eq!(review.commits.len(), 3);
    assert_eq!(review.commits[1].sha, second);
    assert_eq!(review.target, "main");
    assert_eq!(review.source, "feature");
    let a = &review.files[0];
    assert_eq!(a.status, FileStatus::Modified);
    assert_eq!((a.additions, a.deletions), (1, 0));
    assert_eq!(
        a.line_map.locate(Side::New, 41),
        Some(LineKind::Added { new: 41 })
    );

    let all = build(&repo, range.clone(), None, false).unwrap();
    assert_eq!(all.files.len(), 4);
    assert_eq!(all.commit, None);
    assert_eq!(all.base_sha, merge_base);
    assert_eq!(all.merge_base_sha, merge_base);
    assert_ne!(
        all.files[all.file_index("src/a.rs").unwrap()].fingerprint,
        a.fingerprint
    );

    let gone = build(&repo, range, Some("0123456789abcdef"), false).unwrap();
    assert_eq!(gone.commit, None);
    assert_eq!(gone.files.len(), 4);
}

#[test]
fn fingerprint_changes_when_the_source_changes_the_file_again() {
    let (dir, repo) = fixture();
    let first = build(&repo, repo.compare("main", "feature").unwrap(), None, false).unwrap();
    git(dir.path(), &["checkout", "-q", "feature"]);
    write(dir.path(), "src/a.rs", &numbered(41, Some(20)));
    git(dir.path(), &["commit", "-qam", "more"]);
    let second = build(&repo, repo.compare("main", "feature").unwrap(), None, false).unwrap();
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
        gitlab_remote(&repo, "https://gitlab.example.com/", None),
        Some(("origin".to_string(), "group/sub/tool".to_string()))
    );
    assert_eq!(gitlab_remote(&repo, "https://gitlab.other.org", None), None);
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "fork",
            "git@gitlab.example.com:me/tool.git",
        ],
    );
    assert_eq!(
        gitlab_remote(&repo, "https://gitlab.example.com", Some("me/tool")),
        Some(("fork".to_string(), "me/tool".to_string()))
    );
}

#[test]
fn ignoring_whitespace_keeps_exact_comment_positions() {
    let (dir, repo) = fixture();
    git(dir.path(), &["checkout", "-q", "feature"]);
    let mut text = numbered(40, Some(20));
    text = text.replace("line 5\n", "line   5\n");
    write(dir.path(), "src/a.rs", &text);
    git(dir.path(), &["commit", "-qam", "indent"]);
    let review = build(&repo, repo.compare("main", "feature").unwrap(), None, true).unwrap();
    let a = &review.files[review.file_index("src/a.rs").unwrap()];
    assert_eq!((a.additions, a.deletions), (1, 1));
    assert_eq!(
        a.line_map.locate(Side::New, 5),
        Some(LineKind::Added { new: 5 })
    );
    assert_eq!(
        a.line_map.locate(Side::Old, 5),
        Some(LineKind::Removed { old: 5 })
    );
    assert!(review.comments_aligned);
    assert_eq!(
        a.changes,
        vec![Change {
            first: LineKind::Removed { old: 20 },
            anchor: LineKind::Context { old: 18, new: 18 },
        }]
    );
}

#[test]
fn working_tree_review_shows_uncommitted_and_untracked_files() {
    let (dir, repo) = fixture();
    let root = dir.path();
    write(root, "main_only.txt", "edited but not committed\n");
    write(root, "notes/todo.md", "# todo\n- review\n");
    let review = build_working_tree(&repo, false).unwrap();
    let mut seen: Vec<(&str, FileStatus)> = review
        .files
        .iter()
        .map(|f| (f.path.as_ref(), f.status))
        .collect();
    seen.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(
        seen,
        vec![
            ("main_only.txt", FileStatus::Modified),
            ("notes/todo.md", FileStatus::Added),
        ]
    );
    assert_eq!(review.target, "main");
    assert!(review.commits.is_empty());
    let todo = &review.files[review.file_index("notes/todo.md").unwrap()];
    assert_eq!(todo.additions, 2);
    let first = todo.fingerprint.clone();
    write(root, "notes/todo.md", "# todo\n- review\n- ship\n");
    let again = build_working_tree(&repo, false).unwrap();
    let todo = &again.files[again.file_index("notes/todo.md").unwrap()];
    assert_ne!(todo.fingerprint, first);
}

#[test]
fn language_overrides_apply_to_parsed_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "README", "base\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["checkout", "-qb", "feature"]);
    write(root, "build/rules.mk", "all:\n\techo hi\n");
    write(root, "src/lib.rs", "fn main() {}\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "feature"]);
    let repo = Repo::open(root).unwrap();
    let review = build(&repo, repo.compare("main", "feature").unwrap(), None, false).unwrap();
    let language = |path: &str| {
        let ix = review.file_index(path).unwrap();
        review.files[ix].diff.language().to_string()
    };
    assert_eq!(language("build/rules.mk"), "make");
    assert_eq!(language("src/lib.rs"), "rust");
}

#[rstest]
#[case::proto(
    "api/service.proto",
    "syntax = \"proto3\";\nmessage A { string name = 1; }\n"
)]
#[case::makefile("Makefile", "CFLAGS := -O2\nall: build\n\techo $(CFLAGS)\n")]
#[case::make_include("build/rules.mk", "CFLAGS := -O2\nall: build\n\techo $(CFLAGS)\n")]
#[case::csharp("src/App.cs", "public class App { int x = 1; }\n")]
#[case::java("src/Main.java", "public class Main { int x = 1; }\n")]
#[case::c("src/main.c", "int main(void) { return 0; }\n")]
#[case::cpp("src/main.cpp", "int main() { return 0; }\n")]
#[case::kotlin("app/Main.kt", "fun main() { val x = 1 }\n")]
#[case::swift("Sources/App.swift", "let x = 1\nfunc f() {}\n")]
#[case::scala("src/A.scala", "object A { val x = 1 }\n")]
#[case::ruby("lib/a.rb", "def f\n  1\nend\n")]
#[case::php("web/a.php", "<?php function f() { return 1; }\n")]
#[case::lua("scripts/a.lua", "local x = 1\n")]
#[case::elixir("lib/a.ex", "defmodule A do\n  def f, do: 1\nend\n")]
#[case::zig("src/main.zig", "const x: u8 = 1;\n")]
#[case::graphql("schema.graphql", "type Query { a: Int }\n")]
#[case::cmake("CMakeLists.txt", "project(a)\nadd_executable(a main.c)\n")]
#[case::json("package.json", "{\"a\": 1}\n")]
#[case::sql("db/schema.sql", "SELECT id FROM users WHERE id = 1;\n")]
#[case::diff("fixes/0001.patch", "@@ -1 +1 @@\n-a\n+b\n")]
#[case::zsh("scripts/setup.zsh", "export A=1\necho \"$A\"\n")]
fn review_languages_highlight(#[case] path: &str, #[case] source: &str) {
    let body: String = source.lines().map(|line| format!("+{line}\n")).collect();
    let patch = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n{body}",
        source.lines().count()
    );
    let files = super::parse(&patch).unwrap();
    let language = files[0].language();
    let mut highlighter = SyntaxHighlighter::new(&language);
    highlighter.update(None, &Rope::from(source), None);
    let styles = highlighter.styles(&(0..source.len()), &*HighlightTheme::default_dark());
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "{path} ({language}) has no syntax colors"
    );
}
