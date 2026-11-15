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
