use rstest::rstest;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, duplex};

use crate::lines::Lines;

async fn collect(input: &[u8], limit: usize) -> Vec<String> {
    let mut lines = Lines::with_limit(input, limit);
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await.unwrap() {
        out.push(line);
    }
    out
}

#[tokio::test]
async fn invalid_utf8_crlf_and_an_unterminated_last_line_keep_reading() {
    let bytes: &[u8] = b"one\r\nJ\xf6rg\nthree\xe2\x82\nlast";
    assert_eq!(
        collect(bytes, usize::MAX).await,
        vec!["one", "J\u{fffd}rg", "three\u{fffd}", "last"]
    );
}

#[rstest]
#[case::over_the_limit(b"short\nthis line is far too long\nok\n", &["short", "ok"])]
#[case::exactly_the_limit(b"12345678\nok\n", &["12345678", "ok"])]
#[case::one_over(b"123456789\nok\n", &["ok"])]
#[case::long_at_the_end(b"ok\nthis line is far too long", &["ok"])]
#[case::empty_lines(b"\n\nx\n", &["", "", "x"])]
#[tokio::test]
async fn skips_lines_longer_than_the_limit(#[case] input: &[u8], #[case] expected: &[&str]) {
    assert_eq!(collect(input, 8).await, expected);
}

#[tokio::test]
async fn a_line_torn_between_reads_is_put_back_together() {
    let (mut writer, reader) = duplex(64);
    let producer = tokio::spawn(async move {
        for chunk in [&b"hel"[..], b"lo\nwor", b"ld", b"\n\xe2", b"\x82\xac\n"] {
            writer.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut lines = Lines::new(reader);
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("hello"));
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("world"));
    assert_eq!(
        lines.next_line().await.unwrap().as_deref(),
        Some("\u{20ac}")
    );
    assert_eq!(lines.next_line().await.unwrap(), None);
    producer.await.unwrap();
}

#[tokio::test]
async fn a_cancelled_read_resumes_the_same_line() {
    let (mut writer, reader) = duplex(64);
    let mut lines = Lines::new(reader);
    writer.write_all(b"par").await.unwrap();
    let cancelled = tokio::time::timeout(Duration::from_millis(20), lines.next_line()).await;
    assert!(cancelled.is_err());
    writer.write_all(b"tial\nnext\n").await.unwrap();
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("partial"));
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("next"));
}

#[tokio::test]
async fn a_too_long_line_torn_between_reads_is_skipped() {
    let (mut writer, reader) = duplex(64);
    let producer = tokio::spawn(async move {
        for chunk in [&b"aaaa"[..], b"bbbb", b"cccc", b"\nok\n"] {
            writer.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut lines = Lines::with_limit(reader, 8);
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("ok"));
    assert_eq!(lines.next_line().await.unwrap(), None);
    producer.await.unwrap();
}
