use std::cell::Cell;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rstest::rstest;
use tempfile::TempDir;

use crate::diff::parse_raw_numstat;
use crate::{
    Branch, BranchKind, ChangeStatus, ChangedFile, DiffOptions, GitError, PullOutcome, Remote,
    RemoteLocation, Repo, parse_remote_url,
};

type Change<'a> = (&'a str, Option<(&'a str, &'a [u8])>);

struct Fixture {
    dir: TempDir,
    config: PathBuf,
    clock: Cell<i64>,
    indexes: Cell<u32>,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("gitconfig");
        std::fs::write(&config, "").unwrap();
        Fixture {
            dir,
            config,
            clock: Cell::new(1_700_000_000),
            indexes: Cell::new(0),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        env: &[(&str, &OsStr)],
        input: Option<&[u8]>,
    ) -> String {
        let time = self.clock.get();
        self.clock.set(time + 60);
        let date = format!("{time} +0000");
        let mut command = Command::new("git");
        command
            .current_dir(cwd)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "core.autocrlf=false",
                "-c",
                "protocol.file.allow=always",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Ann Author")
            .env("GIT_AUTHOR_EMAIL", "ann@example.com")
            .env("GIT_COMMITTER_NAME", "Carl Committer")
            .env("GIT_COMMITTER_EMAIL", "carl@example.com")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_string()
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        self.run(cwd, args, &[], None)
    }

    fn init(&self, name: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q"]);
        path
    }

    fn write(&self, repo: &Path, path: &str, content: &str) {
        let path = repo.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn commit(&self, repo: &Path, files: &[(&str, &str)], message: &str) -> String {
        for (path, content) in files {
            self.write(repo, path, content);
        }
        self.git(repo, &["add", "-A"]);
        self.git(repo, &["commit", "-q", "--allow-empty", "-m", message]);
        self.rev(repo, "HEAD")
    }

    fn rev(&self, repo: &Path, rev: &str) -> String {
        self.git(repo, &["rev-parse", rev])
    }

    fn plumbing_commit(&self, repo: &Path, parent: Option<&str>, changes: &[Change]) -> String {
        let number = self.indexes.get();
        self.indexes.set(number + 1);
        let index = self.path(&format!("index-{number}"));
        let env = [("GIT_INDEX_FILE", index.as_os_str())];
        if let Some(parent) = parent {
            self.run(repo, &["read-tree", parent], &env, None);
        }
        for (path, change) in changes {
            match change {
                Some((mode, content)) => {
                    let blob = self.run(
                        repo,
                        &["hash-object", "-w", "--stdin"],
                        &env,
                        Some(*content),
                    );
                    let info = format!("{mode},{blob},{path}");
                    self.run(
                        repo,
                        &["update-index", "--add", "--cacheinfo", &info],
                        &env,
                        None,
                    );
                }
                None => {
                    self.run(repo, &["update-index", "--force-remove", *path], &env, None);
                }
            }
        }
        let tree = self.run(repo, &["write-tree"], &env, None);
        let mut args = vec!["commit-tree", tree.as_str(), "-m", "plumbing"];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        self.run(repo, &args, &env, None)
    }

    fn with_origin(&self) -> (Repo, PathBuf, PathBuf) {
        let origin = self.path("origin.git");
        self.git(
            self.dir.path(),
            &["init", "-q", "--bare", origin.to_str().unwrap()],
        );
        let work = self.init("work");
        self.commit(&work, &[("README.md", "hello\n")], "initial");
        self.git(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        self.git(&work, &["push", "-q", "-u", "origin", "main"]);
        self.git(
            self.dir.path(),
            &["clone", "-q", origin.to_str().unwrap(), "other"],
        );
        (Repo::open(&work).unwrap(), work, self.path("other"))
    }
}

fn paths(files: &[ChangedFile]) -> Vec<&str> {
    let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    paths.sort();
    paths
}

fn changed_paths(repo: &Repo, base: &str, head: &str) -> Vec<String> {
    let files = repo
        .changed_files(base, head, &DiffOptions::default())
        .unwrap();
    paths(&files).into_iter().map(str::to_string).collect()
}

#[test]
fn open_outside_a_repository_is_an_error() {
    let fixture = Fixture::new();
    let plain = fixture.path("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        Repo::open(&plain),
        Err(GitError::NotARepository(path)) if path == plain
    ));
}

#[test]
fn open_in_a_subdirectory_finds_the_toplevel() {
    let fixture = Fixture::new();
    let repo_path = fixture.init("my-project");
    fixture.commit(&repo_path, &[("src/deep/file.txt", "x\n")], "initial");
    let repo = Repo::open(&repo_path.join("src/deep")).unwrap();
    assert_eq!(
        repo.root().canonicalize().unwrap(),
        repo_path.canonicalize().unwrap()
    );
    assert_eq!(repo.name(), "my-project");
}

#[test]
fn compare_shows_only_what_the_source_introduces() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let initial = fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    fixture.git(&path, &["checkout", "-q", "-b", "feature"]);
    let feature = fixture.commit(&path, &[("f.txt", "feature\n")], "feature work");
    fixture.git(&path, &["checkout", "-q", "main"]);
    fixture.commit(&path, &[("m.txt", "main\n")], "main work");
    let repo = Repo::open(&path).unwrap();

