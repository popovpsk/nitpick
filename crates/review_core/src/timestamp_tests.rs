use rstest::rstest;

use crate::parse_timestamp;

#[rstest]
#[case::epoch("1970-01-01T00:00:00Z", Some(0))]
#[case::gitlab_millis("2024-05-01T10:20:30.123Z", Some(1_714_558_830))]
#[case::github("2024-05-01T10:20:30Z", Some(1_714_558_830))]
#[case::offset("2024-05-01T13:20:30+03:00", Some(1_714_558_830))]
#[case::negative_offset("2024-05-01T05:20:30-05:00", Some(1_714_558_830))]
#[case::leap_day("2024-02-29T00:00:00Z", Some(1_709_164_800))]
#[case::date_only("2024-05-01", None)]
#[case::bad_month("2024-13-01T00:00:00Z", None)]
#[case::garbage("yesterday", None)]
fn timestamps_parse_to_unix_seconds(#[case] text: &str, #[case] expected: Option<i64>) {
    assert_eq!(parse_timestamp(text), expected);
}
