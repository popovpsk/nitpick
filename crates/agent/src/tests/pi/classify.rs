use rstest::rstest;

use crate::FailureKind;
use crate::pi::classify::kind;

#[rstest]
#[case::missing_key("No API key found for anthropic", FailureKind::NotLoggedIn)]
#[case::status_401("Request failed with status 401", FailureKind::NotLoggedIn)]
#[case::unauthorized("401 Unauthorized: invalid x-api-key", FailureKind::NotLoggedIn)]
#[case::oauth("OAuth token refresh failed", FailureKind::NotLoggedIn)]
#[case::login_hint("Run /login to authenticate", FailureKind::NotLoggedIn)]
#[case::status_429("429 Too Many Requests", FailureKind::UsageLimit)]
#[case::rate_limit("Rate limit reached for requests", FailureKind::UsageLimit)]
#[case::quota("You exceeded your current quota", FailureKind::UsageLimit)]
#[case::usage_limit("Usage limit reached, resets at 5pm", FailureKind::UsageLimit)]
#[case::prompt_too_long(
    "prompt is too long: 250000 tokens > 200000 maximum",
    FailureKind::ContextTooLong
)]
#[case::context_length(
    "This model's maximum context length is 128000 tokens",
    FailureKind::ContextTooLong
)]
#[case::context_code("context_length_exceeded", FailureKind::ContextTooLong)]
#[case::model_not_found("Model not found: gpt-9", FailureKind::ModelNotFound)]
#[case::no_models("No models match pattern \"sonet\"", FailureKind::ModelNotFound)]
#[case::does_not_exist(
    "The model `foo` does not exist or you do not have access to it",
    FailureKind::ModelNotFound
)]
#[case::network("connection reset by peer", FailureKind::Other)]
#[case::number_inside_a_word("request a4012b failed", FailureKind::Other)]
#[case::number_inside_a_number("took 14290 ms", FailureKind::Other)]
#[case::empty("", FailureKind::Other)]
fn the_failure_kind_follows_the_error_text(#[case] message: &str, #[case] expected: FailureKind) {
    assert_eq!(kind(message), expected);
}