    let comparison = repo.compare("main", "feature").unwrap();
    assert_eq!(comparison.base_sha, initial);
    assert_eq!(comparison.head_sha, feature);
    assert_eq!(comparison.target_sha, fixture.rev(&path, "main"));
    assert_eq!(comparison.merge_base_refs, vec!["main".to_string()]);
    assert_eq!(
        changed_paths(&repo, &comparison.base_sha, &comparison.head_sha),
        vec!["f.txt"]
    );
    let patch = repo
        .diff(
            &comparison.base_sha,
            &comparison.head_sha,
            &DiffOptions::default(),
        )
        .unwrap();
    assert!(patch.contains("b/f.txt"));
    assert!(!patch.contains("m.txt"));
    let commits = repo
        .commits(&comparison.base_sha, &comparison.head_sha)
        .unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].sha, feature);
    assert_eq!(commits[0].subject, "feature work");
    assert_eq!(commits[0].author, "Ann Author");
}

#[test]
fn compare_ignores_upstream_commits_missing_from_a_stale_local_target() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let initial = fixture.rev(&work, "main");
    fixture.commit(
        &other,
        &[("upstream.txt", "new upstream\n")],
        "upstream work",
    );
    fixture.git(&other, &["push", "-q", "origin", "main"]);
    fixture.git(&work, &["fetch", "-q", "origin"]);
    let upstream = fixture.rev(&work, "origin/main");

    fixture.git(&work, &["checkout", "-q", "-b", "feature", "main"]);
    fixture.commit(&work, &[("feature.txt", "feature\n")], "feature work");
    fixture.git(&work, &["merge", "-q", "--no-edit", "origin/main"]);
    fixture.commit(&work, &[("feature2.txt", "more\n")], "more feature work");
    assert_eq!(
        fixture.git(&work, &["merge-base", "main", "feature"]),
        initial
    );

    let comparison = repo.compare("main", "feature").unwrap();
    assert_eq!(comparison.base_sha, upstream);
    assert_eq!(
        comparison.merge_base_refs,
        vec!["main".to_string(), "origin/main".to_string()]
    );
    assert_eq!(
        changed_paths(&repo, &comparison.base_sha, &comparison.head_sha),
        vec!["feature.txt", "feature2.txt"]
    );

    let comparison = repo.compare("origin/main", "feature").unwrap();
    assert_eq!(
        comparison.merge_base_refs,
        vec!["origin/main".to_string(), "main".to_string()]
    );
    assert_eq!(
        changed_paths(&repo, &comparison.base_sha, &comparison.head_sha),
        vec!["feature.txt", "feature2.txt"]
    );
}

#[test]
fn compare_against_the_own_upstream_shows_the_upstream_changes() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    fixture.commit(
        &other,
        &[("upstream.txt", "new upstream\n")],
        "upstream work",
    );
    fixture.git(&other, &["push", "-q", "origin", "main"]);
    fixture.git(&work, &["fetch", "-q", "origin"]);

    let comparison = repo.compare("main", "origin/main").unwrap();
    assert_eq!(comparison.merge_base_refs, vec!["main".to_string()]);
    assert_eq!(
        changed_paths(&repo, &comparison.base_sha, &comparison.head_sha),
        vec!["upstream.txt"]
    );
}

