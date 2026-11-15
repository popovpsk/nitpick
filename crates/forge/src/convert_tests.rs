use rstest::rstest;
use serde_json::json;

use crate::convert::{
    github_comment, github_discussions, github_reply_target, github_request, gitlab_discussion,
    gitlab_request, own_avatar,
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

#[rstest]
#[case::uploaded(
    "https://gitlab.example.com/uploads/-/system/user/avatar/7/a.png",
    true
)]
#[case::github("https://avatars.githubusercontent.com/u/1?v=4", true)]
#[case::gravatar("https://secure.gravatar.com/avatar/abc?s=80&d=identicon", false)]
#[case::bare_gravatar("https://gravatar.com/avatar/abc", false)]
#[case::not_a_url("avatar.png", false)]
#[case::blank("  ", false)]
fn avatars_keep_only_uploaded_pictures(#[case] url: &str, #[case] kept: bool) {
    assert_eq!(own_avatar(Some(url)).is_some(), kept);
}

#[test]
fn gitlab_approvals_say_who_approved_and_what_is_allowed() {
    let approvals: gitlab::Approvals = serde_json::from_value(json!({
        "approved_by": [{"user": {"id": 1, "username": "bob", "name": "Bob"}}],
        "user_has_approved": false,
        "user_can_approve": true
    }))
    .unwrap();
    let approval = crate::convert::gitlab_approval(&approvals);
    assert_eq!(approval.approvers[0].username, "bob");
    assert!(approval.can_approve && !approval.can_unapprove && !approval.approved_by_me);
}

#[rstest]
#[case::approved(&[("me", "APPROVED")], true, false)]
#[case::later_changes_requested(&[("me", "APPROVED"), ("me", "CHANGES_REQUESTED")], false, true)]
#[case::comments_do_not_reset(&[("me", "APPROVED"), ("me", "COMMENTED")], true, false)]
#[case::someone_else(&[("bob", "APPROVED")], false, true)]
fn github_approval_follows_each_reviewers_latest_verdict(
    #[case] reviews: &[(&str, &str)],
    #[case] mine: bool,
    #[case] can_approve: bool,
) {
    let reviews: Vec<github::Review> = reviews
        .iter()
        .map(|(login, state)| {
            serde_json::from_value(json!({"user": {"login": login}, "state": state})).unwrap()
        })
        .collect();
    let approval = crate::convert::github_approval(&reviews, "me", "author");
    assert_eq!(approval.approved_by_me, mine);
    assert_eq!(approval.can_approve, can_approve);
    assert!(!approval.can_unapprove);
    assert!(!crate::convert::github_approval(&[], "author", "author").can_approve);
}

#[test]
fn list_entries_carry_people_and_comment_counts() {
    let mut mr = gitlab_mr("opened", false);
    mr.user_notes_count = 4;
    mr.reviewers = vec![gitlab::User {
        id: 2,
        username: "bob".into(),
        name: "Bob".into(),
        avatar_url: None,
    }];
    let request = gitlab_request(&mr);
    assert_eq!(request.comments, Some(4));
    assert_eq!(request.reviewers[0].username, "bob");
    assert_eq!(github_request(&github_pr("open", false)).comments, None);
}

#[rstest]
#[case::text(Some("## Summary"), Some("## Summary"))]
#[case::blank(Some("  \n"), None)]
#[case::missing(None, None)]
fn a_blank_description_is_none(#[case] text: Option<&str>, #[case] expected: Option<&str>) {
    let mut mr = gitlab_mr("opened", false);
    mr.description = text.map(str::to_string);
    assert_eq!(gitlab_request(&mr).description.as_deref(), expected);
    let mut pr = github_pr("open", false);
    pr.body = text.map(str::to_string);
    assert_eq!(github_request(&pr).description.as_deref(), expected);
}

#[test]
fn a_gitlab_general_draft_has_no_position() {
    let draft: gitlab::DraftNote = serde_json::from_value(json!({
        "id": 41, "note": "Overall", "discussion_id": null, "resolve_discussion": false,
        "position": {"base_sha": null, "start_sha": null, "head_sha": null, "old_path": null,
                     "new_path": null, "position_type": "text", "old_line": null,
                     "new_line": null}
    }))
    .unwrap();

    let draft = crate::convert::gitlab_draft(&draft);

    assert_eq!(draft.id, "41");
    assert_eq!(draft.position, None);
}

#[test]
fn a_gitlab_line_draft_keeps_its_line() {
    let draft: gitlab::DraftNote = serde_json::from_value(json!({
        "id": 42, "note": "Rename", "resolve_discussion": false,
        "position": {"base_sha": "b", "start_sha": "s", "head_sha": "h", "old_path": "a.rs",
                     "new_path": "a.rs", "position_type": "text", "new_line": 3}
    }))
    .unwrap();

    let draft = crate::convert::gitlab_draft(&draft);

    let position = draft.position.unwrap();
    assert_eq!(
        (position.new_line, position.head_sha.as_deref()),
        (Some(3), Some("h"))
    );
}

#[rstest]
#[case::right("RIGHT", None, Some(7), None)]
#[case::left("LEFT", None, None, Some(7))]
fn github_drafts_map_their_side(
    #[case] side: &str,
    #[case] reply: Option<u64>,
    #[case] new_line: Option<u32>,
    #[case] old_line: Option<u32>,
) {
    let comment: github::ReviewComment = serde_json::from_value(json!({
        "id": 3, "node_id": "C3", "in_reply_to_id": reply, "body": "b", "path": "a.rs",
        "line": 7, "side": side, "commit_id": "h"
    }))
    .unwrap();

    let draft = crate::convert::github_draft(&comment);

    assert_eq!(draft.id, "C3");
    let position = draft.position.unwrap();
    assert_eq!((position.new_line, position.old_line), (new_line, old_line));
}

#[test]
fn a_github_draft_reply_names_its_thread() {
    let comment: github::ReviewComment = serde_json::from_value(json!({
        "id": 4, "node_id": "C4", "in_reply_to_id": 3, "body": "b", "path": "a.rs",
        "line": 7, "side": "RIGHT", "commit_id": "h"
    }))
    .unwrap();

    let draft = crate::convert::github_draft(&comment);

    assert_eq!(
        (draft.reply_to.as_deref(), draft.position),
        (Some("3"), None)
    );
}
