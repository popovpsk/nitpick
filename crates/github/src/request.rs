use std::fmt;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::error::Error;

const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode_segment(text: &str) -> String {
    utf8_percent_encode(text, PATH_SEGMENT).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn parse(path: &str) -> Result<RepoRef, Error> {
        let trimmed = path.trim().trim_matches('/');
        let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
        match trimmed.split_once('/') {
            Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => {
                Ok(RepoRef {
                    owner: owner.to_string(),
                    name: name.to_string(),
                })
            }
            _ => Err(Error::InvalidRepo(path.to_string())),
        }
    }

    pub(crate) fn api_path(&self) -> String {
        format!(
            "/repos/{}/{}",
            encode_segment(&self.owner),
            encode_segment(&self.name)
        )
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NewReviewComment {
    pub body: String,
    pub commit_id: String,
    pub path: String,
    pub line: u32,
    pub side: Side,
}

#[derive(Serialize)]
pub(crate) struct BodyOnly<'a> {
    pub body: &'a str,
}

#[derive(Serialize)]
pub(crate) struct ReviewEvent<'a> {
    pub event: &'a str,
}

#[derive(Serialize)]
pub(crate) struct GraphQlRequest<'a, V: Serialize> {
    pub query: &'a str,
    pub variables: V,
}
