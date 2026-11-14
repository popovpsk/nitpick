use rstest::rstest;

use crate::{PrUrl, parse_pr_reference, parse_pr_url, parse_pr_url_with_base};

fn pr(base_url: &str, repo: &str, number: u64) -> Option<PrUrl> {
    Some(PrUrl {
        base_url: base_url.into(),
        repo: repo.into(),
        number,
    })
}

#[rstest]
#[case::plain(
    "https://github.com/o/r/pull/123",
    pr("https://github.com", "o/r", 123)
)]
#[case::files_tab(
    "https://github.com/o/r/pull/123/files#diff-1",
    pr("https://github.com", "o/r", 123)
)]
#[case::query(
    "https://github.com/o/r/pull/7?w=1",
    pr("https://github.com", "o/r", 7)
)]
#[case::enterprise(
    "https://ghe.corp.example/team/app/pull/9",
    pr("https://ghe.corp.example", "team/app", 9)
)]
#[case::port(
    "http://ghe.local:8080/a/b/pull/1",
    pr("http://ghe.local:8080", "a/b", 1)
)]
#[case::issue("https://github.com/o/r/issues/123", None)]
#[case::pulls_list("https://github.com/o/r/pulls", None)]
#[case::zero("https://github.com/o/r/pull/0", None)]
#[case::not_a_number("https://github.com/o/r/pull/abc", None)]
#[case::gitlab("https://gitlab.com/g/p/-/merge_requests/5", None)]
#[case::not_a_url("#123", None)]
#[case::ftp("ftp://github.com/o/r/pull/1", None)]
fn parses_pull_request_urls(#[case] url: &str, #[case] expected: Option<PrUrl>) {
    assert_eq!(parse_pr_url(url), expected);
}

#[rstest]
#[case::under_base(
    "https://ghe.example/root/o/r/pull/4",
    "https://ghe.example/root",
    pr("https://ghe.example/root", "o/r", 4)
)]
#[case::other_host_falls_back(
    "https://github.com/o/r/pull/4",
    "https://ghe.example",
    pr("https://github.com", "o/r", 4)
)]
#[case::same_host(
    "https://github.com/o/r/pull/4",
    "https://github.com/",
    pr("https://github.com", "o/r", 4)
)]
fn parses_urls_against_a_known_base(
    #[case] url: &str,
    #[case] base: &str,
    #[case] expected: Option<PrUrl>,
) {
    assert_eq!(parse_pr_url_with_base(url, base), expected);
}

#[rstest]
#[case::hash("#123", Some(123))]
#[case::bare("42", Some(42))]
#[case::spaces("  #7 ", Some(7))]
#[case::bang("!5", None)]
#[case::zero("#0", None)]
#[case::empty("#", None)]
#[case::letters("#12a", None)]
fn parses_references(#[case] text: &str, #[case] expected: Option<u64>) {
    assert_eq!(parse_pr_reference(text), expected);
}
