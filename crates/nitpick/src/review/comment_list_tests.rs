use std::collections::HashMap;

use gpui::SharedString;
use review_core::FileStatus;
use rstest::rstest;

use super::{FilePaths, build_code_threads, general_threads, preview};
use crate::review::{DiscussionView, NoteView};
use crate::settings_store::CommentSort;

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
    resolved_discussion(path, line, replies, false)
}

fn resolved_discussion(
    path: Option<&str>,
    line: Option<u32>,
    replies: usize,
    resolved: bool,
) -> DiscussionView {
    DiscussionView {
        id: String::new(),
        notes: (0..=replies).map(|_| note("text")).collect(),
        can_reply: true,
        resolve_id: None,
        resolved,
        position: path.map(|path| forge::Position {
            new_path: Some(path.to_string()),
            new_line: line,
            ..Default::default()
        }),
        location: None,
        preview: SharedString::default(),
        replies: None,
        created: 0,
        active: 0,
    }
}

fn timed(mut discussion: DiscussionView, created: i64, active: i64) -> DiscussionView {
    discussion.created = created;
    discussion.active = active;
    discussion
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
        resolved_discussion(Some("src/a.rs"), Some(1), 0, true),
    ];
    let by_file: HashMap<SharedString, Vec<usize>> =
        [(a.clone(), vec![0, 3, 5]), (b.clone(), vec![2])].into();
    let threads = build_code_threads(
        &discussions,
        &files,
        &[1, 0],
        &by_file,
        None,
        CommentSort::ByFile,
    );
    let order: Vec<(usize, bool, bool, bool)> = threads
        .iter()
        .map(|t| (t.discussion, t.first_in_section, t.first_in_file, t.placed))
        .collect();
    assert_eq!(
        order,
        vec![
            (2, true, true, true),
            (3, false, true, true),
            (0, false, false, true),
            (4, false, true, false),
            (5, true, true, true),
        ]
    );
    assert_eq!(threads[0].status, Some(FileStatus::Added));
    assert_eq!(
        threads[1].line_label.as_ref().map(|l| l.as_ref()),
        Some("L3")
    );
    assert_eq!(threads[3].file, None);
}

#[rstest]
#[case::newest(CommentSort::Newest, vec![(2, true, true), (0, false, false), (1, false, true), (3, true, true)])]
#[case::oldest(CommentSort::Oldest, vec![(0, true, true), (1, false, true), (2, false, true), (3, true, true)])]
fn threads_by_time_keep_open_ones_first_and_name_the_file_when_it_changes(
    #[case] sort: CommentSort,
    #[case] expected: Vec<(usize, bool, bool)>,
) {
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
            status: FileStatus::Modified,
        },
    ];
    let discussions = [
        timed(discussion(Some("src/a.rs"), Some(1), 0), 10, 50),
        timed(discussion(Some("src/b.rs"), Some(1), 0), 20, 30),
        timed(discussion(Some("src/a.rs"), Some(9), 0), 30, 60),
        timed(
            resolved_discussion(Some("src/b.rs"), Some(2), 0, true),
            5,
            90,
        ),
    ];
    let by_file: HashMap<SharedString, Vec<usize>> =
        [(a.clone(), vec![0, 2]), (b.clone(), vec![1, 3])].into();
    let threads = build_code_threads(&discussions, &files, &[0, 1], &by_file, None, sort);
    let order: Vec<(usize, bool, bool)> = threads
        .iter()
        .map(|t| (t.discussion, t.first_in_section, t.first_in_file))
        .collect();
    assert_eq!(order, expected);
}

#[rstest]
#[case::newest(CommentSort::Newest, vec![2, 0])]
#[case::oldest(CommentSort::Oldest, vec![0, 2])]
#[case::by_file_is_oldest(CommentSort::ByFile, vec![0, 2])]
fn general_threads_follow_the_sort(#[case] sort: CommentSort, #[case] expected: Vec<usize>) {
    let discussions = [
        timed(discussion(None, None, 0), 10, 100),
        timed(discussion(Some("src/a.rs"), Some(1), 0), 20, 20),
        timed(discussion(None, None, 1), 30, 200),
    ];
    assert_eq!(general_threads(&discussions, sort), expected);
}