#[test]
fn compare_after_merging_the_target_midway_shows_only_the_source_changes() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    fixture.git(&path, &["checkout", "-q", "-b", "feature"]);
    fixture.commit(&path, &[("f1.txt", "one\n")], "feature one");
    fixture.git(&path, &["checkout", "-q", "main"]);
    fixture.commit(&path, &[("m1.txt", "one\n")], "main one");
    fixture.git(&path, &["checkout", "-q", "feature"]);
    fixture.git(&path, &["merge", "-q", "--no-edit", "main"]);
    fixture.commit(
        &path,
        &[("f2.txt", "two\n"), ("a.txt", "a\nb\n")],
        "feature two",
    );
    fixture.git(&path, &["checkout", "-q", "main"]);
    fixture.commit(&path, &[("m2.txt", "two\n")], "main two");
    let repo = Repo::open(&path).unwrap();

    let comparison = repo.compare("main", "feature").unwrap();
    assert_eq!(
        changed_paths(&repo, &comparison.base_sha, &comparison.head_sha),
        vec!["a.txt", "f1.txt", "f2.txt"]
    );
}

#[test]
fn compare_of_unrelated_histories_has_no_merge_base() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    fixture.git(&path, &["checkout", "-q", "--orphan", "lonely"]);
    fixture.commit(&path, &[("b.txt", "b\n")], "unrelated");
    let repo = Repo::open(&path).unwrap();
    assert!(matches!(
        repo.compare("main", "lonely"),
        Err(GitError::NoMergeBase { .. })
    ));
}

#[test]
fn changed_files_reports_every_kind_of_change() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let renamed_body: String = (1..=20).map(|line| format!("line {line}\n")).collect();
    let renamed_after = format!("{renamed_body}line 21\n");
    let base = fixture.plumbing_commit(
        &path,
        None,
        &[
            ("old name.txt", Some(("100644", renamed_body.as_bytes()))),
            ("gone.txt", Some(("100644", b"1\n2\n3\n"))),
            ("mod.txt", Some(("100644", b"a\nb\nc\n"))),
            ("image.bin", Some(("100644", b"\x00\x01\x02binary"))),
            ("link", Some(("100644", b"target\n"))),
        ],
    );
    let mut changes: Vec<Change> = vec![
        ("old name.txt", None),
        (
            "new dir/new name.txt",
            Some(("100644", renamed_after.as_bytes())),
        ),
        ("gone.txt", None),
        ("mod.txt", Some(("100644", b"a\nB\nc\nd\n"))),
        ("image.bin", Some(("100644", b"\x00\x01\x03binary"))),
        ("link", Some(("120000", b"target"))),
        (
            "dir with space/\u{fc}n\u{ef}c\u{f8}d\u{e9} f\u{ef}le.txt",
            Some(("100644", b"x\ny\n")),
        ),
    ];
    if cfg!(unix) {
        changes.push(("tab\tand\nnewline.txt", Some(("100644", b"z\n"))));
    }
    let head = fixture.plumbing_commit(&path, Some(&base), &changes);
    let repo = Repo::open(&path).unwrap();

    let files = repo
        .changed_files(&base, &head, &DiffOptions::default())
        .unwrap();
    let find = |path: &str| {
        files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} missing from {files:#?}"))
    };
    let zeros = "0".repeat(40);

    let renamed = find("new dir/new name.txt");
    assert_eq!(renamed.status, ChangeStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
    assert_eq!((renamed.additions, renamed.deletions), (Some(1), Some(0)));

    let gone = find("gone.txt");
    assert_eq!(gone.status, ChangeStatus::Deleted);
    assert_eq!(gone.new_blob, zeros);
    assert_eq!(gone.old_path, None);
    assert_eq!((gone.additions, gone.deletions), (Some(0), Some(3)));

    let modified = find("mod.txt");
    assert_eq!(modified.status, ChangeStatus::Modified);
    assert_eq!((modified.additions, modified.deletions), (Some(2), Some(1)));

    let binary = find("image.bin");
    assert_eq!(binary.status, ChangeStatus::Modified);
    assert_eq!((binary.additions, binary.deletions), (None, None));

    assert_eq!(find("link").status, ChangeStatus::TypeChanged);

    let unicode = find("dir with space/\u{fc}n\u{ef}c\u{f8}d\u{e9} f\u{ef}le.txt");
    assert_eq!(unicode.status, ChangeStatus::Added);
    assert_eq!(unicode.old_blob, zeros);
    assert_eq!((unicode.additions, unicode.deletions), (Some(2), Some(0)));

    if cfg!(unix) {
        assert_eq!(find("tab\tand\nnewline.txt").status, ChangeStatus::Added);
    }
    assert_eq!(files.len(), changes.len() - 1);

    for file in &files {
        if file.status != ChangeStatus::Deleted {
            let spec = format!("{head}:{}", file.path);
            assert_eq!(file.new_blob, fixture.rev(&path, &spec), "{}", file.path);
        }
        if file.status != ChangeStatus::Added {
            let old = file.old_path.as_deref().unwrap_or(&file.path);
            let spec = format!("{base}:{old}");
            assert_eq!(file.old_blob, fixture.rev(&path, &spec), "{}", file.path);
        }
    }
}

