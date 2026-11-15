use serde::{Deserialize, Deserializer};

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct User {
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub username: String,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Project {
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub path_with_namespace: String,
    #[serde(default, deserialize_with = "null_default")]
    pub web_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct DiffRefs {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "RawMergeRequest")]
pub struct MergeRequest {
    pub id: u64,
    pub iid: u64,
    pub project_id: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub source_project_id: u64,
    pub target_project_id: u64,
    pub web_url: String,
    pub author: User,
    pub sha: Option<String>,
    pub diff_refs: Option<DiffRefs>,
    pub draft: bool,
    pub updated_at: Option<String>,
    pub user_notes_count: u32,
    pub reviewers: Vec<User>,
    pub assignees: Vec<User>,
}

#[derive(Deserialize)]
struct RawMergeRequest {
    id: u64,
    iid: u64,
    project_id: u64,
    #[serde(default, deserialize_with = "null_default")]
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    state: String,
    #[serde(default, deserialize_with = "null_default")]
    source_branch: String,
    #[serde(default, deserialize_with = "null_default")]
    target_branch: String,
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default, deserialize_with = "null_default")]
    web_url: String,
    #[serde(default, deserialize_with = "null_default")]
    author: User,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    diff_refs: Option<DiffRefs>,
    #[serde(default)]
    draft: Option<bool>,
    #[serde(default)]
    work_in_progress: Option<bool>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    user_notes_count: u32,
    #[serde(default, deserialize_with = "null_default")]
    reviewers: Vec<User>,
    #[serde(default, deserialize_with = "null_default")]
    assignees: Vec<User>,
}

impl From<RawMergeRequest> for MergeRequest {
    fn from(raw: RawMergeRequest) -> Self {
        MergeRequest {
            id: raw.id,
            iid: raw.iid,
            project_id: raw.project_id,
            title: raw.title,
            description: raw.description,
            state: raw.state,
            source_branch: raw.source_branch,
            target_branch: raw.target_branch,
            source_project_id: raw.source_project_id.unwrap_or(raw.project_id),
            target_project_id: raw.target_project_id.unwrap_or(raw.project_id),
            web_url: raw.web_url,
            author: raw.author,
            sha: raw.sha,
            diff_refs: raw.diff_refs,
            draft: raw.draft.or(raw.work_in_progress).unwrap_or(false),
            updated_at: raw.updated_at,
            user_notes_count: raw.user_notes_count,
            reviewers: raw.reviewers,
            assignees: raw.assignees,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Discussion {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub individual_note: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub notes: Vec<Note>,
}

impl Discussion {
    pub fn is_resolvable(&self) -> bool {
        self.notes.iter().any(|note| note.resolvable)
    }

    pub fn is_resolved(&self) -> bool {
        self.is_resolvable()
            && self
                .notes
                .iter()
                .filter(|note| note.resolvable)
                .all(|note| note.resolved)
    }

    pub fn position(&self) -> Option<&Position> {
        self.notes.first().and_then(|note| note.position.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Note {
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub body: String,
    #[serde(default, deserialize_with = "null_default")]
    pub author: User,
    #[serde(default, deserialize_with = "null_default")]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub system: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub resolvable: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub resolved: bool,
    #[serde(default)]
    pub position: Option<Position>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Position {
    #[serde(default)]
    pub base_sha: Option<String>,
    #[serde(default)]
    pub start_sha: Option<String>,
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub position_type: String,
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
    #[serde(default)]
    pub line_range: Option<LineRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LineRange {
    pub start: LineRangeEnd,
    pub end: LineRangeEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LineRangeEnd {
    #[serde(default)]
    pub line_code: Option<String>,
    #[serde(default, rename = "type")]
    pub line_type: Option<String>,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Approvals {
    #[serde(default, deserialize_with = "null_default")]
    pub approved_by: Vec<Approver>,
    #[serde(default, deserialize_with = "null_default")]
    pub user_has_approved: bool,
    #[serde(default)]
    pub user_can_approve: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Approver {
    #[serde(default, deserialize_with = "null_default")]
    pub user: User,
}
