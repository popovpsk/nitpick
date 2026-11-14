use rstest::rstest;

use crate::fuzzy::{fuzzy_match, highlight_ranges, rank};

fn positions(query: &str, text: &str) -> Option<Vec<usize>> {
    fuzzy_match(query, text).map(|m| m.positions)
}

fn order<'a>(query: &str, items: &[(&'a str, Option<&'a str>)]) -> Vec<usize> {
    rank(query, items.iter().copied())
        .into_iter()
        .map(|r| r.index)
        .collect()
}

#[rstest]
#[case::empty_query("", "main", Some(vec![]))]
#[case::whitespace_query("  ", "main", Some(vec![]))]
#[case::exact("main", "main", Some(vec![0, 1, 2, 3]))]
#[case::case_insensitive("MAIN", "main", Some(vec![0, 1, 2, 3]))]
#[case::path_segment_start("fb", "foo/bar", Some(vec![0, 4]))]
#[case::word_start_over_inner_letter("gp", "gitlab-pipeline", Some(vec![0, 7]))]
#[case::consecutive_run_at_a_boundary_beats_the_first_letter("ab", "a_ab", Some(vec![2, 3]))]
#[case::spaces_in_the_query_are_ignored("fe at", "feat", Some(vec![0, 1, 2, 3]))]
#[case::byte_offsets_of_multibyte_chars("ÄB", "äbc", Some(vec![0, 2]))]
#[case::out_of_order("abc", "acb", None)]
#[case::query_longer_than_text("main", "mai", None)]
#[case::empty_text("x", "", None)]
fn fuzzy_match_positions(
    #[case] query: &str,
    #[case] text: &str,
    #[case] expected: Option<Vec<usize>>,
) {
    assert_eq!(positions(query, text), expected);
}

#[rstest]
#[case::consecutive_beats_scattered("feat", &[("fix/eat-tea", None), ("feature/x", None)], vec![1, 0])]
#[case::camel_case_hump("fb", &[("fabric", None), ("fooBar", None)], vec![1, 0])]
#[case::path_segment_start("sc", &[("misc", None), ("src/components", None)], vec![1, 0])]
#[case::name_beats_path(
    "api",
    &[("gateway", Some("~/api/gateway")), ("api", Some("~/work/api"))],
    vec![1, 0]
)]
#[case::ties_prefer_shorter_then_given_order(
    "main",
    &[("maintenance", None), ("main", None), ("main", None)],
    vec![1, 2, 0]
)]
#[case::empty_query_keeps_everything_in_order("", &[("b", None), ("a", None), ("c", None)], vec![0, 1, 2])]
#[case::non_matching_are_dropped("zz", &[("main", None), ("dev", Some("~/dev"))], vec![])]
fn rank_order(
    #[case] query: &str,
    #[case] items: &[(&str, Option<&str>)],
    #[case] expected: Vec<usize>,
) {
    assert_eq!(order(query, items), expected);
}

#[test]
fn rank_reports_where_the_query_matched() {
    let ranked = rank(
        "api",
        [
            ("gateway", Some("~/api/gateway")),
            ("api", Some("~/work/api")),
        ],
    );
    assert_eq!(ranked[0].label, vec![0, 1, 2]);
    assert!(ranked[0].detail.is_empty());
    assert!(ranked[1].label.is_empty());
    assert_eq!(ranked[1].detail, vec![2, 3, 4]);
    assert!(ranked[0].score > ranked[1].score);
}

#[rstest]
#[case::merges_runs("abcdefg", &[0, 1, 2, 5], vec![0..3, 5..6])]
#[case::multibyte("äbc", &[0, 2], vec![0..3])]
#[case::out_of_bounds_is_skipped("ab", &[5], vec![])]
fn highlight_ranges_cover_whole_chars(
    #[case] text: &str,
    #[case] positions: &[usize],
    #[case] expected: Vec<std::ops::Range<usize>>,
) {
    assert_eq!(highlight_ranges(text, positions), expected);
}