#[test]
fn changed_files_without_rename_detection_reports_delete_and_add() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let body: String = (1..=20).map(|line| format!("line {line}\n")).collect();
    let base = fixture.commit(&path, &[("old.txt", &body)], "initial");
    fixture.git(&path, &["mv", "old.txt", "new.txt"]);
    let head = fixture.commit(&path, &[], "rename");
    let repo = Repo::open(&path).unwrap();
    let options = DiffOptions {
        find_renames: false,
        ..DiffOptions::default()
    };
    let files = repo.changed_files(&base, &head, &options).unwrap();
    let statuses: Vec<(&str, ChangeStatus)> = files
        .iter()
        .map(|file| (file.path.as_str(), file.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("new.txt", ChangeStatus::Added),
            ("old.txt", ChangeStatus::Deleted)
        ]
    );
    let renamed = repo
        .changed_files(&base, &head, &DiffOptions::default())
        .unwrap();
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].status, ChangeStatus::Renamed);
    assert_eq!(
        (renamed[0].additions, renamed[0].deletions),
        (Some(0), Some(0))
    );
}

#[test]
fn changed_files_can_ignore_whitespace() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let base = fixture.commit(
        &path,
        &[("spaces.txt", "a b\n"), ("real.txt", "one\n")],
        "initial",
    );
    let head = fixture.commit(
        &path,
        &[("spaces.txt", "a   b\n"), ("real.txt", "two\n")],
        "change",
    );
    let repo = Repo::open(&path).unwrap();
    assert_eq!(
        changed_paths(&repo, &base, &head),
        vec!["real.txt", "spaces.txt"]
    );
    let options = DiffOptions {
        ignore_whitespace: true,
        ..DiffOptions::default()
    };
    let files = repo.changed_files(&base, &head, &options).unwrap();
    let spaces = files.iter().find(|file| file.path == "spaces.txt");
    assert!(spaces.is_none_or(|file| file.additions == Some(0)));
    assert!(files.iter().any(|file| file.path == "real.txt"));
}

#[test]
fn raw_numstat_parsing_maps_copies_and_unknown_letters() {
    let old = "1".repeat(40);
    let new = "2".repeat(40);
    let raw = format!(
        ":100644 100644 {old} {new} C075\0src.txt\0copy.txt\0:100644 100644 {old} {new} U\0odd.txt\0\
         3\t1\t\0src.txt\0copy.txt\0-\t-\todd.txt\0"
    );
    let files = parse_raw_numstat(raw.as_bytes()).unwrap();
    assert_eq!(
        files,
        vec![
            ChangedFile {
                path: "copy.txt".to_string(),
                old_path: Some("src.txt".to_string()),
                status: ChangeStatus::Copied,
                old_blob: old.clone(),
                new_blob: new.clone(),
                additions: Some(3),
                deletions: Some(1),
            },
            ChangedFile {
                path: "odd.txt".to_string(),
                old_path: None,
                status: ChangeStatus::Modified,
                old_blob: old,
                new_blob: new,
                additions: None,
                deletions: None,
            },
        ]
    );
    assert!(parse_raw_numstat(b":100644 100644 abc def M\0").is_err());
}

#[test]
fn diff_with_huge_context_returns_the_whole_file() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let before: String = (1..=200).map(|line| format!("line {line}\n")).collect();
    let after = before.replace("line 100\n", "line one hundred\n");
    let base = fixture.commit(&path, &[("long.txt", &before)], "initial");
    let head = fixture.commit(&path, &[("long.txt", &after)], "change");
    fixture.git(&path, &["config", "diff.noprefix", "true"]);
    fixture.git(&path, &["config", "diff.mnemonicPrefix", "true"]);
    fixture.git(&path, &["config", "color.diff", "always"]);
    let repo = Repo::open(&path).unwrap();

    let full = DiffOptions {
        context_lines: 1_000_000,
        ..DiffOptions::default()
    };
    let patch = repo.diff(&base, &head, &full).unwrap();
    assert!(patch.starts_with("diff --git a/long.txt b/long.txt\n"));
    assert!(patch.contains(&format!(
        "index {}..",
        fixture.rev(&path, &format!("{base}:long.txt"))
    )));
    assert!(patch.contains("@@ -1,200 +1,200 @@"));
    assert!(patch.contains("\n line 1\n"));
    assert!(patch.contains("\n line 200\n"));
    assert!(patch.contains("\n-line 100\n+line one hundred\n"));
    assert!(!patch.contains('\u{1b}'));

    let short = repo.diff(&base, &head, &DiffOptions::default()).unwrap();
    assert!(short.contains("@@ -97,7 +97,7 @@"));
    assert!(!short.contains("\n line 1\n"));
}

