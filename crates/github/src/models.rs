use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use crate::request::Side;

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct User {
    #[serde(default, deserialize_with = "null_default")]
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

impl User {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.login)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct BranchRef {
    #[serde(default, rename = "ref", deserialize_with = "null_default")]
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub sha: String,
    #[serde(default, deserialize_with = "null_default")]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub state: String,
    #[serde(default, deserialize_with = "null_default")]
    pub html_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub user: User,
    #[serde(default, deserialize_with = "null_default")]
    pub head: BranchRef,
    #[serde(default, deserialize_with = "null_default")]
    pub base: BranchRef,
    #[serde(default, deserialize_with = "null_default")]
    pub draft: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub requested_reviewers: Vec<User>,
    #[serde(default, deserialize_with = "null_default")]
    pub assignees: Vec<User>,
}

impl PullRequest {
    pub fn is_merged(&self) -> bool {
        self.merged_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReviewComment {
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub node_id: String,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
    #[serde(default, deserialize_with = "null_default")]
    pub body: String,
    #[serde(default, deserialize_with = "null_default")]
    pub user: User,
    #[serde(default, deserialize_with = "null_default")]
    pub created_at: String,
    #[serde(default, deserialize_with = "null_default")]
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub original_line: Option<u32>,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default, deserialize_with = "null_default")]
    pub commit_id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub original_commit_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub body: String,
    #[serde(default, deserialize_with = "null_default")]
    pub user: User,
    #[serde(default, deserialize_with = "null_default")]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadState {
    pub id: String,
    pub first_comment: u64,
    pub resolved: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThread {
    pub comments: Vec<ReviewComment>,
    pub state: Option<ThreadState>,
}

impl ReviewThread {
    pub fn root(&self) -> &ReviewComment {
        &self.comments[0]
    }
}

pub fn group_threads(comments: Vec<ReviewComment>, states: &[ThreadState]) -> Vec<ReviewThread> {
    let mut order: Vec<u64> = Vec::new();
    let mut threads: HashMap<u64, Vec<ReviewComment>> = HashMap::new();
    for comment in comments {
        let root = comment.in_reply_to_id.unwrap_or(comment.id);
        let list = threads.entry(root).or_default();
        if list.is_empty() {
            order.push(root);
        }
        list.push(comment);
    }
    let by_comment: HashMap<u64, &ThreadState> =
        states.iter().map(|s| (s.first_comment, s)).collect();
    order
        .into_iter()
        .filter_map(|root| {
            let mut comments = threads.remove(&root)?;
            comments.sort_by(|a, b| {
                (a.id != root)
                    .cmp(&(b.id != root))
                    .then_with(|| a.created_at.cmp(&b.created_at))
                    .then(a.id.cmp(&b.id))
            });
            let state = by_comment
                .get(&root)
                .or_else(|| by_comment.get(&comments[0].id))
                .map(|state| (*state).clone());
            Some(ReviewThread { comments, state })
        })
        .collect()
}

#[derive(Deserialize)]
pub(crate) struct GraphQlResponse<T> {
    #[serde(default = "Option::default")]
    pub data: Option<T>,
    #[serde(default, deserialize_with = "null_default")]
    pub errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
pub(crate) struct GraphQlError {
    #[serde(default, deserialize_with = "null_default")]
    pub message: String,
}

#[derive(Deserialize)]
pub(crate) struct ThreadsData {
    pub repository: Option<ThreadsRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadsRepository {
    pub pull_request: Option<ThreadsPull>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadsPull {
    pub review_threads: ThreadConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadConnection {
    pub page_info: PageInfo,
    #[serde(default, deserialize_with = "null_default")]
    pub nodes: Vec<Option<ThreadNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageInfo {
    #[serde(default, deserialize_with = "null_default")]
    pub has_next_page: bool,
    #[serde(default)]
    pub end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadNode {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub is_resolved: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub viewer_can_resolve: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub viewer_can_unresolve: bool,
    pub comments: CommentConnection,
}

#[derive(Deserialize)]
pub(crate) struct CommentConnection {
    #[serde(default, deserialize_with = "null_default")]
    pub nodes: Vec<Option<CommentNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommentNode {
    #[serde(default)]
    pub database_id: Option<u64>,
}

impl ThreadNode {
    pub(crate) fn into_state(self) -> Option<ThreadState> {
        let first_comment = self
            .comments
            .nodes
            .into_iter()
            .flatten()
            .find_map(|c| c.database_id)?;
        Some(ThreadState {
            id: self.id,
            first_comment,
            resolved: self.is_resolved,
            can_resolve: self.viewer_can_resolve,
            can_unresolve: self.viewer_can_unresolve,
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct ResolveData {
    #[serde(default)]
    pub result: Option<ResolvePayload>,
}

#[derive(Deserialize)]
pub(crate) struct ResolvePayload {
    pub thread: Option<ResolvedThread>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedThread {
    #[serde(default, deserialize_with = "null_default")]
    pub is_resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Review {
    #[serde(default, deserialize_with = "null_default")]
    pub user: User,
    #[serde(default, deserialize_with = "null_default")]
    pub state: String,
}
