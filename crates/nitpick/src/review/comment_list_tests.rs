use std::collections::HashMap;

use gpui::SharedString;
use review_core::FileStatus;
use rstest::rstest;

use super::{FilePaths, build_code_threads, preview};
use crate::review::{DiscussionView, NoteView};

fn note(body: &str) -> NoteView {
    NoteView {
        id: 1,
        author: "Ada".into(),
        avatar: None,
        initials: "A".into(),
        tint: 0,
        long: false,
        own: false,
        meta: SharedString::default(),
        body: body.to_string().into(),
    }
}

fn discussion(path: Option<&str>, line: Option<u32>, replies: usize) -> DiscussionView {
    DiscussionView {
        id: String::new(),
        notes: (0..=replies).map(|_| note("text")).collect(),
        can_reply: true,
        resolve_id: None,
        resolved: false,
        position: path.map(|path| forge::Position {
            new_path: Some(path.to_string()),
            new_line: line,
            ..Default::default()
        }),
        location: None,
    }
}

#[rstest]
#[case::heading("## Summary\n\nbody", "Summary")]
#[case::blank_lines("\n\n  first real line  \nsecond", "first real line")]
#[case::quote("> quoted", "quoted")]
#[case::empty("", "")]
fn previews_take_the_first_meaningful_line(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(preview(body).as_ref(), expected);
}

#[test]
fn long_previews_are_cut() {
    let text = preview(&"x".repeat(500));
    assert!(text.ends_with('…'));
    assert_eq!(text.chars().count(), super::PREVIEW_CHARS + 1);
}

#[test]
fn threads_follow_the_tree_order_then_the_line_and_mark_each_file_once() {
    let a: SharedString = "src/a.rs".into();
    let b: SharedString = "src/b.rs".into();
    let files = [
        FilePaths {
            path: &a,
            old_path: "src/a.rs",
            new_path: "src/a.rs",
            status: FileStatus::Modified,
        },
        FilePaths {
            path: &b,
            old_path: "src/b.rs",
            new_path: "src/b.rs",
            status: FileStatus::Added,
        },
    ];
    let discussions = [
        discussion(Some("src/a.rs"), Some(30), 0),
        discussion(None, None, 0),
        discussion(Some("src/b.rs"), Some(5), 2),
        discussion(Some("src/a.rs"), Some(3), 0),
        discussion(Some("gone.rs"), Some(1), 0),
    ];
    let by_file: HashMap<SharedString, Vec<usize>> =
        [(a.clone(), vec![0, 3]), (b.clone(), vec![2])].into();
    let threads = build_code_threads(&discussions, &files, &[1, 0], &by_file, None);
    let order: Vec<(usize, bool, bool)> = threads
        .iter()
        .map(|t| (t.discussion, t.first_in_file, t.placed))
        .collect();
    assert_eq!(
        order,
        vec![
            (2, true, true),
            (3, true, true),
            (0, false, true),
            (4, true, false),
        ]
    );
    assert_eq!(threads[0].replies.as_ref().map(|r| r.as_ref()), Some("2"));
    assert_eq!(threads[0].status, Some(FileStatus::Added));
    assert_eq!(
        threads[1].line_label.as_ref().map(|l| l.as_ref()),
        Some("L3")
    );
    assert_eq!(threads[3].file, None);
}
