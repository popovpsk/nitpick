use serde::{Deserialize, Serialize};

pub const GITHUB_DEFAULT_URL: &str = "https://github.com";
const GITHUB_COM_HOSTS: [&str; 3] = ["github.com", "www.github.com", "api.github.com"];
const GITHUB_COMMENT_CONTEXT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ForgeKind {
    #[default]
    #[serde(rename = "gitlab")]
    GitLab,
    #[serde(rename = "github")]
    GitHub,
}

impl ForgeKind {
    pub fn is_gitlab(&self) -> bool {
        *self == ForgeKind::GitLab
    }

    pub fn name(self) -> &'static str {
        match self {
            ForgeKind::GitLab => "GitLab",
            ForgeKind::GitHub => "GitHub",
        }
    }

    pub fn reference(self, number: u64) -> String {
        match self {
            ForgeKind::GitLab => format!("!{number}"),
            ForgeKind::GitHub => format!("#{number}"),
        }
    }

    pub fn refspecs(self, number: u64, target_tip: Option<&str>) -> Vec<String> {
        match self {
            ForgeKind::GitLab => vec![format!(
                "+refs/merge-requests/{number}/head:refs/nitpick/mr/{number}"
            )],
            ForgeKind::GitHub => {
                let mut specs = vec![format!(
                    "+refs/pull/{number}/head:refs/nitpick/pr/{number}/head"
                )];
                if let Some(tip) = target_tip.filter(|tip| !tip.is_empty()) {
                    specs.push(format!("+{tip}:refs/nitpick/pr/{number}/base"));
                }
                specs
            }
        }
    }

    pub fn comment_context(self) -> Option<u32> {
        match self {
            ForgeKind::GitLab => None,
            ForgeKind::GitHub => Some(GITHUB_COMMENT_CONTEXT),
        }
    }

    pub fn web_url(self, configured: &str) -> String {
        let url = configured.trim().trim_end_matches('/');
        match self {
            ForgeKind::GitLab => url.to_string(),
            ForgeKind::GitHub => {
                let host = url
                    .split_once("://")
                    .map_or(url, |(_, rest)| rest)
                    .split(['/', '?', '#'])
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if url.is_empty() || GITHUB_COM_HOSTS.contains(&host.as_str()) {
                    GITHUB_DEFAULT_URL.to_string()
                } else {
                    url.to_string()
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct User {
    pub username: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestState {
    #[default]
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffRefs {
    pub base_sha: Option<String>,
    pub start_sha: String,
    pub head_sha: String,
}

impl DiffRefs {
    pub fn needed_commits(&self) -> [&str; 2] {
        [
            self.head_sha.as_str(),
            self.base_sha.as_deref().unwrap_or(&self.start_sha),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: ForgeKind,
    pub number: u64,
    pub title: String,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub state: RequestState,
    pub draft: bool,
    pub diff_refs: Option<DiffRefs>,
}

impl Request {
    pub fn reference(&self) -> String {
        self.kind.reference(self.number)
    }

    pub fn refspecs(&self) -> Vec<String> {
        let tip = self.diff_refs.as_ref().map(|refs| refs.start_sha.as_str());
        self.kind.refspecs(self.number, tip)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: User,
    pub created_at: String,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Position {
    pub base_sha: Option<String>,
    pub start_sha: Option<String>,
    pub head_sha: Option<String>,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
    pub position: Option<Position>,
    pub can_reply: bool,
    pub resolved: bool,
    pub resolve_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NewPosition {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
    pub old_path: String,
    pub new_path: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}
