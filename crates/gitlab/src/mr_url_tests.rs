use rstest::rstest;

use crate::{MrUrl, parse_mr_reference, parse_mr_url, parse_mr_url_with_base};

fn mr(base_url: &str, project_path: &str, iid: u64) -> Option<MrUrl> {
    Some(MrUrl {
        base_url: base_url.to_string(),
        project_path: project_path.to_string(),
        iid,
    })
}

#[rstest]
#[case::plain(
    "https://gitlab.example.com/group/sub/project/-/merge_requests/123",
    mr("https://gitlab.example.com", "group/sub/project", 123)
)]
#[case::diffs_tab(
    "https://gitlab.example.com/group/project/-/merge_requests/7/diffs",
    mr("https://gitlab.example.com", "group/project", 7)
)]
#[case::note_anchor(
    "https://gitlab.example.com/group/project/-/merge_requests/7#note_1",
    mr("https://gitlab.example.com", "group/project", 7)
)]
#[case::query(
    "https://gitlab.example.com/group/project/-/merge_requests/7/diffs?commit_id=abc&view=parallel",
    mr("https://gitlab.example.com", "group/project", 7)
)]
#[case::trailing_slash(
    "https://gitlab.example.com/group/project/-/merge_requests/7/",
    mr("https://gitlab.example.com", "group/project", 7)
)]
#[case::port_and_http(
    "http://10.0.0.5:8080/team/app/-/merge_requests/42",
    mr("http://10.0.0.5:8080", "team/app", 42)
)]
#[case::surrounding_space(
    "  https://gitlab.com/a/b/-/merge_requests/1  ",
    mr("https://gitlab.com", "a/b", 1)
)]
#[case::relative_root_becomes_group(
    "https://host/gitlab/group/p/-/merge_requests/5",
    mr("https://host", "gitlab/group/p", 5)
)]
#[case::issue_not_mr("https://gitlab.com/a/b/-/issues/1", None)]
#[case::no_project("https://gitlab.com/-/merge_requests/1", None)]
#[case::no_iid("https://gitlab.com/a/b/-/merge_requests", None)]
#[case::bad_iid("https://gitlab.com/a/b/-/merge_requests/new", None)]
#[case::zero_iid("https://gitlab.com/a/b/-/merge_requests/0", None)]
#[case::not_web("ssh://git@gitlab.com/a/b/-/merge_requests/1", None)]
#[case::not_url("!123", None)]
#[case::empty("", None)]
fn parses_mr_urls(#[case] url: &str, #[case] expected: Option<MrUrl>) {
    assert_eq!(parse_mr_url(url), expected);
}

#[rstest]
#[case::relative_root(
    "https://host/gitlab/group/p/-/merge_requests/5",
    "https://host/gitlab",
    mr("https://host/gitlab", "group/p", 5)
)]
#[case::base_with_trailing_slash(
    "https://host/gitlab/group/sub/p/-/merge_requests/5/diffs#note_9",
    "https://host/gitlab/",
    mr("https://host/gitlab", "group/sub/p", 5)
)]
#[case::root_install(
    "https://gitlab.example.com/group/p/-/merge_requests/5",
    "https://gitlab.example.com",
    mr("https://gitlab.example.com", "group/p", 5)
)]
#[case::other_host_falls_back(
    "https://other.example.com/gitlab/p/-/merge_requests/5",
    "https://host/gitlab",
    mr("https://other.example.com", "gitlab/p", 5)
)]
#[case::other_root_falls_back(
    "https://host/code/g/p/-/merge_requests/5",
    "https://host/gitlab",
    mr("https://host", "code/g/p", 5)
)]
fn parses_mr_urls_with_known_base(
    #[case] url: &str,
    #[case] base_url: &str,
    #[case] expected: Option<MrUrl>,
) {
    assert_eq!(parse_mr_url_with_base(url, base_url), expected);
}

#[rstest]
#[case::bang("!123", Some(123))]
#[case::bare("123", Some(123))]
#[case::spaces(" 123 ", Some(123))]
#[case::bang_spaces("  !7\n", Some(7))]
#[case::zero("!0", None)]
#[case::empty("", None)]
#[case::bang_only("!", None)]
#[case::plus("+5", None)]
#[case::negative("-5", None)]
#[case::double_bang("!!5", None)]
#[case::issue_ref("#5", None)]
#[case::text("abc", None)]
#[case::space_inside("! 5", None)]
#[case::overflow("99999999999999999999999", None)]
fn parses_mr_references(#[case] text: &str, #[case] expected: Option<u64>) {
    assert_eq!(parse_mr_reference(text), expected);
}
