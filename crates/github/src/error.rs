use serde_json::Value;

const MAX_BODY_CHARS: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid GitHub URL: {0}")]
    InvalidUrl(String),
    #[error("the GitHub access token is empty or contains characters not allowed in a header")]
    InvalidToken,
    #[error("not a GitHub repository path (owner/name): {0}")]
    InvalidRepo(String),
    #[error("could not set up TLS: {0}")]
    Tls(String),
    #[error("GitHub request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GitHub answered {status}: {message}")]
    Status { status: u16, message: String },
    #[error("GitHub rejected the access token")]
    Unauthorized,
    #[error("not found on GitHub: {0}")]
    NotFound(String),
    #[error("unexpected response from GitHub: {0}")]
    Decode(String),
    #[error("GitHub GraphQL error: {0}")]
    GraphQl(String),
}

impl From<http_client::Error> for Error {
    fn from(error: http_client::Error) -> Error {
        match error {
            http_client::Error::Tls(message) => Error::Tls(message),
        }
    }
}

impl Error {
    pub(crate) fn from_status(status: u16, path: &str, body: &str) -> Error {
        match status {
            401 => Error::Unauthorized,
            404 => Error::NotFound(path.to_string()),
            _ => Error::Status {
                status,
                message: status_message(status, body),
            },
        }
    }
}

fn status_message(status: u16, body: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body)
        && let Some(message) = map.get("message").and_then(text_of)
    {
        let details: Vec<String> = map
            .get("errors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|error| match error {
                Value::Object(fields) => fields.get("message").and_then(text_of),
                other => text_of(other),
            })
            .collect();
        return if details.is_empty() {
            message
        } else {
            format!("{message}: {}", details.join("; "))
        };
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|code| code.canonical_reason())
            .unwrap_or("no details")
            .to_string();
    }
    truncate(trimmed, MAX_BODY_CHARS)
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Null | Value::String(_) => None,
        other => Some(other.to_string()),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}