#[test]
fn branches_are_sorted_local_first_by_recency() {
    let fixture = Fixture::new();
    let (_, work, _) = fixture.with_origin();
    fixture.git(&work, &["checkout", "-q", "-b", "older"]);
    fixture.commit(&work, &[("older.txt", "x\n")], "older work");
    fixture.git(&work, &["checkout", "-q", "-b", "newer", "main"]);
    fixture.commit(&work, &[("newer.txt", "x\n")], "newer work");
    fixture.git(&work, &["push", "-q", "origin", "older"]);
    fixture.git(&work, &["checkout", "-q", "main"]);
    fixture.git(&work, &["remote", "set-head", "origin", "main"]);
    let repo = Repo::open(&work).unwrap();

    let branches = repo.branches().unwrap();
    let names: Vec<(&str, BranchKind)> = branches
        .iter()
        .map(|branch| (branch.name.as_str(), branch.kind))
        .collect();
    assert_eq!(
        names,
        vec![
            ("newer", BranchKind::Local),
            ("older", BranchKind::Local),
            ("main", BranchKind::Local),
            ("origin/older", BranchKind::Remote),
            ("origin/main", BranchKind::Remote),
        ]
    );
    let main = &branches[2];
    assert!(main.is_head);
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));
    assert_eq!(main.subject, "initial");
    assert_eq!(main.sha, fixture.rev(&work, "main"));
    assert!(branches.iter().filter(|branch| branch.is_head).count() == 1);
    assert_eq!(branches[0].upstream, None);
    assert!(branches[0].committer_time > branches[1].committer_time);
    assert_eq!(repo.current_branch().unwrap().as_deref(), Some("main"));
}

#[test]
fn current_branch_is_none_when_detached_or_unborn() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let repo = Repo::open(&path).unwrap();
    assert_eq!(repo.current_branch().unwrap(), None);
    fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    assert_eq!(repo.current_branch().unwrap().as_deref(), Some("main"));
    fixture.git(&path, &["checkout", "-q", "--detach"]);
    assert_eq!(repo.current_branch().unwrap(), None);
}

fn local(name: &str) -> Branch {
    Branch {
        name: name.to_string(),
        kind: BranchKind::Local,
        sha: String::new(),
        upstream: None,
        is_head: false,
        committer_time: 0,
        subject: String::new(),
    }
}

fn remote(name: &str) -> Branch {
    Branch {
        kind: BranchKind::Remote,
        ..local(name)
    }
}

#[test]
fn default_target_follows_the_remote_head() {
    let fixture = Fixture::new();
    let (_, work, other) = fixture.with_origin();
    fixture.git(&work, &["checkout", "-q", "-b", "trunk"]);
    fixture.git(&work, &["push", "-q", "origin", "trunk"]);
    let clone = Repo::open(&other).unwrap();
    assert_eq!(
        clone.default_target(&clone.branches().unwrap()).as_deref(),
        Some("main")
    );
    fixture.git(&other, &["checkout", "-q", "-b", "feature"]);
    fixture.git(&other, &["branch", "-q", "-D", "main"]);
    assert_eq!(
        clone.default_target(&clone.branches().unwrap()).as_deref(),
        Some("origin/main")
    );
    fixture.git(&other, &["fetch", "-q", "origin"]);
    fixture.git(&other, &["remote", "set-head", "origin", "trunk"]);
    assert_eq!(
        clone.default_target(&clone.branches().unwrap()).as_deref(),
        Some("origin/trunk")
    );
}

