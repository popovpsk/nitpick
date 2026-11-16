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
    let fp = fingerprint("aaa", "bbb");
    let store = ViewedStore::from_marks([("src/a.rs", fp.as_str(), 1_000)]);
    assert!(store.is_viewed("src/a.rs", &fp));
    assert!(!store.is_viewed("src/a.rs", &fingerprint("aaa", "ccc")));
    assert!(!store.is_viewed("src/b.rs", &fp));
}

#[test]
fn other_viewed_versions_come_newest_first() {
    let mut store = ViewedStore::new();
    let (a, b, c) = (
        fingerprint("o", "a"),
        fingerprint("o", "b"),
        fingerprint("o", "c"),
    );
    assert!(store.viewed_versions_except("f", &c).is_empty());
    store.set_viewed("f", &a, true, 10);
    store.set_viewed("f", &b, true, 20);
    store.set_viewed("g", &c, true, 30);
    assert_eq!(
        store.viewed_versions_except("f", &c),
        vec![b.as_str(), a.as_str()]
    );
    assert_eq!(store.viewed_versions_except("f", &b), vec![a.as_str()]);
    assert_eq!(fingerprint_blobs(&b), Some(("o", "b")));
}

#[test]
fn unmarking_removes_the_mark() {
    let mut store = ViewedStore::new();
    store.set_viewed("a", "f1", true, 0);
    store.set_viewed("b", "f2", true, 0);
    store.set_viewed("b", "f2", false, 0);
    assert!(store.is_viewed("a", "f1"));
    assert!(!store.is_viewed("b", "f2"));
}

#[test]
fn a_chain_of_viewed_steps_covers_the_whole_change() {
    let mut store = ViewedStore::new();
    store.set_viewed("f", &fingerprint("0", "a"), true, 1);
    assert!(!store.is_viewed("f", &fingerprint("0", "b")));
    store.set_viewed("f", &fingerprint("a", "b"), true, 2);
    assert!(store.is_viewed("f", &fingerprint("0", "b")));
    assert!(!store.is_viewed("f", &fingerprint("x", "b")));
}

#[test]
fn a_rename_chains_through_the_source_path() {
    let mut store = ViewedStore::new();
    store.set_viewed("old.rs", &fingerprint("0", "s"), true, 1);
    store.set_viewed("new.rs", &fingerprint("s", "x"), true, 2);
    assert!(store.is_viewed("new.rs", &fingerprint("0", "x")));
}

#[test]
fn an_identical_change_elsewhere_does_not_count() {
    let mut store = ViewedStore::new();
    store.set_viewed("g", &fingerprint("0", "x"), true, 1);
    assert!(!store.is_viewed("f", &fingerprint("0", "x")));
    assert!(store.covering_marks("f", &fingerprint("0", "x")).is_empty());
}

