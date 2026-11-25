use std::fmt;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;

const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn encode_segment(text: &str) -> String {
    utf8_percent_encode(text, PATH_SEGMENT).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProjectRef {
    Id(u64),
    Path(String),
}

impl ProjectRef {
    pub(crate) fn api_segment(&self) -> String {
        match self {
            ProjectRef::Id(id) => id.to_string(),
            ProjectRef::Path(path) => encode_segment(path.trim_matches('/')),
        }
    }
}

impl fmt::Display for ProjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectRef::Id(id) => write!(f, "{id}"),
            ProjectRef::Path(path) => f.write_str(path),
        }
    }
}

impl From<u64> for ProjectRef {
    fn from(id: u64) -> Self {
        ProjectRef::Id(id)
    }
}

impl From<&str> for ProjectRef {
    fn from(path: &str) -> Self {
        ProjectRef::Path(path.to_string())
    }
}

impl From<String> for ProjectRef {
    fn from(path: String) -> Self {
        ProjectRef::Path(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MrState {
    #[default]
    Opened,
    Merged,
    Closed,
    All,
}

impl MrState {
    pub fn as_str(self) -> &'static str {
        match self {
            MrState::Opened => "opened",
            MrState::Merged => "merged",
            MrState::Closed => "closed",
            MrState::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrQuery {
    pub state: MrState,
    pub search: Option<String>,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub per_page: u32,
}

impl Default for MrQuery {
    fn default() -> Self {
        MrQuery {
            state: MrState::Opened,
            search: None,
            source_branch: None,
            target_branch: None,
            per_page: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NewPosition {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
    pub old_path: String,
    pub new_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

#[derive(Serialize)]
pub(crate) struct NewDiscussionBody<'a> {
    pub body: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<WirePosition<'a>>,
}

#[derive(Serialize)]
pub(crate) struct WirePosition<'a> {
    position_type: &'static str,
    #[serde(flatten)]
    position: &'a NewPosition,
}

impl<'a> From<&'a NewPosition> for WirePosition<'a> {
    fn from(position: &'a NewPosition) -> Self {
        WirePosition {
            position_type: "text",
            position,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct NoteBody<'a> {
    pub body: &'a str,
}

#[derive(Serialize)]
pub(crate) struct NewDraftBody<'a> {
    pub note: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<WirePosition<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to_discussion_id: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolve_discussion: bool,
}

#[derive(Serialize)]
pub(crate) struct DraftEditBody<'a> {
    pub note: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<WirePosition<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftTarget<'a> {
    General,
    Line(&'a NewPosition),
    Reply {
        discussion_id: &'a str,
        resolve: bool,
    },
}
