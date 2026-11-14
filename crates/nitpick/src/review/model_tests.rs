use std::path::Path;
use std::process::Command;

use forge::ForgeKind;
use git_repo::Repo;
use gpui_component::Rope;
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
use review_core::{Change, FileStatus, LineKind, Side};
use rstest::rstest;

use super::{
    ForgeRemote, MrRefs, build, build_working_tree, forge_remote, interdiff, resolve_mr_refs,
};

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

fn forges(list: &[(ForgeKind, &str)]) -> Vec<(ForgeKind, String)> {
    list.iter()
        .map(|(kind, url)| (*kind, url.to_string()))
        .collect()
}

fn found(kind: ForgeKind, remote: &str, project: &str) -> Option<ForgeRemote> {
    Some(ForgeRemote {
        kind,
        remote: remote.into(),
        project: project.into(),
    })
}

#[test]
fn forge_remote_matches_the_configured_host() {
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
    let gitlab = forges(&[(ForgeKind::GitLab, "https://gitlab.example.com/")]);
    assert_eq!(
        forge_remote(&repo, &gitlab, None),
        found(ForgeKind::GitLab, "origin", "group/sub/tool")
    );
    let other = forges(&[(ForgeKind::GitLab, "https://gitlab.other.org")]);
    assert_eq!(forge_remote(&repo, &other, None), None);
    let github = forges(&[(ForgeKind::GitHub, "https://github.com")]);
    assert_eq!(
        forge_remote(&repo, &github, None),
        found(ForgeKind::GitHub, "upstream", "acme/tool")
    );
    let both = forges(&[
        (ForgeKind::GitLab, "https://gitlab.example.com"),
        (ForgeKind::GitHub, "https://github.com"),
    ]);
    assert_eq!(
        forge_remote(&repo, &both, None),
        found(ForgeKind::GitLab, "origin", "group/sub/tool")
    );
    assert_eq!(
        forge_remote(&repo, &both, Some("acme/tool")),
        found(ForgeKind::GitHub, "upstream", "acme/tool")
    );
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "fork",
            "git@gitlab.example.com:me/tool.git",
        ],
    );
    let gitlab = forges(&[(ForgeKind::GitLab, "https://gitlab.example.com")]);
    assert_eq!(
        forge_remote(&repo, &gitlab, Some("me/tool")),
        found(ForgeKind::GitLab, "fork", "me/tool")
    );
}

#[test]
fn a_fork_lists_the_requests_of_its_upstream() {
    let (dir, repo) = fixture();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:me/tool.git"],
    );
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/acme/tool.git",
        ],
    );
    let github = forges(&[(ForgeKind::GitHub, "https://github.com")]);
    assert_eq!(
        forge_remote(&repo, &github, None),
        found(ForgeKind::GitHub, "upstream", "acme/tool")
    );
    assert_eq!(
        forge_remote(&repo, &github, Some("me/tool")),
        found(ForgeKind::GitHub, "origin", "me/tool")
    );
}

#[test]
fn a_request_without_a_merge_base_gets_it_from_git() {
    let (_dir, repo) = fixture();
    let head = repo.resolve("feature").unwrap();
    let tip = repo.resolve("main").unwrap();
    let base = repo.compare("main", "feature").unwrap().base_sha;
    assert_ne!(base, tip);
    let github = forge::DiffRefs {
        base_sha: None,
        start_sha: tip.clone(),
        head_sha: head.clone(),
    };
    assert_eq!(
        resolve_mr_refs(&repo, &github).unwrap(),
        MrRefs {
            base_sha: base,
            start_sha: tip.clone(),
            head_sha: head.clone(),
        }
    );
    let gitlab = forge::DiffRefs {
        base_sha: Some("reported".into()),
        start_sha: tip,
        head_sha: head,
    };
    assert_eq!(
        resolve_mr_refs(&repo, &gitlab).unwrap().base_sha,
        "reported"
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

#[test]
fn interdiff_shows_what_changed_since_the_viewed_version() {
    let (dir, repo) = fixture();
    let first = build(&repo, repo.compare("main", "feature").unwrap(), None, false).unwrap();
    let viewed = first.files[first.file_index("src/a.rs").unwrap()]
        .new_blob
        .clone();
    git(dir.path(), &["checkout", "-q", "feature"]);
    let mut text = numbered(40, Some(20));
    text = text.replace("line 35\n", "line 35 again\n");
    write(dir.path(), "src/a.rs", &text);
    git(dir.path(), &["commit", "-qam", "more"]);
    let second = build(&repo, repo.compare("main", "feature").unwrap(), None, false).unwrap();
    let file = second.file_index("src/a.rs").unwrap();
    let current = second.files[file].new_blob.clone();
    let diff = interdiff(
        &repo,
        file,
        "k".into(),
        "src/a.rs",
        &viewed,
        &current,
        false,
    )
    .unwrap();
    assert_eq!(diff.file, file);
    assert_eq!(diff.previous, viewed);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(
        diff.map.locate(Side::New, 35),
        Some(LineKind::Added { new: 35 })
    );
    assert_eq!(
        diff.map.locate(Side::New, 20),
        Some(LineKind::Context { old: 20, new: 20 })
    );
    assert_eq!(
        diff.diff.as_ref().map(|d| d.path().to_string()),
        Some("src/a.rs".to_string())
    );
    let same = interdiff(
        &repo,
        file,
        "k".into(),
        "src/a.rs",
        &current,
        &current,
        false,
    )
    .unwrap();
    assert!(same.is_empty());
    assert!(same.changes.is_empty());
}
