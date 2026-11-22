use rstest::rstest;
use serde_json::{Value, json};

use crate::FailureKind;
use crate::codex::classify::{
    MINIMUM_VERSION, ResumeError, Version, check_version, classify, parse_version, resume_error,
    version_from_user_agent,
};

#[rstest]
#[case::unauthorized(Some(json!("unauthorized")), "", FailureKind::NotLoggedIn)]
#[case::unauthorized_snake(Some(json!("Unauthorized")), "", FailureKind::NotLoggedIn)]
#[case::http_401(Some(json!({"httpConnectionFailed": {"httpStatusCode": 401}})), "", FailureKind::NotLoggedIn)]
#[case::stream_401(Some(json!({"responseStreamConnectionFailed": {"httpStatusCode": 401}})), "", FailureKind::NotLoggedIn)]
#[case::text_login(None, "Not logged in. Run `codex login`.", FailureKind::NotLoggedIn)]
#[case::usage(Some(json!("usageLimitExceeded")), "", FailureKind::UsageLimit)]
#[case::usage_snake(Some(json!("usage_limit_exceeded")), "", FailureKind::UsageLimit)]
#[case::rate(Some(json!("rateLimitExceeded")), "", FailureKind::UsageLimit)]
#[case::http_429(Some(json!({"httpConnectionFailed": {"httpStatusCode": 429}})), "", FailureKind::UsageLimit)]
#[case::text_usage(None, "You've hit your usage limit.", FailureKind::UsageLimit)]
#[case::context(Some(json!("contextWindowExceeded")), "", FailureKind::ContextTooLong)]
#[case::context_text(
    None,
    "error code: context_length_exceeded",
    FailureKind::ContextTooLong
)]
#[case::model_code(None, "error code: model_not_found", FailureKind::ModelNotFound)]
#[case::model_text(
    None,
    "The model `gpt-9` does not exist or you do not have access",
    FailureKind::ModelNotFound
)]
#[case::bad_request(Some(json!("badRequest")), "bad", FailureKind::Other)]
#[case::server_error(Some(json!({"httpConnectionFailed": {"httpStatusCode": 503}})), "", FailureKind::Other)]
#[case::empty_object(Some(json!({})), "", FailureKind::Other)]
#[case::number(Some(json!(7)), "", FailureKind::Other)]
#[case::plain(None, "something broke", FailureKind::Other)]
#[case::file_does_not_exist(None, "the file does not exist", FailureKind::Other)]
fn errors_are_classified_by_code_first_and_text_as_a_hint(
    #[case] code: Option<Value>,
    #[case] message: &str,
    #[case] kind: FailureKind,
) {
    assert_eq!(classify(code.as_ref(), message), kind);
}

#[rstest]
#[case::no_rollout("no rollout found for thread id 01a00fe7", ResumeError::Lost)]
#[case::thread("Thread not found: abc", ResumeError::Lost)]
#[case::session("session not found", ResumeError::Lost)]
#[case::conversation("conversation not found", ResumeError::Lost)]
#[case::invalid_session("invalid session id `x`", ResumeError::Lost)]
#[case::invalid_thread("invalid thread id: x", ResumeError::Lost)]
#[case::empty("failed to read thread: rollout is empty", ResumeError::Lost)]
#[case::not_utf8(
    "failed to read thread: stream did not contain valid UTF-8",
    ResumeError::Lost
)]
#[case::unparseable(
    "failed to read thread: failed to parse rollout line 3",
    ResumeError::Lost
)]
#[case::archived("thread 01a0 is archived", ResumeError::Archived)]
#[case::permission(
    "failed to read thread: Permission denied (os error 13)",
    ResumeError::Other
)]
#[case::locked("failed to read thread: file is locked", ResumeError::Other)]
#[case::busy("failed to read thread: device busy", ResumeError::Other)]
#[case::too_many("failed to read thread: too many open files", ResumeError::Other)]
#[case::timed_out("failed to read thread: timed out", ResumeError::Other)]
#[case::unknown_read("failed to read thread: something odd", ResumeError::Other)]
#[case::config("failed to load configuration: invalid value", ResumeError::Other)]
#[case::bare_not_found("entity not found", ResumeError::Other)]
#[case::missing_binary("helper binary not found in PATH", ResumeError::Other)]
#[case::provider("modelProviderMismatch", ResumeError::Other)]
fn resume_errors_separate_a_gone_thread_from_ordinary_failures(
    #[case] message: &str,
    #[case] expected: ResumeError,
) {
    assert_eq!(resume_error(message), expected);
}

#[rstest]
#[case::plain("0.171.0", Some(Version(0, 171, 0)))]
#[case::v("v1.2.3", Some(Version(1, 2, 3)))]
#[case::alpha("0.146.0-alpha.2", Some(Version(0, 146, 0)))]
#[case::too_short("0.146", None)]
#[case::too_long("1.2.3.4", None)]
#[case::words("abc", None)]
#[case::empty("", None)]
fn versions_parse_ignoring_a_pre_release_suffix(
    #[case] text: &str,
    #[case] expected: Option<Version>,
) {
    assert_eq!(parse_version(text), expected);
}

#[rstest]
#[case::live(
    "zeron-probe/0.146.1 (Ubuntu 26.4.0; x86_64) unknown (zeron-probe; 0.0.1)",
    Some(Version(0, 146, 1))
)]
#[case::ours("nitpick/0.171.0 (Mac OS 15.1.0; arm64)", Some(Version(0, 171, 0)))]
#[case::no_slash("codex 0.171.0", None)]
#[case::no_version("nitpick/dev (x)", None)]
fn the_server_version_is_read_from_the_user_agent(
    #[case] user_agent: &str,
    #[case] expected: Option<Version>,
) {
    assert_eq!(version_from_user_agent(user_agent), expected);
}

#[test]
fn only_a_known_older_version_is_refused() {
    assert_eq!(MINIMUM_VERSION, Version(0, 146, 0));
    assert!(check_version(None).is_ok());
    assert!(check_version(Some(MINIMUM_VERSION)).is_ok());
    assert!(check_version(Some(Version(0, 171, 0))).is_ok());
    let message = check_version(Some(Version(0, 120, 3))).unwrap_err();
    assert!(
        message.contains("0.120.3") && message.contains("0.146.0"),
        "{message}"
    );
}
