use chrono::FixedOffset;
use rstest::rstest;

use super::{nearest_unviewed, note_time};

#[rstest]
#[case::gitlab_utc("2024-05-01T10:20:30.123Z", "2024-05-01 13:20")]
#[case::github_utc("2024-05-01T23:50:00Z", "2024-05-02 02:50")]
#[case::server_offset("2024-05-01T10:20:30+02:00", "2024-05-01 11:20")]
#[case::unparsable("2024-05-01T10:20", "2024-05-01 10:20")]
fn comment_times_show_in_the_local_zone(#[case] created: &str, #[case] shown: &str) {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    assert_eq!(note_time(created, &moscow), shown);
}

#[rstest]
#[case::below_first(&[false, true, false, false, false], Some(4))]
#[case::nearest_above(&[false, true, false, true, true], Some(2))]
#[case::none_left(&[true, true, true, true, true], None)]
fn advancing_prefers_below_then_the_nearest_above(
    #[case] viewed: &[bool],
    #[case] expected: Option<usize>,
) {
    let order = [0, 1, 2, 3, 4];
    assert_eq!(nearest_unviewed(&order[4..], &order[..3], viewed), expected);
}
