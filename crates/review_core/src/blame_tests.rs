use std::collections::HashSet;

use rstest::rstest;

use crate::blame::{initials, marker_lines, tint_index};

#[rstest]
#[case::runs_only(&[1, 1, 2, 2, 2, 1], &[], &[1, 3, 6])]
#[case::block_start_inside_a_run(&[1, 1, 2, 2, 2, 1], &[4], &[1, 3, 4, 6])]
#[case::block_start_on_a_run_start(&[1, 1, 2], &[3], &[1, 3])]
#[case::block_starts_past_the_end(&[7, 7], &[1, 2, 9], &[1, 2])]
#[case::one_commit(&[5, 5, 5, 5], &[], &[1])]
#[case::empty(&[], &[1], &[])]
fn markers_go_on_run_and_block_starts(
    #[case] keys: &[u32],
    #[case] block_starts: &[u32],
    #[case] expected: &[u32],
) {
    assert_eq!(marker_lines(keys, block_starts), expected);
}

#[rstest]
#[case::two_words("Ann Author", "AA")]
#[case::one_word("alice", "A")]
#[case::three_words("Jean-Luc Picard", "JP")]
#[case::email("bob.builder@example.com", "BB")]
#[case::cyrillic("мария иванова", "МИ")]
#[case::cjk("李小龙", "李")]
#[case::punctuation("o'neil  (smith)", "OS")]
#[case::blank("  ", "?")]
#[case::symbols_only("*** ---", "?")]
fn initials_take_the_first_and_last_word(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(initials(name), expected);
}

#[test]
fn tint_index_is_stable_and_in_range() {
    let id = "d79fbcdec908855e755459b1a2622983cdd35100";
    assert_eq!(tint_index(id, 12), tint_index(id, 12));
    assert!(tint_index(id, 12) < 12);
    assert_eq!(tint_index(id, 0), 0);
    let spread: HashSet<usize> = (0..64)
        .map(|n| tint_index(&format!("{n:040x}"), 12))
        .collect();
    assert!(spread.len() > 6);
}
