use rstest::rstest;
use serde_json::json;

use crate::convert::{
    github_comment, github_discussions, github_reply_target, github_request, gitlab_discussion,
    gitlab_request,
};
use crate::{DiffRefs, Error, ForgeKind, NewPosition, Position, RequestState};

fn gitlab_mr(state: &str, diff_refs: bool) -> gitlab::MergeRequest {
    let mut value = json!({
        "id": 1, "iid": 5, "project_id": 3, "title": "Speed up", "state": state,
        "source_branch": "feature", "target_branch": "main",
        "web_url": "https://gitlab.example.com/g/p/-/merge_requests/5",
        "author": {"id": 7, "username": "ada", "name": "Ada Lovelace"},
        "draft": true
    });
    if diff_refs {
        value["diff_refs"] = json!({"base_sha": "b", "start_sha": "s", "head_sha": "h"});
    }
    serde_json::from_value(value).unwrap()
}

fn github_pr(state: &str, merged: bool) -> github::PullRequest {
    serde_json::from_value(json!({
        "number": 9, "title": "Add a thing", "state": state,
        "html_url": "https://github.com/o/r/pull/9",
        "user": {"login": "octocat", "id": 1},
        "head": {"ref": "topic", "sha": "head1"},
        "base": {"ref": "main", "sha": "tip1"},
        "draft": false,
        "merged_at": if merged { json!("2024-01-01T00:00:00Z") } else { json!(null) }
    }))
    .unwrap()
}

