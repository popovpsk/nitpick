use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::{Client, ForgeKind, NewPosition};

const TOKEN: &str = "test-token";

fn json_response(status: u16, body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

fn position() -> NewPosition {
    NewPosition {
        base_sha: "b".into(),
        start_sha: "s".into(),
        head_sha: "h".into(),
        old_path: "a.rs".into(),
        new_path: "a.rs".into(),
        old_line: None,
        new_line: Some(4),
    }
}

#[tokio::test]
async fn gitlab_lists_open_merge_requests_as_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/g%2Fp/merge_requests"))
        .respond_with(json_response(
            200,
            json!([{"id": 1, "iid": 5, "project_id": 3, "title": "T",
                    "author": {"id": 1, "username": "ada", "name": "Ada"}}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    let list = client.open_requests("g/p").await.unwrap();

    assert_eq!(client.kind(), ForgeKind::GitLab);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reference(), "!5");
}

#[tokio::test]
async fn gitlab_line_comments_send_the_gitlab_position() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/g%2Fp/merge_requests/5/discussions"))
        .and(body_json(json!({
            "body": "hi",
            "position": {"position_type": "text", "base_sha": "b", "start_sha": "s",
                         "head_sha": "h", "old_path": "a.rs", "new_path": "a.rs", "new_line": 4}
        })))
        .respond_with(json_response(201, json!({"id": "d1", "notes": []})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client
        .comment("g/p", 5, "hi", Some(&position()))
        .await
        .unwrap();
}

#[tokio::test]
async fn github_line_and_general_comments_go_to_their_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/pulls/9/comments"))
        .and(body_json(json!({
            "body": "hi", "commit_id": "h", "path": "a.rs", "line": 4, "side": "RIGHT"
        })))
        .respond_with(json_response(201, json!({"id": 30})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/issues/9/comments"))
        .and(body_json(json!({"body": "overall"})))
        .respond_with(json_response(201, json!({"id": 31})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/pulls/9/comments/30/replies"))
        .and(body_json(json!({"body": "again"})))
        .respond_with(json_response(201, json!({"id": 32, "in_reply_to_id": 30})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    client
        .comment("o/r", 9, "hi", Some(&position()))
        .await
        .unwrap();
    client.comment("o/r", 9, "overall", None).await.unwrap();
    client.reply("o/r", 9, "30", "again").await.unwrap();
    assert!(client.reply("o/r", 9, "issue-31", "no").await.is_err());
}

#[tokio::test]
async fn github_discussions_combine_review_threads_and_conversation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/o/r/pulls/9/comments"))
        .respond_with(json_response(
            200,
            json!([{"id": 10, "path": "a.rs", "line": 2, "side": "RIGHT", "commit_id": "h",
                    "body": "nit", "user": {"login": "x"}, "created_at": "2024-01-02T00:00:00Z"}]),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            json!({"data": {"repository": {"pullRequest": {"reviewThreads": {
                "pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": [{"id": "PRRT_10", "isResolved": false, "viewerCanResolve": true,
                           "viewerCanUnresolve": false,
                           "comments": {"nodes": [{"databaseId": 10}]}}]
            }}}}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/o/r/issues/9/comments"))
        .respond_with(json_response(
            200,
            json!([{"id": 5, "body": "hello", "user": {"login": "y"},
                    "created_at": "2024-01-01T00:00:00Z"}]),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_json(json!({
            "query": "mutation($id: ID!) { result: resolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }",
            "variables": {"id": "PRRT_10"}
        })))
        .respond_with(json_response(
            200,
            json!({"data": {"result": {"thread": {"isResolved": true}}}}),
        ))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    let discussions = client.discussions("o/r", 9).await.unwrap();

    let ids: Vec<&str> = discussions.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["issue-5", "10"]);
    let resolve_id = discussions[1].resolve_id.clone().unwrap();
    client
        .set_resolved("o/r", 9, &resolve_id, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn connection_check_names_the_user_on_either_forge() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(json_response(
            200,
            json!({"id": 1, "username": "ada", "name": "Ada"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/user"))
        .respond_with(json_response(200, json!({"id": 2, "login": "octocat"})))
        .mount(&server)
        .await;

    let gitlab = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();
    let github = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    assert_eq!(gitlab.current_user().await.unwrap().username, "ada");
    assert_eq!(github.current_user().await.unwrap().username, "octocat");
}

#[test]
fn an_invalid_github_project_is_an_error_not_a_panic() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = Client::new(ForgeKind::GitHub, "https://github.com", TOKEN).unwrap();
    let result = runtime.block_on(client.request("not-a-repo", 1));
    assert!(result.is_err());
}

#[tokio::test]
async fn gitlab_edits_deletes_and_approves() {
    let server = MockServer::start().await;
    let note = "/api/v4/projects/g%2Fp/merge_requests/5/discussions/d1/notes/7";
    Mock::given(method("PUT"))
        .and(path(note))
        .and(body_json(json!({"body": "fixed"})))
        .respond_with(json_response(200, json!({"id": 7})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(note))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    for action in ["approve", "unapprove"] {
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v4/projects/g%2Fp/merge_requests/5/{action}"
            )))
            .respond_with(json_response(201, json!({})))
            .expect(1)
            .mount(&server)
            .await;
    }
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client.edit_note("g/p", 5, "d1", 7, "fixed").await.unwrap();
    client.delete_note("g/p", 5, "d1", 7).await.unwrap();
    client.approve("g/p", 5).await.unwrap();
    client.unapprove("g/p", 5).await.unwrap();
}

#[tokio::test]
async fn github_edits_each_comment_kind_at_its_endpoint() {
    let server = MockServer::start().await;
    for (verb, endpoint) in [
        ("PATCH", "/api/v3/repos/o/r/pulls/comments/30"),
        ("DELETE", "/api/v3/repos/o/r/pulls/comments/30"),
        ("PATCH", "/api/v3/repos/o/r/issues/comments/40"),
        ("DELETE", "/api/v3/repos/o/r/issues/comments/40"),
    ] {
        Mock::given(method(verb))
            .and(path(endpoint))
            .respond_with(json_response(200, json!({"id": 1})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/pulls/9/reviews"))
        .and(body_json(json!({"event": "APPROVE"})))
        .respond_with(json_response(200, json!({"id": 2})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    client.edit_note("o/r", 9, "30", 30, "x").await.unwrap();
    client.delete_note("o/r", 9, "30", 30).await.unwrap();
    client
        .edit_note("o/r", 9, "issue-40", 40, "x")
        .await
        .unwrap();
    client.delete_note("o/r", 9, "issue-40", 40).await.unwrap();
    client.approve("o/r", 9).await.unwrap();
    assert!(client.unapprove("o/r", 9).await.is_err());
}

#[tokio::test]
async fn a_gitlab_review_publishes_then_approves_then_summarises() {
    let server = MockServer::start().await;
    let mr = "/api/v4/projects/g%2Fp/merge_requests/5";
    Mock::given(method("POST"))
        .and(path(format!("{mr}/draft_notes/bulk_publish")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{mr}/discussions")))
        .and(body_json(json!({"body": "Looks good"})))
        .respond_with(json_response(201, json!({"id": "d1", "notes": []})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{mr}/approvals")))
        .respond_with(json_response(200, json!({"user_has_approved": false})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{mr}/approve")))
        .respond_with(json_response(201, json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();
    let pending = crate::PendingReview {
        handle: None,
        drafts: vec![crate::Draft {
            id: "41".into(),
            body: "n".into(),
            position: None,
            reply_to: None,
            resolve: false,
        }],
    };

    client
        .submit_review("g/p", 5, &pending, "Looks good", crate::Verdict::Approve)
        .await
        .unwrap();

    let order: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().rsplit('/').next().unwrap().to_string())
        .collect();
    assert_eq!(
        order,
        ["bulk_publish", "approvals", "approve", "discussions"]
    );
}

#[tokio::test]
async fn a_first_github_draft_starts_a_review() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/o/r/pulls/5/reviews"))
        .respond_with(json_response(200, json!([])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/o/r/pulls/5"))
        .respond_with(json_response(200, json!({"number": 5, "node_id": "PR5"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            json!({"data": {"addPullRequestReview": {"pullRequestReview": {"id": "R9"}},
                            "addPullRequestReviewThread": {"thread": {"id": "T1"}}}}),
        ))
        .expect(2)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    client
        .add_draft(
            "o/r",
            5,
            None,
            "Why?",
            crate::DraftTarget::Line(&position()),
        )
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let start: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
    let thread: serde_json::Value = serde_json::from_slice(&requests[3].body).unwrap();
    assert_eq!(start["variables"], json!({"pr": "PR5", "commit": "h"}));
    assert_eq!(
        thread["variables"],
        json!({"review": "R9", "path": "a.rs", "line": 4, "side": "RIGHT", "body": "Why?"})
    );
}

#[tokio::test]
async fn github_takes_no_general_draft() {
    let server = MockServer::start().await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();
    let handle = crate::ReviewHandle {
        id: 9,
        node_id: "R9".into(),
    };

    let err = client
        .add_draft("o/r", 5, Some(&handle), "x", crate::DraftTarget::General)
        .await
        .unwrap_err();

    assert!(matches!(err, crate::Error::Unsupported(_)));
}

#[tokio::test]
async fn a_gitlab_retry_does_not_approve_twice() {
    let server = MockServer::start().await;
    let mr = "/api/v4/projects/g%2Fp/merge_requests/5";
    Mock::given(method("GET"))
        .and(path(format!("{mr}/approvals")))
        .respond_with(json_response(200, json!({"user_has_approved": true})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client
        .submit_review(
            "g/p",
            5,
            &crate::PendingReview::default(),
            "",
            crate::Verdict::Approve,
        )
        .await
        .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_github_review_started_elsewhere_is_reused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/o/r/pulls/5/reviews"))
        .respond_with(json_response(
            200,
            json!([{"id": 9, "node_id": "R9", "state": "PENDING", "user": {"login": "me"}}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            json!({"data": {"addPullRequestReviewThread": {"thread": {"id": "T1"}}}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitHub, &server.uri(), TOKEN).unwrap();

    client
        .add_draft(
            "o/r",
            5,
            None,
            "Why?",
            crate::DraftTarget::Line(&position()),
        )
        .await
        .unwrap();
}

fn gitlab_draft(position: serde_json::Value) -> serde_json::Value {
    json!({"id": 41, "note": "n", "position": position})
}

fn line_draft() -> crate::Draft {
    crate::Draft {
        id: "41".into(),
        body: "n".into(),
        position: Some(crate::Position {
            base_sha: Some("b".into()),
            start_sha: Some("s".into()),
            head_sha: Some("h".into()),
            old_path: Some("a.rs".into()),
            new_path: Some("a.rs".into()),
            old_line: None,
            new_line: Some(4),
        }),
        reply_to: None,
        resolve: false,
    }
}

#[tokio::test]
async fn a_gitlab_line_draft_saved_without_its_line_is_removed() {
    let server = MockServer::start().await;
    let mr = "/api/v4/projects/g%2Fp/merge_requests/5";
    Mock::given(method("POST"))
        .and(path(format!("{mr}/draft_notes")))
        .respond_with(json_response(201, gitlab_draft(json!(null))))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{mr}/draft_notes/41")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    let err = client
        .add_draft("g/p", 5, None, "n", crate::DraftTarget::Line(&position()))
        .await
        .unwrap_err();

    assert!(matches!(err, crate::Error::DraftLineIgnored));
}

#[tokio::test]
async fn a_gitlab_line_draft_on_its_line_is_kept() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/g%2Fp/merge_requests/5/draft_notes"))
        .respond_with(json_response(
            201,
            gitlab_draft(json!({"new_path": "a.rs", "new_line": 4})),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client
        .add_draft("g/p", 5, None, "n", crate::DraftTarget::Line(&position()))
        .await
        .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_gitlab_draft_edit_that_drops_the_line_puts_it_back() {
    let server = MockServer::start().await;
    let draft = "/api/v4/projects/g%2Fp/merge_requests/5/draft_notes/41";
    Mock::given(method("PUT"))
        .and(path(draft))
        .and(body_json(json!({"note": "better"})))
        .respond_with(json_response(200, gitlab_draft(json!({}))))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(draft))
        .and(body_json(json!({
            "note": "better",
            "position": {"position_type": "text", "base_sha": "b", "start_sha": "s",
                         "head_sha": "h", "old_path": "a.rs", "new_path": "a.rs", "new_line": 4}
        })))
        .respond_with(json_response(
            200,
            gitlab_draft(json!({"new_path": "a.rs", "new_line": 4})),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client
        .edit_draft("g/p", 5, &line_draft(), "better")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_gitlab_draft_edit_that_keeps_the_line_is_one_call() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/v4/projects/g%2Fp/merge_requests/5/draft_notes/41",
        ))
        .and(body_json(json!({"note": "better"})))
        .respond_with(json_response(
            200,
            gitlab_draft(json!({"new_path": "a.rs", "new_line": 4})),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(ForgeKind::GitLab, &server.uri(), TOKEN).unwrap();

    client
        .edit_draft("g/p", 5, &line_draft(), "better")
        .await
        .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
