use std::collections::HashSet;

use rstest::rstest;

use crate::*;

fn file(path: &str) -> TreeFile {
    TreeFile {
        path: path.to_string(),
        status: FileStatus::Modified,
        additions: 1,
        deletions: 0,
    }
}

fn labels(tree: &FileTree, collapsed: &HashSet<String>) -> Vec<String> {
    tree.rows(collapsed)
        .into_iter()
        .map(|row| format!("{}{}", "  ".repeat(row.depth), tree.node(row.node).name))
        .collect()
}

#[test]
fn single_child_folders_are_compressed_and_dirs_come_first() {
    let files = [
        file("src/main.rs"),
        file("crates/app/src/lib.rs"),
        file("crates/app/src/view.rs"),
        file("README.md"),
        file("src/util/a.rs"),
    ];
    let tree = FileTree::build(&files);
    assert_eq!(
        labels(&tree, &HashSet::new()),
        vec![
            "crates/app/src",
            "  lib.rs",
            "  view.rs",
            "src",
            "  util",
            "    a.rs",
            "  main.rs",
            "README.md",
        ]
    );
    assert_eq!(tree.file_order(), &[1, 2, 4, 0, 3]);
}

#[test]
fn collapsed_dirs_hide_their_rows() {
    let files = [file("a/b/x.rs"), file("a/c/y.rs"), file("z.rs")];
    let tree = FileTree::build(&files);
    let collapsed: HashSet<String> = ["a/b".to_string()].into();
    assert_eq!(
        labels(&tree, &collapsed),
        vec!["a", "  b", "  c", "    y.rs", "z.rs"]
    );
    let all: HashSet<String> = tree.dir_paths().map(str::to_string).collect();
    assert_eq!(labels(&tree, &all), vec!["a", "z.rs"]);
}

#[test]
fn progress_rolls_up_to_folders() {
    let files = [file("a/x.rs"), file("a/y.rs"), file("b/z.rs")];
    let tree = FileTree::build(&files);
    let progress = tree.progress(&[true, false, true]);
    let a = tree.roots()[0];
    let b = tree.roots()[1];
    assert_eq!(
        progress[a],
        Progress {
            viewed: 1,
            total: 2
        }
    );
    assert!(progress[a].is_partial());
    assert!(progress[b].is_complete());
    let x = tree.file_node(0).unwrap();
    assert_eq!(
        tree.ancestors(x)
            .map(|n| n.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a"]
    );
    assert_eq!(tree.files_under(a), vec![0, 1]);
}

#[test]
fn viewed_marks_follow_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new("/work/my repo");
    let mut store = ViewedStore::load(dir.path(), repo, 1_000);
    let fp = fingerprint("aaa", "bbb");
    store.set_viewed("src/a.rs", &fp, true, 1_000);
    store.save().unwrap();
    let store = ViewedStore::load(dir.path(), repo, 2_000);
    assert!(store.is_viewed("src/a.rs", &fp));
    assert!(!store.is_viewed("src/a.rs", &fingerprint("aaa", "ccc")));
    assert!(!store.is_viewed("src/b.rs", &fp));
    let other = ViewedStore::load(dir.path(), std::path::Path::new("/work/other"), 2_000);
    assert!(!other.is_viewed("src/a.rs", &fp));
}

#[test]
fn old_viewed_marks_expire_and_unmarking_removes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new("/r");
    let mut store = ViewedStore::load(dir.path(), repo, 0);
    store.set_viewed("a", "f1", true, 0);
    store.set_viewed("b", "f2", true, 0);
    store.set_viewed("b", "f2", false, 0);
    assert!(!store.is_viewed("b", "f2"));
    store.save().unwrap();
    assert!(ViewedStore::load(dir.path(), repo, 10).is_viewed("a", "f1"));
    assert!(!ViewedStore::load(dir.path(), repo, 400 * 24 * 3600).is_viewed("a", "f1"));
}

#[test]
fn corrupt_store_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new("/r");
    std::fs::write(ViewedStore::file_for(dir.path(), repo), b"{not json").unwrap();
    let store = ViewedStore::load(dir.path(), repo, 0);
    assert!(!store.is_viewed("a", "b"));
}

const PATCH: &str = "diff --git a/one.rs b/one.rs
index 1111111..2222222 100644
--- a/one.rs
+++ b/one.rs
@@ -1,5 +1,6 @@
 fn a() {}
-fn b() {}
+fn b2() {}
+fn b3() {}
 fn c() {}
 fn d() {}
 fn e() {}
diff --git a/bin.png b/bin.png
index 3333333..4444444 100644
Binary files a/bin.png and b/bin.png differ
diff --git a/two.txt b/two.txt
new file mode 100644
index 0000000..5555555
--- /dev/null
+++ b/two.txt
@@ -0,0 +1,2 @@
+hello
+world
\\ No newline at end of file
diff --git a/three.txt b/three.txt
index 6666666..7777777 100644
--- a/three.txt
+++ b/three.txt
@@ -10,3 +10,2 @@ fn header() {
 ten
--- removed line that looks like a header
 twelve
";

#[test]
fn line_maps_follow_patch_order() {
    let maps = line_maps(PATCH);
    assert_eq!(maps.len(), 4);
    assert!(maps[1].is_empty());
}

#[rstest]
#[case::context_new(0, Side::New, 1, Some(LineKind::Context { old: 1, new: 1 }))]
#[case::removed(0, Side::Old, 2, Some(LineKind::Removed { old: 2 }))]
#[case::added(0, Side::New, 3, Some(LineKind::Added { new: 3 }))]
#[case::shifted_context(0, Side::Old, 4, Some(LineKind::Context { old: 4, new: 5 }))]
#[case::shifted_context_new(0, Side::New, 6, Some(LineKind::Context { old: 5, new: 6 }))]
#[case::past_end(0, Side::New, 7, None)]
#[case::zero(0, Side::New, 0, None)]
#[case::new_file(2, Side::New, 2, Some(LineKind::Added { new: 2 }))]
#[case::new_file_old_side(2, Side::Old, 1, None)]
#[case::header_like_removed(3, Side::Old, 11, Some(LineKind::Removed { old: 11 }))]
#[case::after_header_like(3, Side::Old, 12, Some(LineKind::Context { old: 12, new: 11 }))]
#[case::outside_hunk(3, Side::Old, 3, None)]
fn line_map_locates_lines(
    #[case] file: usize,
    #[case] side: Side,
    #[case] line: u32,
    #[case] expected: Option<LineKind>,
) {
    assert_eq!(line_maps(PATCH)[file].locate(side, line), expected);
}
