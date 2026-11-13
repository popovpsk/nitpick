use rstest::rstest;
use url::Url;

use super::{builder, redirect_allowed, same_host_redirects};

#[test]
fn client_builds_with_the_shared_tls_setup() {
    let client = builder("sloptaster-test").unwrap().https_only(true).build();
    assert!(client.is_ok(), "{client:?}");
}

#[test]
fn client_builds_with_the_same_host_policy() {
    let origin = Url::parse("https://gitlab.example.com").unwrap();
    let client = builder("sloptaster-test")
        .unwrap()
        .redirect(same_host_redirects(&origin))
        .build();
    assert!(client.is_ok(), "{client:?}");
}

#[rstest]
#[case::same_host("https://gitlab.example.com", "https://gitlab.example.com/a", true)]
#[case::other_port(
    "https://gitlab.example.com",
    "https://gitlab.example.com:8443/a",
    true
)]
#[case::https_upgrade("http://gitlab.example.com", "https://gitlab.example.com/a", true)]
#[case::plain_http("http://gitlab.example.com", "http://gitlab.example.com/a", true)]
#[case::https_downgrade("https://gitlab.example.com", "http://gitlab.example.com/a", false)]
#[case::other_host("https://gitlab.example.com", "https://evil.example.com/a", false)]
#[case::subdomain("https://example.com", "https://gitlab.example.com/a", false)]
#[case::other_scheme("https://gitlab.example.com", "ftp://gitlab.example.com/a", false)]
fn redirect_rule(#[case] origin: &str, #[case] target: &str, #[case] allowed: bool) {
    let origin = Url::parse(origin).unwrap();
    let target = Url::parse(target).unwrap();
    assert_eq!(redirect_allowed(&origin, &target), allowed);
}
