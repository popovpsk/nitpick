#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    GitLab(#[from] gitlab::Error),
    #[error(transparent)]
    GitHub(#[from] github::Error),
    #[error("a line comment needs an old or a new line")]
    InvalidPosition,
    #[error("comment {0} takes no replies")]
    CannotReply(String),
    #[error("draft {0} is not known")]
    UnknownDraft(String),
    #[error("{0} is not supported here")]
    Unsupported(&'static str),
}