#[rstest]
#[case::main_wins(&["develop", "main", "master"], &[], Some("main"))]
#[case::master(&["master", "feature"], &[], Some("master"))]
#[case::develop_before_trunk(&["trunk", "develop"], &[], Some("develop"))]
#[case::name_order_before_kind(&["master"], &["origin/main"], Some("origin/main"))]
#[case::remote_fallback(&["feature"], &["origin/master"], Some("origin/master"))]
#[case::nothing(&["feature"], &["origin/feature"], None)]
fn default_target_falls_back_to_well_known_names(
    #[case] locals: &[&str],
    #[case] remotes: &[&str],
    #[case] expected: Option<&str>,
) {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    let repo = Repo::open(&path).unwrap();
    let branches: Vec<Branch> = locals
        .iter()
        .map(|name| local(name))
        .chain(remotes.iter().map(|name| remote(name)))
        .collect();
    assert_eq!(repo.default_target(&branches).as_deref(), expected);
}

#[test]
fn resolve_and_has_commit() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let sha = fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    fixture.git(&path, &["tag", "-a", "-m", "annotated", "v1"]);
    let repo = Repo::open(&path).unwrap();
    assert_eq!(repo.resolve("main").unwrap(), sha);
    assert_eq!(repo.resolve("v1").unwrap(), sha);
    assert_eq!(repo.resolve(&sha[..10]).unwrap(), sha);
    assert!(repo.resolve("missing").is_err());
    assert!(repo.resolve("--all").is_err());
    assert!(repo.has_commit(&sha));
    assert!(!repo.has_commit(&"0".repeat(40)));
    assert!(!repo.has_commit(&fixture.rev(&path, "HEAD:a.txt")));
}

#[test]
fn remotes_and_upstream_remote() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    fixture.commit(&path, &[("a.txt", "a\n")], "initial");
    let repo = Repo::open(&path).unwrap();
    assert_eq!(repo.remotes().unwrap(), Vec::<Remote>::new());
    assert_eq!(repo.upstream_remote(), None);

    fixture.git(
        &path,
        &["remote", "add", "upstream", "git@example.com:a/b.git"],
    );
    assert_eq!(repo.upstream_remote().as_deref(), Some("upstream"));
    fixture.git(
        &path,
        &["remote", "add", "origin", "https://example.com/c/d"],
    );
    assert_eq!(repo.upstream_remote().as_deref(), Some("origin"));
    assert_eq!(
        repo.remotes().unwrap(),
        vec![
            Remote {
                name: "origin".to_string(),
                url: "https://example.com/c/d".to_string()
            },
            Remote {
                name: "upstream".to_string(),
                url: "git@example.com:a/b.git".to_string()
            },
        ]
    );
    fixture.git(&path, &["config", "branch.main.remote", "upstream"]);
    fixture.git(&path, &["config", "branch.main.merge", "refs/heads/main"]);
    assert_eq!(repo.upstream_remote().as_deref(), Some("upstream"));
}

#[test]
fn commits_are_newest_first() {
    let fixture = Fixture::new();
    let path = fixture.init("repo");
    let base = fixture.commit(&path, &[("a.txt", "1\n")], "one");
    let second = fixture.commit(&path, &[("a.txt", "2\n")], "two");
    let third = fixture.commit(&path, &[("a.txt", "3\n")], "three: with\x1fodd chars");
    let repo = Repo::open(&path).unwrap();
    let commits = repo.commits(&base, "main").unwrap();
    let shas: Vec<&str> = commits.iter().map(|commit| commit.sha.as_str()).collect();
    assert_eq!(shas, vec![third.as_str(), second.as_str()]);
    assert!(commits[0].time > commits[1].time);
    assert_eq!(commits[1].subject, "two");
    assert!(repo.commits("main", "main").unwrap().is_empty());
}

#[tokio::test]
async fn pull_updates_a_remote_tracking_ref() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let before = fixture.rev(&work, "origin/main");
    let after = fixture.commit(&other, &[("o.txt", "o\n")], "upstream");
    fixture.git(&other, &["push", "-q", "origin", "main"]);

    assert_eq!(
        repo.pull("origin/main").await.unwrap(),
        PullOutcome::Updated {
            from: before,
            to: after.clone()
        }
    );
    assert_eq!(fixture.rev(&work, "origin/main"), after);
    assert_eq!(fixture.rev(&work, "main"), fixture.rev(&work, "HEAD"));
    assert_ne!(fixture.rev(&work, "main"), after);
    assert_eq!(
        repo.pull("origin/main").await.unwrap(),
        PullOutcome::UpToDate
    );

    fixture.git(&other, &["checkout", "-q", "-b", "fresh/topic"]);
    let topic = fixture.commit(&other, &[("t.txt", "t\n")], "topic");
    fixture.git(&other, &["push", "-q", "origin", "fresh/topic"]);
    assert_eq!(
        repo.pull("origin/fresh/topic").await.unwrap(),
        PullOutcome::Updated {
            from: "0".repeat(40),
            to: topic
        }
    );
}

