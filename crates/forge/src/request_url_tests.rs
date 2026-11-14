use rstest::rstest;

use crate::{ForgeKind, RequestUrl, parse_reference, parse_request_url};

const GITLAB: &str = "https://gitlab.example.com";
const GITHUB: &str = "https://github.com";

fn url(kind: ForgeKind, project: &str, number: u64) -> Option<RequestUrl> {
    Some(RequestUrl {
        kind,
        project: project.into(),
        number,
    })
}

#[rstest]
#[case::gitlab(
    "https://gitlab.example.com/group/sub/app/-/merge_requests/12",
    url(ForgeKind::GitLab, "group/sub/app", 12)
)]
#[case::gitlab_other_host(
    "https://gitlab.com/g/p/-/merge_requests/3/diffs",
    url(ForgeKind::GitLab, "g/p", 3)
)]
#[case::github("https://github.com/o/r/pull/123", url(ForgeKind::GitHub, "o/r", 123))]
#[case::github_files(
    "https://github.com/o/r/pull/123/files",
    url(ForgeKind::GitHub, "o/r", 123)
)]
#[case::github_enterprise(
    "https://ghe.corp.example/team/app/pull/7",
    url(ForgeKind::GitHub, "team/app", 7)
)]
#[case::github_issue("https://github.com/o/r/issues/123", None)]
#[case::reference("!12", None)]
#[case::garbage("hello", None)]
fn request_urls_name_their_forge(#[case] text: &str, #[case] expected: Option<RequestUrl>) {
    assert_eq!(parse_request_url(text, GITLAB, GITHUB), expected);
}

#[test]
fn an_unset_gitlab_server_still_parses_gitlab_urls() {
    assert_eq!(
        parse_request_url("https://gitlab.com/g/p/-/merge_requests/3", "", GITHUB),
        url(ForgeKind::GitLab, "g/p", 3)
    );
}

#[rstest]
#[case::gitlab("!12", Some(12))]
#[case::github("#12", Some(12))]
#[case::bare(" 7 ", Some(7))]
#[case::zero("#0", None)]
#[case::both_marks("!#3", None)]
#[case::text("#abc", None)]
#[case::empty("", None)]
fn references_accept_either_mark(#[case] text: &str, #[case] expected: Option<u64>) {
    assert_eq!(parse_reference(text), expected);
}
