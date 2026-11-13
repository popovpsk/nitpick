use serde_json::Value;

const MAX_BODY_CHARS: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid GitLab URL: {0}")]
    InvalidUrl(String),
    #[error("the GitLab access token is empty or contains characters not allowed in a header")]
    InvalidToken,
    #[error("could not set up TLS: {0}")]
    Tls(String),
    #[error("GitLab request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GitLab answered {status}: {message}")]
    Status { status: u16, message: String },
    #[error("GitLab rejected the access token")]
    Unauthorized,
    #[error("not found on GitLab: {0}")]
    NotFound(String),
    #[error("unexpected response from GitLab: {0}")]
    Decode(String),
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
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) {
        for key in ["message", "error"] {
            match map.get(key) {
                Some(Value::String(text)) => return text.clone(),
                Some(Value::Null) | None => {}
                Some(other) => return other.to_string(),
            }
        }
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

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}
