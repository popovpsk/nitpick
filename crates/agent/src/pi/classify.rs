use crate::FailureKind;

const NOT_LOGGED_IN: &[&str] = &[
    "no api key",
    "missing api key",
    "invalid api key",
    "incorrect api key",
    "invalid x-api-key",
    "unauthorized",
    "authentication",
    "not logged in",
    "/login",
    "auth login",
    "oauth",
];

const USAGE_LIMIT: &[&str] = &[
    "rate limit",
    "rate_limit",
    "usage limit",
    "quota",
    "too many requests",
];

const CONTEXT_TOO_LONG: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "prompt is too long",
    "input is too long",
    "maximum context",
    "too many tokens",
];

const MODEL_NOT_FOUND: &[&str] = &[
    "model not found",
    "model_not_found",
    "no models match",
    "unknown model",
];

pub fn kind(message: &str) -> FailureKind {
    let text = message.to_lowercase();
    let any = |hints: &[&str]| hints.iter().any(|hint| text.contains(hint));
    if any(NOT_LOGGED_IN) || has_number(&text, "401") {
        FailureKind::NotLoggedIn
    } else if any(USAGE_LIMIT) || has_number(&text, "429") {
        FailureKind::UsageLimit
    } else if any(CONTEXT_TOO_LONG) {
        FailureKind::ContextTooLong
    } else if any(MODEL_NOT_FOUND) || (text.contains("model") && text.contains("does not exist")) {
        FailureKind::ModelNotFound
    } else {
        FailureKind::Other
    }
}

fn has_number(text: &str, number: &str) -> bool {
    text.match_indices(number).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + number.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}