#[tokio::test]
async fn pull_fast_forwards_a_branch_that_is_not_checked_out() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    fixture.git(&work, &["branch", "release"]);
    fixture.git(&work, &["push", "-q", "-u", "origin", "release"]);
    let before = fixture.rev(&work, "release");
    fixture.git(&other, &["fetch", "-q", "origin"]);
    fixture.git(&other, &["checkout", "-q", "release"]);
    let after = fixture.commit(&other, &[("r.txt", "r\n")], "release fix");
    fixture.git(&other, &["push", "-q", "origin", "release"]);

    assert_eq!(
        repo.pull("release").await.unwrap(),
        PullOutcome::Updated {
            from: before.clone(),
            to: after.clone()
        }
    );
    assert_eq!(fixture.rev(&work, "release"), after);
    assert_eq!(fixture.rev(&work, "origin/release"), after);
    assert_eq!(fixture.rev(&work, "HEAD"), before);
    assert!(!work.join("r.txt").exists());
    assert_eq!(repo.pull("release").await.unwrap(), PullOutcome::UpToDate);
}

#[tokio::test]
async fn pull_fast_forwards_the_checked_out_branch() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let before = fixture.rev(&work, "main");
    let after = fixture.commit(&other, &[("o.txt", "o\n")], "upstream");
    fixture.git(&other, &["push", "-q", "origin", "main"]);

    assert_eq!(
        repo.pull("main").await.unwrap(),
        PullOutcome::Updated {
            from: before,
            to: after.clone()
        }
    );
    assert_eq!(fixture.rev(&work, "HEAD"), after);
    assert_eq!(std::fs::read_to_string(work.join("o.txt")).unwrap(), "o\n");
    assert_eq!(fixture.git(&work, &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn pull_refuses_to_overwrite_local_changes() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let before = fixture.rev(&work, "main");
    fixture.commit(&other, &[("README.md", "changed upstream\n")], "upstream");
    fixture.git(&other, &["push", "-q", "origin", "main"]);
    fixture.write(&work, "README.md", "local edit\n");

    let error = repo.pull("main").await.unwrap_err();
    assert!(matches!(error, GitError::Command { .. }), "{error}");
    assert_eq!(fixture.rev(&work, "HEAD"), before);
    assert_eq!(
        std::fs::read_to_string(work.join("README.md")).unwrap(),
        "local edit\n"
    );
}

#[tokio::test]
async fn pull_rejects_diverged_branches() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let local = fixture.commit(&work, &[("local.txt", "l\n")], "local");
    fixture.commit(&other, &[("o.txt", "o\n")], "upstream");
    fixture.git(&other, &["push", "-q", "origin", "main"]);

    assert!(matches!(
        repo.pull("main").await,
        Err(GitError::NotFastForward(branch)) if branch == "main"
    ));
    assert_eq!(fixture.rev(&work, "main"), local);
}

#[tokio::test]
async fn pull_of_a_branch_ahead_of_its_upstream_is_up_to_date() {
    let fixture = Fixture::new();
    let (repo, work, _) = fixture.with_origin();
    let local = fixture.commit(&work, &[("local.txt", "l\n")], "local");
    assert_eq!(repo.pull("main").await.unwrap(), PullOutcome::UpToDate);
    assert_eq!(fixture.rev(&work, "main"), local);
}

#[tokio::test]
async fn pull_without_upstream_or_unknown_branch_fails() {
    let fixture = Fixture::new();
    let (repo, work, _) = fixture.with_origin();
    fixture.git(&work, &["branch", "topic"]);
    assert!(matches!(
        repo.pull("topic").await,
        Err(GitError::NoUpstream(branch)) if branch == "topic"
    ));
    assert!(matches!(
        repo.pull("nowhere/branch").await,
        Err(GitError::UnknownBranch(_))
    ));
    assert!(matches!(
        repo.pull("--force").await,
        Err(GitError::Parse(_))
    ));
}

