use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use tempfile::TempDir;

use super::{find_repositories, strip_verbatim};

fn repo(root: &Path, rel: &str) {
    fs::create_dir_all(root.join(rel).join(".git")).unwrap();
}

fn linked(root: &Path, rel: &str) {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
}

fn found(root: &Path) -> Vec<String> {
    let base = fs::canonicalize(root).unwrap();
    find_repositories(root)
        .unwrap()
        .into_iter()
        .map(|path| {
            path.strip_prefix(&base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn finds_repositories_and_applies_the_skip_rules() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    repo(root, "alpha");
    repo(root, "alpha/vendored/inner");
    repo(root, "group/beta");
    repo(root, "group/sub/gamma");
    repo(root, ".hidden/secret");
    for skipped in ["node_modules", "target", "vendor", "build", "dist"] {
        repo(root, &format!("{skipped}/nested"));
    }
    linked(root, "worktree");
    linked(root, "group/submodule-like");
    fs::create_dir_all(root.join("bare.git/objects")).unwrap();
    fs::create_dir_all(root.join("bare.git/refs")).unwrap();
    fs::write(root.join("bare.git/HEAD"), "ref: refs/heads/main\n").unwrap();
    repo(root, "bare.git/refs/not-scanned");
    fs::write(root.join("notes.txt"), "not a dir").unwrap();
    assert_eq!(found(root), vec!["alpha", "group/beta", "group/sub/gamma"]);
}

#[rstest]
#[case::at_the_depth_limit("d1/d2/d3/d4/repo", true)]
#[case::past_the_depth_limit("d1/d2/d3/d4/d5/repo", false)]
fn depth_is_bounded(#[case] rel: &str, #[case] expected: bool) {
    let dir = TempDir::new().unwrap();
    repo(dir.path(), rel);
    assert_eq!(found(dir.path()) == vec![rel.to_string()], expected);
}

#[test]
fn a_repository_root_is_returned_alone() {
    let dir = TempDir::new().unwrap();
    repo(dir.path(), "");
    repo(dir.path(), "nested");
    assert_eq!(found(dir.path()), vec![String::new()]);
}

#[test]
fn a_missing_root_is_an_error() {
    let dir = TempDir::new().unwrap();
    assert!(find_repositories(&dir.path().join("missing")).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_are_followed_once_and_loops_are_not() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    repo(&root, "real");
    repo(&outside, "linked-in");
    repo(dir.path(), "elsewhere/unrelated");
    symlink(root.join("real"), root.join("alias")).unwrap();
    symlink(&root, root.join("loop")).unwrap();
    symlink(dir.path(), root.join("parent")).unwrap();
    symlink(outside.join("linked-in"), root.join("external")).unwrap();
    let outside = fs::canonicalize(&outside).unwrap();
    let mut result = find_repositories(&root).unwrap();
    result.sort();
    let mut expected: Vec<PathBuf> = vec![
        fs::canonicalize(root.join("real")).unwrap(),
        outside.join("linked-in"),
    ];
    expected.sort();
    assert_eq!(result, expected);
}

#[rstest]
#[case::drive(r"\\?\C:\Users\me\repo", r"C:\Users\me\repo")]
#[case::unc(r"\\?\UNC\server\share", r"\\?\UNC\server\share")]
#[case::plain("/home/me/repo", "/home/me/repo")]
fn verbatim_prefix_is_dropped_from_drive_paths(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        strip_verbatim(PathBuf::from(input)),
        PathBuf::from(expected)
    );
}
