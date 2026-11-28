use review_core::{LineKind, Side};
use rstest::rstest;

use super::{FindText, Hit, find_hits, seed_query, start_hit};

#[rstest]
#[case::word("parse", Some("parse"))]
#[case::inner_spaces("let x = 1", Some("let x = 1"))]
#[case::edge_spaces_kept("  foo ", Some("  foo "))]
#[case::whole_line("fn main() {\n", Some("fn main() {"))]
#[case::crlf_line("fn main() {\r\n", Some("fn main() {"))]
#[case::empty("", None)]
#[case::blank("   ", None)]
#[case::only_break("\n", None)]
#[case::two_lines("one\ntwo", Some("one\ntwo"))]
#[case::two_whole_lines("one\ntwo\n", Some("one\ntwo"))]
#[case::blank_lines("\n\n", None)]
fn the_selection_seeds_the_query(#[case] selected: &str, #[case] expected: Option<&str>) {
    assert_eq!(seed_query(selected), expected);
}

fn rows() -> Vec<LineKind> {
    vec![
        LineKind::Context { old: 1, new: 1 },
        LineKind::Removed { old: 2 },
        LineKind::Added { new: 2 },
        LineKind::Context { old: 3, new: 3 },
        LineKind::Added { new: 4 },
        LineKind::Context { old: 4, new: 5 },
    ]
}

#[rstest]
#[case::no_anchor(None, 0)]
#[case::on_a_hit(Some((Side::New, 4)), 1)]
#[case::between_hits(Some((Side::New, 3)), 1)]
#[case::old_side(Some((Side::Old, 2)), 1)]
#[case::context_by_old_number(Some((Side::Old, 4)), 2)]
#[case::past_the_last_wraps(Some((Side::New, 9)), 0)]
#[case::first_row(Some((Side::Old, 1)), 0)]
fn the_first_hit_is_the_one_at_or_after_the_anchor(
    #[case] anchor: Option<(Side, u32)>,
    #[case] expected: usize,
) {
    let hits = [0, 4, 5];
    assert_eq!(start_hit(&rows(), &hits, anchor), expected);
}

#[test]
fn an_anchor_after_every_hit_starts_at_the_first() {
    assert_eq!(start_hit(&rows(), &[0, 2], Some((Side::New, 5))), 0);
}

fn text(rows: Vec<LineKind>, old: &[&str], new: &[&str]) -> FindText {
    FindText::new(
        String::new(),
        "a.rs".into(),
        rows,
        [Vec::new(), Vec::new()],
        old.iter().map(|line| line.to_string()).collect(),
        new.iter().map(|line| line.to_string()).collect(),
    )
}

fn hit(rows: &[usize], side: Option<Side>) -> Hit {
    Hit {
        rows: rows.to_vec(),
        side,
    }
}

fn struct_text() -> FindText {
    text(
        vec![
            LineKind::Context { old: 1, new: 1 },
            LineKind::Removed { old: 2 },
            LineKind::Added { new: 2 },
            LineKind::Added { new: 3 },
            LineKind::Context { old: 3, new: 4 },
            LineKind::Context { old: 4, new: 5 },
        ],
        &["struct s {", "    old: u8,", "    closed: bool,", "}"],
        &[
            "struct s {",
            "    session: option<string>,",
            "    last: u8,",
            "    closed: bool,",
            "}",
        ],
    )
}

#[test]
fn one_line_queries_hit_every_row_that_contains_them() {
    assert_eq!(
        find_hits(&struct_text(), "closed", false),
        vec![hit(&[4], None)]
    );
}

#[test]
fn consecutive_rows_match_in_the_shown_order() {
    assert_eq!(
        find_hits(
            &struct_text(),
            "session: option<string>,\n    last: u8,\n    closed",
            false
        ),
        vec![hit(&[2, 3, 4], None)]
    );
}

#[test]
fn a_selection_on_the_new_side_skips_the_removed_lines() {
    assert_eq!(
        find_hits(&struct_text(), "struct s {\n    session", false),
        vec![hit(&[0, 2], Some(Side::New))]
    );
}

#[test]
fn a_selection_in_the_unified_order_spans_removed_and_added_lines() {
    assert_eq!(
        find_hits(&struct_text(), "old: u8,\n    session", false),
        vec![hit(&[1, 2], None)]
    );
}

#[test]
fn a_selection_on_the_old_side_skips_the_added_lines() {
    assert_eq!(
        find_hits(&struct_text(), "old: u8,\n    closed: bool,\n}", false),
        vec![hit(&[1, 4, 5], Some(Side::Old))]
    );
}

#[test]
fn context_lines_found_on_every_reading_count_once() {
    assert_eq!(
        find_hits(&struct_text(), "closed: bool,\n}", false),
        vec![hit(&[4, 5], None)]
    );
}

#[test]
fn inner_lines_must_match_whole_and_crlf_is_ignored() {
    assert!(
        find_hits(
            &struct_text(),
            "s {\nsession: option<string>,\n    last",
            false
        )
        .is_empty()
    );
    assert_eq!(
        find_hits(&struct_text(), "last: u8,\r\n    closed", false),
        vec![hit(&[3, 4], None)]
    );
}

#[test]
fn an_empty_query_hits_nothing() {
    assert!(find_hits(&struct_text(), "", false).is_empty());
}

#[test]
fn case_sensitive_queries_compare_the_text_as_written() {
    let text = text(
        vec![
            LineKind::Context { old: 1, new: 1 },
            LineKind::Context { old: 2, new: 2 },
        ],
        &["let Foo = 1;", "let foo = 2;"],
        &["let Foo = 1;", "let foo = 2;"],
    );
    assert_eq!(
        find_hits(&text, "foo", false),
        vec![hit(&[0], None), hit(&[1], None)]
    );
    assert_eq!(find_hits(&text, "Foo", true), vec![hit(&[0], None)]);
    assert_eq!(find_hits(&text, "FOO", true), Vec::new());
    assert_eq!(
        find_hits(&text, "Foo = 1;\nlet foo", true),
        vec![hit(&[0, 1], None)]
    );
}
