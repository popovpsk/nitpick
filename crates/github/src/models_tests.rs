use crate::{IssueComment, PullRequest, ReviewComment, Side, ThreadState, group_threads};

pub(crate) const PULL_REQUEST: &str = r#"{
  "url": "https://api.github.com/repos/octocat/Hello-World/pulls/1347",
  "id": 1,
  "node_id": "MDExOlB1bGxSZXF1ZXN0MQ==",
  "html_url": "https://github.com/octocat/Hello-World/pull/1347",
  "number": 1347,
  "state": "open",
  "locked": false,
  "title": "Amazing new feature",
  "user": {"login": "octocat", "id": 1, "type": "User", "site_admin": false},
  "body": "Please pull these awesome changes in!",
  "labels": [],
  "created_at": "2011-01-26T19:01:12Z",
  "updated_at": "2011-01-26T19:01:12Z",
  "closed_at": null,
  "merged_at": null,
  "merge_commit_sha": "e5bd3914e2e596debea16f433f57875b5b90bcd6",
  "draft": false,
  "head": {
    "label": "octocat:new-topic",
    "ref": "new-topic",
    "sha": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
    "user": {"login": "octocat", "id": 1},
    "repo": {"id": 1296269, "full_name": "octocat/Hello-World"}
  },
  "base": {
    "label": "octocat:master",
    "ref": "master",
    "sha": "4ab3f2a4a4d1c1e5a1b8c6c0e2f3b1d2c3a4b5c6",
    "user": {"login": "octocat", "id": 1},
    "repo": {"id": 1296269, "full_name": "octocat/Hello-World"}
  },
  "author_association": "OWNER",
  "merged": false,
  "comments": 10,
  "review_comments": 0,
  "commits": 3,
  "additions": 100,
  "deletions": 3,
  "changed_files": 5
}"#;

pub(crate) const ROOT_COMMENT: &str = r#"{
  "url": "https://api.github.com/repos/octocat/Hello-World/pulls/comments/10",
  "pull_request_review_id": 42,
  "id": 10,
  "node_id": "MDI0OlB1bGxSZXF1ZXN0UmV2aWV3Q29tbWVudDEw",
  "diff_hunk": "@@ -16,33 +16,40 @@ public class Connection : IConnection...",
  "path": "file1.txt",
  "position": 1,
  "original_position": 4,
  "commit_id": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
  "original_commit_id": "9c48853fa3dc5c1c3d6f1f1cd1f2743e72652840",
  "user": {"login": "octocat", "id": 1},
  "body": "Great stuff!",
  "created_at": "2011-04-14T16:00:49Z",
  "updated_at": "2011-04-14T16:00:49Z",
  "html_url": "https://github.com/octocat/Hello-World/pull/1#discussion-diff-1",
  "author_association": "NONE",
  "start_line": null,
  "original_start_line": null,
  "start_side": null,
  "line": 2,
  "original_line": 2,
  "side": "RIGHT",
  "subject_type": "line"
}"#;

pub(crate) const REPLY_COMMENT: &str = r#"{
  "id": 11,
  "node_id": "MDI0OlB1bGxSZXF1ZXN0UmV2aWV3Q29tbWVudDEx",
  "in_reply_to_id": 10,
  "path": "file1.txt",
  "commit_id": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
  "original_commit_id": "9c48853fa3dc5c1c3d6f1f1cd1f2743e72652840",
  "user": {"login": "hubot", "id": 2},
  "body": "Thanks!",
  "created_at": "2011-04-14T17:00:00Z",
  "line": 2,
  "original_line": 2,
  "side": "RIGHT"
}"#;

pub(crate) const OUTDATED_COMMENT: &str = r#"{
  "id": 20,
  "path": "src/old.rs",
  "commit_id": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
  "original_commit_id": "1111111111111111111111111111111111111111",
  "user": null,
  "body": "This went away",
  "created_at": "2011-04-15T10:00:00Z",
  "line": null,
  "original_line": 7,
  "side": "LEFT"
}"#;