fn review_comment(value: serde_json::Value) -> github::ReviewComment {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_gitlab_merge_request_keeps_its_merge_base() {
    let request = gitlab_request(&gitlab_mr("opened", true));
    assert_eq!(request.kind, ForgeKind::GitLab);
    assert_eq!(request.number, 5);
    assert_eq!(request.reference(), "!5");
    assert_eq!(request.author.username, "ada");
    assert_eq!(request.author.name, "Ada Lovelace");
    assert_eq!(
        (
            request.source_branch.as_str(),
            request.target_branch.as_str()
        ),
        ("feature", "main")
    );
    assert!(request.draft);
    assert_eq!(request.state, RequestState::Open);
    let refs = request.diff_refs.unwrap();
    assert_eq!(
        refs,
        DiffRefs {
            base_sha: Some("b".into()),
            start_sha: "s".into(),
            head_sha: "h".into(),
        }
    );
    assert_eq!(refs.needed_commits(), ["h", "b"]);
}

#[rstest]
#[case::merged("merged", RequestState::Merged)]
#[case::closed("closed", RequestState::Closed)]
#[case::locked("locked", RequestState::Open)]
fn gitlab_states_map(#[case] state: &str, #[case] expected: RequestState) {
    assert_eq!(gitlab_request(&gitlab_mr(state, false)).state, expected);
}

#[test]
fn a_gitlab_merge_request_without_diff_refs_has_none() {
    assert_eq!(gitlab_request(&gitlab_mr("opened", false)).diff_refs, None);
}

#[test]
fn a_github_pull_request_leaves_the_merge_base_to_git() {
    let request = github_request(&github_pr("open", false));
    assert_eq!(request.kind, ForgeKind::GitHub);
    assert_eq!(request.reference(), "#9");
    assert_eq!(request.author.username, "octocat");
    assert_eq!(request.author.name, "octocat");
    assert_eq!(request.source_branch, "topic");
    assert_eq!(request.target_branch, "main");
    let refs = request.diff_refs.clone().unwrap();
    assert_eq!(refs.base_sha, None);
    assert_eq!(refs.needed_commits(), ["head1", "tip1"]);
    assert_eq!(
        request.refspecs(),
        vec![
            "+refs/pull/9/head:refs/nitpick/pr/9/head".to_string(),
            "+tip1:refs/nitpick/pr/9/base".to_string(),
        ]
    );
}

#[rstest]
#[case::open("open", false, RequestState::Open)]
#[case::closed("closed", false, RequestState::Closed)]
#[case::merged("closed", true, RequestState::Merged)]
fn github_states_map(#[case] state: &str, #[case] merged: bool, #[case] expected: RequestState) {
    assert_eq!(github_request(&github_pr(state, merged)).state, expected);
}

#[test]
fn gitlab_refspec_is_the_merge_request_head() {
    assert_eq!(
        ForgeKind::GitLab.refspecs(5, Some("s")),
        vec!["+refs/merge-requests/5/head:refs/nitpick/mr/5".to_string()]
    );
}

#[test]
fn a_gitlab_discussion_converts_with_its_position_and_resolve_state() {
    let discussion: gitlab::Discussion = serde_json::from_value(json!({
        "id": "abc",
        "individual_note": false,
        "notes": [
            {"id": 1, "body": "Why?", "author": {"id": 1, "username": "ada", "name": ""},
             "created_at": "2024-01-01T10:00:00Z", "system": false, "resolvable": true,
             "resolved": true,
             "position": {"base_sha": "b", "start_sha": "s", "head_sha": "h",
                          "old_path": "a.rs", "new_path": "a.rs", "old_line": null, "new_line": 4}},
            {"id": 2, "body": "changed the title", "system": true, "resolvable": false}
        ]
    }))
    .unwrap();
    let neutral = gitlab_discussion(&discussion);
    assert_eq!(neutral.id, "abc");
    assert!(neutral.can_reply);
    assert!(neutral.resolved);
    assert_eq!(neutral.resolve_id.as_deref(), Some("abc"));
    assert_eq!(neutral.notes.len(), 2);
    assert_eq!(neutral.notes[0].author.name, "ada");
    assert!(neutral.notes[1].system);
    assert_eq!(
        neutral.position,
        Some(Position {
            base_sha: Some("b".into()),
            start_sha: Some("s".into()),
            head_sha: Some("h".into()),
            old_path: Some("a.rs".into()),
            new_path: Some("a.rs".into()),
            old_line: None,
            new_line: Some(4),
        })
    );
}

#[test]
fn a_gitlab_note_that_is_not_resolvable_has_no_resolve_handle() {
    let discussion: gitlab::Discussion = serde_json::from_value(json!({
        "id": "n1", "individual_note": true,
        "notes": [{"id": 3, "body": "LGTM", "created_at": "2024-01-01T10:00:00Z"}]
    }))
    .unwrap();
    let neutral = gitlab_discussion(&discussion);
    assert_eq!(neutral.resolve_id, None);
    assert!(!neutral.resolved);
    assert_eq!(neutral.position, None);
}

#[test]
fn github_threads_and_conversation_comments_merge_by_time() {
    let root = review_comment(json!({
        "id": 10, "path": "src/a.rs", "line": 3, "side": "RIGHT",
        "commit_id": "head1", "original_commit_id": "old",
        "user": {"login": "octocat"}, "body": "nit", "created_at": "2024-01-02T00:00:00Z"
    }));
    let reply = review_comment(json!({
        "id": 11, "in_reply_to_id": 10, "path": "src/a.rs", "line": 3, "side": "RIGHT",
        "commit_id": "head1", "user": {"login": "hubot", "name": "Hubot"},
        "body": "fixed", "created_at": "2024-01-03T00:00:00Z"
    }));
    let outdated = review_comment(json!({
        "id": 20, "path": "src/gone.rs", "line": null, "original_line": 8, "side": "LEFT",
        "commit_id": "head1", "original_commit_id": "old",
        "user": {"login": "octocat"}, "body": "?", "created_at": "2024-01-04T00:00:00Z"
    }));
    let states = [
        github::ThreadState {
            id: "PRRT_10".into(),
            first_comment: 10,
            resolved: false,
            can_resolve: true,
            can_unresolve: false,
        },
        github::ThreadState {
            id: "PRRT_20".into(),
            first_comment: 20,
            resolved: true,
            can_resolve: true,
            can_unresolve: false,
        },
    ];
    let threads = github::group_threads(vec![root, reply, outdated], &states);
    let issue: github::IssueComment = serde_json::from_value(json!({
        "id": 5, "body": "Thanks!", "user": {"login": "ada"}, "created_at": "2024-01-01T00:00:00Z"
    }))
    .unwrap();

    let all = github_discussions(&threads, &[issue]);

    let ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["issue-5", "10", "20"]);
    let general = &all[0];
    assert!(!general.can_reply);
    assert_eq!(general.position, None);
    assert_eq!(general.resolve_id, None);
    let thread = &all[1];
    assert!(thread.can_reply);
    assert_eq!(thread.notes.len(), 2);
    assert_eq!(thread.notes[1].author.name, "Hubot");
    assert!(!thread.resolved);
    assert_eq!(thread.resolve_id.as_deref(), Some("PRRT_10"));
    let position = thread.position.clone().unwrap();
    assert_eq!(position.head_sha.as_deref(), Some("head1"));
    assert_eq!((position.old_line, position.new_line), (None, Some(3)));
    assert_eq!(position.new_path.as_deref(), Some("src/a.rs"));
    let old = &all[2];
    assert!(old.resolved);
    assert_eq!(old.resolve_id, None);
    let position = old.position.clone().unwrap();
    assert_eq!(position.head_sha.as_deref(), Some("old"));
    assert_eq!((position.old_line, position.new_line), (Some(8), None));
}