#[test]
fn covering_marks_unview_a_chained_file() {
    let mut store = ViewedStore::new();
    let (first, second, other) = (
        fingerprint("0", "a"),
        fingerprint("a", "b"),
        fingerprint("0", "z"),
    );
    store.set_viewed("f", &first, true, 1);
    store.set_viewed("f", &second, true, 2);
    store.set_viewed("g", &other, true, 3);
    store.set_viewed("h", &fingerprint("0", "y"), true, 3);
    store.set_viewed("h", &fingerprint("y", "0"), true, 3);
    let whole = fingerprint("0", "b");
    let mut marks = store.covering_marks("f", &whole);
    marks.sort();
    assert_eq!(
        marks,
        vec![
            ("f".to_string(), first.clone()),
            ("f".to_string(), second.clone())
        ]
    );
    for (path, fp) in &marks {
        store.set_viewed(path, fp, false, 4);
    }
    assert!(!store.is_viewed("f", &whole));
    assert!(store.is_viewed("g", &other));
    assert!(store.is_viewed("h", &fingerprint("y", "0")));
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

#[rstest]
#[case::vue("src/components/App.vue", Some("vue"))]
#[case::make_include("build/rules.mk", Some("make"))]
#[case::make_mak("win/nmake.mak", Some("make"))]
#[case::makefile_am("Makefile.am", Some("make"))]
#[case::gnumakefile_suffix("sub/GNUmakefile.inc", Some("make"))]
#[case::backslash_path("tools\\deps.mk", Some("make"))]
#[case::zsh_script("scripts/install.zsh", Some("bash"))]
#[case::zshrc("home/.zshrc", Some("bash"))]
#[case::cargo_lock("Cargo.lock", Some("toml"))]
#[case::gemfile("Gemfile", Some("ruby"))]
#[case::cpp_header("include/view.hh", Some("cpp"))]
#[case::upper_case_extension("docs/GUIDE.MARKDOWN", Some("markdown"))]
#[case::patch("fixes/0001-fix.patch", Some("diff"))]
#[case::bazel("tools/defs.bzl", Some("python"))]
#[case::cargo_make_toml("Makefile.toml", None)]
#[case::makefile_yaml("ci/Makefile.yml", None)]
#[case::detected_upstream_makefile("Makefile", None)]
#[case::detected_upstream_proto("api/v1/service.proto", None)]
#[case::detected_upstream_csharp("src/App.cs", None)]
#[case::detected_upstream_java("src/Main.java", None)]
#[case::no_grammar_dockerfile("Dockerfile", None)]
#[case::no_extension("LICENSE", None)]
fn language_overrides(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(language_override(path), expected);
}

const CHANGES_PATCH: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,12 +1,11 @@
-first
+FIRST
 two
 three
 four
 five
-six
-seven
+SIX
 eight
+inserted
 nine
 ten
 eleven
-twelve
diff --git a/new.rs b/new.rs
new file mode 100644
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+a
+b
";

#[test]
fn changes_are_runs_of_changed_lines_with_context_above_as_anchor() {
    let maps = line_maps(CHANGES_PATCH);
    let changes = maps[0].changes();
    assert_eq!(
        changes,
        &[
            Change {
                first: LineKind::Removed { old: 1 },
                anchor: LineKind::Removed { old: 1 },
            },
            Change {
                first: LineKind::Removed { old: 6 },
                anchor: LineKind::Context { old: 4, new: 4 },
            },
            Change {
                first: LineKind::Added { new: 8 },
                anchor: LineKind::Context { old: 8, new: 7 },
            },
            Change {
                first: LineKind::Removed { old: 12 },
                anchor: LineKind::Context { old: 10, new: 10 },
            },
        ]
    );
    assert_eq!(
        maps[1].changes(),
        &[Change {
            first: LineKind::Added { new: 1 },
            anchor: LineKind::Added { new: 1 },
        }]
    );
}

fn whole_file_patch(ops: &str) -> String {
    let old = ops.chars().filter(|op| *op != '+').count();
    let new = ops.chars().filter(|op| *op != '-').count();
    let mut patch = format!("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{old} +1,{new} @@\n");
    for (ix, op) in ops.chars().enumerate() {
        let prefix = if op == '.' { ' ' } else { op };
        patch.push_str(&format!("{prefix}line {ix}\n"));
    }
    patch
}

#[rstest]
#[case::added_line(".........+..........", Side::New, Some(3), &[7])]
#[case::added_line_old_side(".........+..........", Side::Old, Some(3), &[7])]
#[case::removed_line(".........-..........", Side::New, Some(3), &[7])]
#[case::removed_line_old_side(".........-..........", Side::Old, Some(3), &[7])]
#[case::short_fold_stays_open("......+.......+......", Side::New, Some(3), &[4])]
#[case::two_line_fold_folds("......+........+......", Side::New, Some(3), &[4, 13])]
#[case::change_at_the_top("+..........", Side::New, Some(3), &[1])]
#[case::short_fold_at_the_top("..+..........", Side::New, Some(3), &[1])]
#[case::replaced_block("......--+.........", Side::New, Some(3), &[4])]
#[case::no_folding("......+........+......", Side::New, None, &[1])]
#[case::no_changes("......", Side::New, Some(3), &[])]
fn shown_block_starts_follow_the_folding(
    #[case] ops: &str,
    #[case] side: Side,
    #[case] context: Option<usize>,
    #[case] expected: &[u32],
) {
    let map = &line_maps(&whole_file_patch(ops))[0];
    assert_eq!(map.shown_block_starts(side, context, 2), expected);
}

#[rstest]
#[case::changed_line(".........+..........", Side::New, 10, true)]
#[case::three_below_an_added_line(".........+..........", Side::New, 13, true)]
#[case::four_below_an_added_line(".........+..........", Side::New, 14, false)]
#[case::three_above_a_removed_line(".........-..........", Side::Old, 7, true)]
#[case::four_above_a_removed_line(".........-..........", Side::Old, 6, false)]
#[case::three_below_a_removed_block("......----..........", Side::New, 9, true)]
#[case::four_below_a_removed_block("......----..........", Side::New, 10, false)]
#[case::no_changes("..........", Side::New, 5, false)]
fn near_change_follows_unified_context(
    #[case] ops: &str,
    #[case] side: Side,
    #[case] line: u32,
    #[case] expected: bool,
) {
    let map = &line_maps(&whole_file_patch(ops))[0];
    let kind = map.locate(side, line).unwrap();
    assert_eq!(map.near_change(kind, 3), expected);
}

#[test]
fn rows_follow_the_patch_order() {
    let maps = line_maps(PATCH);
    assert_eq!(
        maps[0].rows(),
        vec![
            LineKind::Context { old: 1, new: 1 },
            LineKind::Removed { old: 2 },
            LineKind::Added { new: 2 },
            LineKind::Added { new: 3 },
            LineKind::Context { old: 3, new: 4 },
            LineKind::Context { old: 4, new: 5 },
            LineKind::Context { old: 5, new: 6 },
        ]
    );
    assert!(maps[1].rows().is_empty());
    assert_eq!(
        maps[3].rows(),
        vec![
            LineKind::Context { old: 10, new: 10 },
            LineKind::Removed { old: 11 },
            LineKind::Context { old: 12, new: 11 },
        ]
    );
}

#[test]
fn shown_lines_mark_what_the_folding_keeps() {
    let map = &line_maps(&whole_file_patch(".........+.........."))[0];
    let shown = map.shown_lines(Side::New, Some(3), 2);
    let lines: Vec<usize> = (0..shown.len())
        .filter(|&ix| shown[ix])
        .map(|ix| ix + 1)
        .collect();
    assert_eq!(lines, vec![7, 8, 9, 10, 11, 12, 13]);
    assert!(
        map.shown_lines(Side::New, None, 2)
            .iter()
            .all(|shown| *shown)
    );
}

#[test]
fn shown_block_starts_begin_each_hunk_of_a_short_patch() {
    let patch = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,7 +1,7 @@
 a
 b
 c
-d
+D
 e
 f
 g
@@ -20,7 +20,7 @@
 t
 u
 v
-w
+W
 x
 y
 z
";
    let map = &line_maps(patch)[0];
    assert_eq!(map.shown_block_starts(Side::New, Some(3), 2), vec![1, 20]);
    assert_eq!(map.shown_block_starts(Side::Old, None, 2), vec![1, 20]);
}

#[test]
fn a_filtered_tree_leaves_hidden_files_out_and_keeps_indices() {
    let files = [
        file("store/user.go"),
        file("store/user_mock.go"),
        file("vendor/lib/a.go"),
        file("main.go"),
    ];
    let (rules, _) = HideRules::new(&["*_mock.go".to_string(), "vendor/".to_string()], &[]);
    let tree = FileTree::build_with(&files, |ix| !rules.is_hidden(&files[ix].path));
    assert_eq!(tree.file_order(), &[0, 3]);
    assert_eq!(tree.file_node(1), None);
    assert_eq!(tree.file_node(2), None);
    assert_eq!(
        labels(&tree, &HashSet::new()),
        vec!["store", "  user.go", "main.go"]
    );
    let progress = tree.progress(&[true, false, false, false]);
    let root = tree.roots()[0];
    assert_eq!(
        progress[root],
        Progress {
            viewed: 1,
            total: 1
        }
    );
}

#[test]
fn a_flat_tree_lists_files_by_path_and_an_uncompacted_tree_keeps_every_folder() {
    let files = [
        file("src/b.rs"),
        file("a/deep/x.rs"),
        file("README.md"),
        file("src/a.rs"),
    ];
    let flat = FileTree::build_shaped(
        &files,
        |ix| ix != 2,
        TreeShape {
            flat: true,
            compact: true,
        },
    );
    assert_eq!(flat.file_order(), &[1, 3, 0]);
    assert_eq!(labels(&flat, &HashSet::new()), vec!["x.rs", "a.rs", "b.rs"]);
    assert_eq!(flat.node(flat.file_node(1).unwrap()).path, "a/deep/x.rs");
    assert_eq!(flat.file_node(2), None);

    let loose = FileTree::build_shaped(
        &files,
        |_| true,
        TreeShape {
            flat: false,
            compact: false,
        },
    );
    assert_eq!(
        labels(&loose, &HashSet::new()),
        vec![
            "a",
            "  deep",
            "    x.rs",
            "src",
            "  a.rs",
            "  b.rs",
            "README.md"
        ]
    );
}

#[test]
fn untracked_files_count_nowhere_in_progress() {
    let files = [file("src/a.rs"), file("src/a_mock.rs")];
    let tree = FileTree::build(&files);
    let progress = tree.progress_tracked(&[true, false], |ix| ix != 1);
    let root = tree.roots()[0];
    assert_eq!(
        progress[root],
        Progress {
            viewed: 1,
            total: 1
        }
    );
    let mock = tree.file_node(1).unwrap();
    assert_eq!(progress[mock], Progress::default());
}

#[test]
fn completed_folders_are_the_finished_ancestors_of_the_marked_files() {
    let files = [
        file("a/b/x.rs"),
        file("a/b/y.rs"),
        file("a/c/z.rs"),
        file("a/b/gen_mock.rs"),
    ];
    let tree = FileTree::build(&files);
    let tracked = |ix: usize| ix != 3;
    let progress = tree.progress_tracked(&[true, false, true, false], tracked);
    assert!(tree.completed_folders(&[0], &progress).is_empty());
    assert_eq!(tree.completed_folders(&[2], &progress), vec!["a/c"]);
    let progress = tree.progress_tracked(&[true, true, true, false], tracked);
    assert_eq!(tree.completed_folders(&[1, 0], &progress), vec!["a/b", "a"]);
}

#[cfg(unix)]
#[test]
fn atomic_writes_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("review-core-write-{}", std::process::id()));
    let path = dir.join("settings.json");
    crate::write_atomic(&path, b"{}").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(mode, 0o600);
}
