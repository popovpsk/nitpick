use chrono::FixedOffset;
use rstest::rstest;

use super::note_time;

#[rstest]
#[case::gitlab_utc("2024-05-01T10:20:30.123Z", "2024-05-01 13:20")]
#[case::github_utc("2024-05-01T23:50:00Z", "2024-05-02 02:50")]
#[case::server_offset("2024-05-01T10:20:30+02:00", "2024-05-01 11:20")]
#[case::unparsable("2024-05-01T10:20", "2024-05-01 10:20")]
fn comment_times_show_in_the_local_zone(#[case] created: &str, #[case] shown: &str) {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    assert_eq!(note_time(created, &moscow), shown);
}