pub(crate) const ISSUE_COMMENT: &str = r#"{
  "id": 1,
  "node_id": "MDEyOklzc3VlQ29tbWVudDE=",
  "url": "https://api.github.com/repos/octocat/Hello-World/issues/comments/1",
  "html_url": "https://github.com/octocat/Hello-World/issues/1347#issuecomment-1",
  "body": "Me too",
  "user": {"login": "octocat", "id": 1},
  "created_at": "2011-04-14T16:30:00Z",
  "updated_at": "2011-04-14T16:30:00Z",
  "author_association": "COLLABORATOR"
}"#;

pub(crate) fn json_list(items: &[&str]) -> String {
    format!("[{}]", items.join(","))
}

fn comment(json: &str) -> ReviewComment {
    serde_json::from_str(json).unwrap()
}

#[test]
fn pull_request_parses_branches_and_shas() {
    let pr: PullRequest = serde_json::from_str(PULL_REQUEST).unwrap();
    assert_eq!(pr.number, 1347);
    assert_eq!(pr.title, "Amazing new feature");
    assert_eq!(pr.user.login, "octocat");
    assert_eq!(pr.head.name, "new-topic");
    assert_eq!(pr.head.sha, "6dcb09b5b57875f334f61aebed695e2e4193db5e");
    assert_eq!(pr.base.name, "master");
    assert_eq!(
        pr.html_url,
        "https://github.com/octocat/Hello-World/pull/1347"
    );
    assert!(!pr.draft);
    assert!(!pr.is_merged());
}

#[test]
fn pull_request_tolerates_nulls_and_missing_fields() {
    let pr: PullRequest =
        serde_json::from_str(r#"{"number": 3, "title": null, "user": null, "draft": null}"#)
            .unwrap();
    assert_eq!(pr.number, 3);
    assert!(pr.title.is_empty());
    assert!(pr.user.login.is_empty());
    assert!(pr.head.sha.is_empty());
}

#[test]
fn review_comment_parses_line_side_and_reply_link() {
    let root = comment(ROOT_COMMENT);
    assert_eq!(root.id, 10);
    assert_eq!(root.in_reply_to_id, None);
    assert_eq!(root.line, Some(2));
    assert_eq!(root.side, Some(Side::Right));
    assert_eq!(root.path, "file1.txt");
    let reply = comment(REPLY_COMMENT);
    assert_eq!(reply.in_reply_to_id, Some(10));
    let outdated = comment(OUTDATED_COMMENT);
    assert_eq!(outdated.line, None);
    assert_eq!(outdated.original_line, Some(7));
    assert_eq!(outdated.side, Some(Side::Left));
    assert!(outdated.user.login.is_empty());
}

#[test]
fn issue_comment_parses() {
    let note: IssueComment = serde_json::from_str(ISSUE_COMMENT).unwrap();
    assert_eq!(note.id, 1);
    assert_eq!(note.body, "Me too");
    assert_eq!(note.user.login, "octocat");
}

#[test]
fn comments_group_into_threads_by_their_root() {
    let comments = vec![
        comment(ROOT_COMMENT),
        comment(OUTDATED_COMMENT),
        comment(REPLY_COMMENT),
    ];
    let states = [ThreadState {
        id: "PRRT_1".into(),
        first_comment: 10,
        resolved: true,
        can_resolve: true,
        can_unresolve: true,
    }];
    let threads = group_threads(comments, &states);
    assert_eq!(threads.len(), 2);
    let ids: Vec<Vec<u64>> = threads
        .iter()
        .map(|t| t.comments.iter().map(|c| c.id).collect())
        .collect();
    assert_eq!(ids, vec![vec![10, 11], vec![20]]);
    assert_eq!(threads[0].root().id, 10);
    assert_eq!(
        threads[0].state.as_ref().map(|s| s.id.as_str()),
        Some("PRRT_1")
    );
    assert_eq!(threads[1].state, None);
}

#[test]
fn a_reply_whose_root_is_gone_still_forms_a_thread() {
    let threads = group_threads(vec![comment(REPLY_COMMENT)], &[]);
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].root().id, 11);
}