#[tokio::test]
async fn pull_refuses_a_branch_checked_out_in_another_worktree() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    fixture.git(&work, &["branch", "release"]);
    fixture.git(&work, &["push", "-q", "-u", "origin", "release"]);
    let before = fixture.rev(&work, "release");
    let worktree = fixture.path("release-worktree");
    fixture.git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
            "release",
        ],
    );
    fixture.git(&other, &["fetch", "-q", "origin"]);
    fixture.git(&other, &["checkout", "-q", "release"]);
    fixture.commit(&other, &[("r.txt", "r\n")], "release fix");
    fixture.git(&other, &["push", "-q", "origin", "release"]);

    assert!(matches!(
        repo.pull("release").await,
        Err(GitError::CheckedOutElsewhere { branch, .. }) if branch == "release"
    ));
    assert_eq!(fixture.rev(&work, "release"), before);
}

#[tokio::test]
async fn fetch_updates_remote_refs() {
    let fixture = Fixture::new();
    let (repo, work, other) = fixture.with_origin();
    let after = fixture.commit(&other, &[("o.txt", "o\n")], "upstream");
    fixture.git(&other, &["push", "-q", "origin", "main"]);
    repo.fetch("origin", &[]).await.unwrap();
    assert_eq!(fixture.rev(&work, "origin/main"), after);
    assert!(matches!(
        repo.fetch("origin", &["refs/heads/missing".to_string()])
            .await,
        Err(GitError::Command { .. })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn fetch_is_stopped_after_the_timeout() {
    let fixture = Fixture::new();
    let (repo, work, _) = fixture.with_origin();
    let origin = fixture.path("origin.git");
    fixture.git(&work, &["remote", "add", "slow", origin.to_str().unwrap()]);
    fixture.git(
        &work,
        &[
            "config",
            "remote.slow.uploadpack",
            "sleep 5; git-upload-pack",
        ],
    );
    let repo = repo.with_network_timeout(Duration::from_millis(500));
    let started = Instant::now();
    let error = repo.fetch("slow", &[]).await.unwrap_err();
    assert!(matches!(error, GitError::TimedOut { .. }), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[rstest]
#[case::https(
    "https://gitlab.com/group/project.git",
    "gitlab.com",
    "group/project",
    "https://gitlab.com"
)]
#[case::https_no_suffix_trailing_slash(
    "https://gitlab.com/group/project/",
    "gitlab.com",
    "group/project",
    "https://gitlab.com"
)]
#[case::https_credentials_port(
    "https://user:tok@git.example.com:8443/g/sub/p",
    "git.example.com:8443",
    "g/sub/p",
    "https://git.example.com:8443"
)]
#[case::http(
    "http://localhost:8080/a/b.git",
    "localhost:8080",
    "a/b",
    "http://localhost:8080"
)]
#[case::ssh_port(
    "ssh://git@host.example:2222/g/p.git",
    "host.example",
    "g/p",
    "https://host.example"
)]
#[case::ssh_plain(
    "ssh://host.example/g/p",
    "host.example",
    "g/p",
    "https://host.example"
)]
#[case::scp(
    "git@gitlab.example.com:g/p.git",
    "gitlab.example.com",
    "g/p",
    "https://gitlab.example.com"
)]
#[case::scp_nested_slash(
    "git@gitlab.example.com:/g/s/p.git/",
    "gitlab.example.com",
    "g/s/p",
    "https://gitlab.example.com"
)]
#[case::scp_without_user(
    "gitlab.example.com:g/p",
    "gitlab.example.com",
    "g/p",
    "https://gitlab.example.com"
)]
fn parse_remote_url_cases(
    #[case] url: &str,
    #[case] host: &str,
    #[case] path: &str,
    #[case] https_base: &str,
) {
    assert_eq!(
        parse_remote_url(url),
        Some(RemoteLocation {
            host: host.to_string(),
            path: path.to_string(),
            https_base: https_base.to_string(),
        })
    );
}

#[rstest]
#[case::local_path("/srv/git/project.git")]
#[case::relative_path("../project")]
#[case::windows_drive("C:\\repos\\project")]
#[case::file_url("file:///srv/git/project.git")]
#[case::no_path("https://gitlab.com")]
#[case::empty("")]
fn parse_remote_url_rejects(#[case] url: &str) {
    assert_eq!(parse_remote_url(url), None);
}

#[test]
fn errors_read_well() {
    let error = GitError::Command {
        args: "fetch origin".to_string(),
        status: Some(128),
        stderr: "fatal: could not read from remote repository\n".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "`git fetch origin` failed (exit code 128): fatal: could not read from remote repository"
    );
}
