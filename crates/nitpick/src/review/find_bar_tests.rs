use review_core::{LineKind, Side};
use rstest::rstest;

use super::{seed_query, start_hit};

#[rstest]
#[case::word("parse", Some("parse"))]
#[case::inner_spaces("let x = 1", Some("let x = 1"))]
#[case::edge_spaces_kept("  foo ", Some("  foo "))]
#[case::whole_line("fn main() {\n", Some("fn main() {"))]
#[case::crlf_line("fn main() {\r\n", Some("fn main() {"))]
#[case::empty("", None)]
#[case::blank("   ", None)]
#[case::only_break("\n", None)]
#[case::two_lines("one\ntwo", None)]
#[case::two_whole_lines("one\ntwo\n", None)]
fn the_selection_seeds_a_one_line_query(#[case] selected: &str, #[case] expected: Option<&str>) {
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