fn new_position(old_line: Option<u32>, new_line: Option<u32>) -> NewPosition {
    NewPosition {
        base_sha: "b".into(),
        start_sha: "s".into(),
        head_sha: "h".into(),
        old_path: "old.rs".into(),
        new_path: "new.rs".into(),
        old_line,
        new_line,
    }
}

#[rstest]
#[case::added(None, Some(4), 4, github::Side::Right)]
#[case::context(Some(3), Some(4), 4, github::Side::Right)]
#[case::removed(Some(3), None, 3, github::Side::Left)]
fn github_line_comments_pick_the_side(
    #[case] old_line: Option<u32>,
    #[case] new_line: Option<u32>,
    #[case] line: u32,
    #[case] side: github::Side,
) {
    let comment = github_comment("hi", &new_position(old_line, new_line)).unwrap();
    assert_eq!(comment.line, line);
    assert_eq!(comment.side, side);
    assert_eq!(comment.commit_id, "h");
    assert_eq!(comment.path, "new.rs");
}

#[test]
fn a_github_line_comment_needs_a_line() {
    assert!(matches!(
        github_comment("hi", &new_position(None, None)),
        Err(Error::InvalidPosition)
    ));
}

#[test]
fn conversation_comments_take_no_replies_on_github() {
    assert_eq!(github_reply_target("10").unwrap(), 10);
    assert!(matches!(
        github_reply_target("issue-5"),
        Err(Error::CannotReply(_))
    ));
}

#[test]
fn a_thread_whose_root_is_gone_takes_no_replies() {
    let orphan = review_comment(json!({
        "id": 11, "in_reply_to_id": 10, "path": "a.rs", "line": 1, "side": "RIGHT",
        "commit_id": "h", "body": "reply", "created_at": "2024-01-01T00:00:00Z"
    }));
    let threads = github::group_threads(vec![orphan], &[]);
    let all = github_discussions(&threads, &[]);
    assert_eq!(all[0].id, "11");
    assert!(!all[0].can_reply);
}

#[rstest]
#[case::gitlab(
    ForgeKind::GitLab,
    " https://gitlab.example.com/ ",
    "https://gitlab.example.com"
)]
#[case::gitlab_unset(ForgeKind::GitLab, "", "")]
#[case::github_unset(ForgeKind::GitHub, "  ", "https://github.com")]
#[case::github_www(ForgeKind::GitHub, "https://www.github.com/", "https://github.com")]
#[case::github_api(ForgeKind::GitHub, "https://API.github.com", "https://github.com")]
#[case::enterprise(
    ForgeKind::GitHub,
    "https://ghe.example.com/",
    "https://ghe.example.com"
)]
fn web_urls_normalise_the_configured_address(
    #[case] kind: ForgeKind,
    #[case] configured: &str,
    #[case] expected: &str,
) {
    assert_eq!(kind.web_url(configured), expected);
}

#[test]
fn only_github_limits_comments_to_its_hunks() {
    assert_eq!(ForgeKind::GitLab.comment_context(), None);
    assert_eq!(ForgeKind::GitHub.comment_context(), Some(3));
}

#[test]
fn forge_kind_serializes_as_lowercase_names() {
    assert_eq!(
        serde_json::to_value(ForgeKind::GitLab).unwrap(),
        json!("gitlab")
    );
    assert_eq!(
        serde_json::to_value(ForgeKind::GitHub).unwrap(),
        json!("github")
    );
    assert_eq!(ForgeKind::default(), ForgeKind::GitLab);
}
