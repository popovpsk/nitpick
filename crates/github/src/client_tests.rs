use rstest::rstest;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use crate::client::{MAX_COMMENT_PAGES, PER_PAGE};
use crate::models_tests::{
    ISSUE_COMMENT, OUTDATED_COMMENT, PULL_REQUEST, REPLY_COMMENT, ROOT_COMMENT, json_list,
};
use crate::{Client, Error, NewReviewComment, RepoRef, Side};

const TOKEN: &str = "ghp_test-token";
const REPO_PATH: &str = "/api/v3/repos/octocat/Hello-World";

const USER: &str = r#"{
  "login": "octocat",
  "id": 1,
  "type": "User",
  "name": "The Octocat",
  "company": "GitHub"
}"#;

fn repo() -> RepoRef {
    RepoRef::parse("octocat/Hello-World").unwrap()
}

fn client(server: &MockServer) -> Client {
    Client::new(&server.uri(), TOKEN).unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

async fn received(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn threads_answer(nodes: Value, next: Option<&str>) -> String {
    json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
            "nodes": nodes
        }}}}
    })
    .to_string()
}

#[tokio::test]
async fn current_user_sends_the_github_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/user"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(header("accept", "application/vnd.github+json"))
        .and(header("x-github-api-version", "2022-11-28"))
        .and(header("user-agent", "nitpick"))
        .respond_with(json_response(200, USER))
        .expect(1)
        .mount(&server)
        .await;

    let user = client(&server).current_user().await.unwrap();

    assert_eq!(user.login, "octocat");
    assert_eq!(user.display_name(), "The Octocat");
}

#[rstest]
#[case::github("https://github.com", "https://github.com", "https://api.github.com")]
#[case::www(
    "https://www.github.com/",
    "https://github.com",
    "https://api.github.com"
)]
#[case::api(
    "https://api.github.com",
    "https://github.com",
    "https://api.github.com"
)]
#[case::enterprise(
    "https://ghe.example.com/",
    "https://ghe.example.com",
    "https://ghe.example.com/api/v3"
)]
#[case::enterprise_api(
    "https://ghe.example.com/api/v3",
    "https://ghe.example.com",
    "https://ghe.example.com/api/v3"
)]
#[case::bare_host(
    "ghe.example.com",
    "https://ghe.example.com",
    "https://ghe.example.com/api/v3"
)]
#[case::bare_github("github.com", "https://github.com", "https://api.github.com")]
#[case::data_residency(
    "https://octo.ghe.com",
    "https://octo.ghe.com",
    "https://api.octo.ghe.com"
)]
#[case::data_residency_api(
    "https://api.octo.ghe.com/",
    "https://octo.ghe.com",
    "https://api.octo.ghe.com"
)]
fn api_root_follows_the_host(#[case] base: &str, #[case] web: &str, #[case] api: &str) {
    let client = Client::new(base, TOKEN).unwrap();
    assert_eq!(client.base_url(), web);
    assert_eq!(client.api_root(), api);
}

#[rstest]
#[case::empty("")]
#[case::ftp("ftp://github.com")]
#[case::credentials("https://user:pass@github.com")]
fn rejects_invalid_base_urls(#[case] base_url: &str) {
    assert!(matches!(
        Client::new(base_url, TOKEN),
        Err(Error::InvalidUrl(_))
    ));
}

#[rstest]
#[case::empty("  ")]
#[case::newline("abc\ndef")]
fn rejects_invalid_tokens(#[case] token: &str) {
    assert!(matches!(
        Client::new("https://github.com", token),
        Err(Error::InvalidToken)
    ));
}

#[rstest]
#[case::one_segment("octocat")]
#[case::three_segments("a/b/c")]
#[case::empty_owner("/repo")]
fn rejects_invalid_repository_paths(#[case] path: &str) {
    assert!(matches!(RepoRef::parse(path), Err(Error::InvalidRepo(_))));
}

#[test]
fn debug_output_hides_the_token() {
    let client = Client::new("https://github.com", TOKEN).unwrap();
    let debug = format!("{client:?}");
    assert!(debug.contains("https://api.github.com"));
    assert!(!debug.contains(TOKEN));
}

#[tokio::test]
async fn lists_open_pull_requests_newest_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls")))
        .and(query_param("state", "open"))
        .and(query_param("sort", "updated"))
        .and(query_param("direction", "desc"))
        .and(query_param("per_page", PER_PAGE.to_string().as_str()))
        .respond_with(json_response(200, &json_list(&[PULL_REQUEST])))
        .expect(1)
        .mount(&server)
        .await;

    let list = client(&server).pull_requests(&repo()).await.unwrap();

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].number, 1347);
}

#[tokio::test]
async fn pull_request_lists_follow_the_link_header() {
    let server = MockServer::start().await;
    let next = format!("{}{REPO_PATH}/pulls?page=2", server.uri());
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls")))
        .and(query_param("page", "2"))
        .respond_with(json_response(200, &json_list(&[PULL_REQUEST])))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls")))
        .and(query_param("state", "open"))
        .respond_with(
            json_response(200, &json_list(&[PULL_REQUEST])).insert_header(
                "link",
                format!("<{next}>; rel=\"next\", <{next}>; rel=\"last\""),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let list = client(&server).pull_requests(&repo()).await.unwrap();

    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn does_not_follow_a_next_link_to_another_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments")))
        .respond_with(
            json_response(200, &json_list(&[ROOT_COMMENT]))
                .insert_header("link", "<https://evil.example/steal?page=2>; rel=\"next\""),
        )
        .expect(1)
        .mount(&server)
        .await;

    let comments = client(&server)
        .review_comments(&repo(), 1347)
        .await
        .unwrap();

    assert_eq!(comments.len(), 1);
}

#[tokio::test]
async fn comment_pages_stop_at_the_cap() {
    let server = MockServer::start().await;
    let next = format!("{}{REPO_PATH}/pulls/1347/comments?page=n", server.uri());
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments")))
        .respond_with(
            json_response(200, &json_list(&[ROOT_COMMENT]))
                .insert_header("link", format!("<{next}>; rel=\"next\"")),
        )
        .expect(MAX_COMMENT_PAGES as u64)
        .mount(&server)
        .await;

    let comments = client(&server)
        .review_comments(&repo(), 1347)
        .await
        .unwrap();

    assert_eq!(comments.len(), MAX_COMMENT_PAGES);
}

#[tokio::test]
async fn gets_one_pull_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347")))
        .respond_with(json_response(200, PULL_REQUEST))
        .expect(1)
        .mount(&server)
        .await;

    let pr = client(&server).pull_request(&repo(), 1347).await.unwrap();

    assert_eq!(pr.base.sha, "4ab3f2a4a4d1c1e5a1b8c6c0e2f3b1d2c3a4b5c6");
}

#[tokio::test]
async fn review_threads_group_comments_and_carry_graphql_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments")))
        .respond_with(json_response(
            200,
            &json_list(&[ROOT_COMMENT, OUTDATED_COMMENT, REPLY_COMMENT]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let nodes = json!([{
        "id": "PRRT_10",
        "isResolved": true,
        "viewerCanResolve": false,
        "viewerCanUnresolve": true,
        "comments": {"nodes": [{"databaseId": 10}]}
    }]);
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(200, &threads_answer(nodes, None)))
        .expect(1)
        .mount(&server)
        .await;

    let threads = client(&server).review_threads(&repo(), 1347).await.unwrap();

    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].comments.len(), 2);
    let state = threads[0].state.as_ref().unwrap();
    assert_eq!(state.id, "PRRT_10");
    assert!(state.resolved);
    assert!(state.can_unresolve);
    assert_eq!(threads[1].state, None);
    let requests = received(&server).await;
    let graphql: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(graphql["variables"]["owner"], "octocat");
    assert_eq!(graphql["variables"]["name"], "Hello-World");
    assert_eq!(graphql["variables"]["number"], 1347);
}

#[tokio::test]
async fn thread_states_follow_the_graphql_cursor() {
    let server = MockServer::start().await;
    let node = |id: &str, comment: u64| {
        json!({"id": id, "isResolved": false, "viewerCanResolve": true,
               "viewerCanUnresolve": false, "comments": {"nodes": [{"databaseId": comment}]}})
    };
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_partial_after("c1"))
        .respond_with(json_response(
            200,
            &threads_answer(json!([node("PRRT_2", 2)]), None),
        ))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            &threads_answer(json!([node("PRRT_1", 1)]), Some("c1")),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let states = client(&server).thread_states(&repo(), 1347).await.unwrap();

    let ids: Vec<&str> = states.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["PRRT_1", "PRRT_2"]);
}

fn body_partial_after(cursor: &'static str) -> impl wiremock::Match {
    move |request: &Request| {
        serde_json::from_slice::<Value>(&request.body)
            .is_ok_and(|body| body["variables"]["after"] == cursor)
    }
}

#[tokio::test]
async fn review_threads_survive_a_graphql_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments")))
        .respond_with(json_response(200, &json_list(&[ROOT_COMMENT])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            r#"{"data": null, "errors": [{"message": "Resource not accessible by integration"}]}"#,
        ))
        .mount(&server)
        .await;

    let threads = client(&server).review_threads(&repo(), 1347).await.unwrap();

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].state, None);
}

#[tokio::test]
async fn graphql_errors_are_reported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            r#"{"errors": [{"message": "Could not resolve to a node"}]}"#,
        ))
        .mount(&server)
        .await;

    let err = client(&server)
        .set_thread_resolved("PRRT_x", true)
        .await
        .unwrap_err();

    assert!(
        matches!(&err, Error::GraphQl(m) if m == "Could not resolve to a node"),
        "{err}"
    );
}

#[rstest]
#[case::resolve(true, "resolveReviewThread")]
#[case::unresolve(false, "unresolveReviewThread")]
#[tokio::test]
async fn resolving_sends_the_thread_mutation(#[case] resolved: bool, #[case] mutation: &str) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(json_response(
            200,
            &json!({"data": {"result": {"thread": {"isResolved": resolved}}}}).to_string(),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let state = client(&server)
        .set_thread_resolved("PRRT_10", resolved)
        .await
        .unwrap();

    assert_eq!(state, resolved);
    let body: Value = serde_json::from_slice(&received(&server).await[0].body).unwrap();
    assert_eq!(body["variables"]["id"], "PRRT_10");
    let query = body["query"].as_str().unwrap();
    assert!(query.contains(&format!("result: {mutation}(")), "{query}");
}

#[tokio::test]
async fn creates_a_line_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments")))
        .and(body_json(json!({
            "body": "Looks off",
            "commit_id": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
            "path": "src/lib.rs",
            "line": 12,
            "side": "LEFT"
        })))
        .respond_with(json_response(201, ROOT_COMMENT))
        .expect(1)
        .mount(&server)
        .await;

    let comment = NewReviewComment {
        body: "Looks off".into(),
        commit_id: "6dcb09b5b57875f334f61aebed695e2e4193db5e".into(),
        path: "src/lib.rs".into(),
        line: 12,
        side: Side::Left,
    };
    let created = client(&server)
        .create_review_comment(&repo(), 1347, &comment)
        .await
        .unwrap();

    assert_eq!(created.id, 10);
}

#[tokio::test]
async fn replies_to_the_thread_root() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/pulls/1347/comments/10/replies")))
        .and(body_json(json!({"body": "Done"})))
        .respond_with(json_response(201, REPLY_COMMENT))
        .expect(1)
        .mount(&server)
        .await;

    let reply = client(&server)
        .reply(&repo(), 1347, 10, "Done")
        .await
        .unwrap();

    assert_eq!(reply.in_reply_to_id, Some(10));
}

#[tokio::test]
async fn general_comments_go_to_the_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/issues/1347/comments")))
        .and(body_json(json!({"body": "Me too"})))
        .respond_with(json_response(201, ISSUE_COMMENT))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/issues/1347/comments")))
        .respond_with(json_response(200, &json_list(&[ISSUE_COMMENT])))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server);
    let created = client
        .create_issue_comment(&repo(), 1347, "Me too")
        .await
        .unwrap();
    let all = client.issue_comments(&repo(), 1347).await.unwrap();

    assert_eq!(created.id, 1);
    assert_eq!(all, vec![created]);
}

#[rstest]
#[case::unauthorized(401, r#"{"message": "Bad credentials"}"#, "unauthorized")]
#[case::not_found(404, r#"{"message": "Not Found"}"#, "not_found")]
#[case::validation(
    422,
    r#"{"message": "Validation Failed", "errors": [{"resource": "PullRequestReviewComment", "code": "custom", "message": "line must be part of the diff"}]}"#,
    "Validation Failed: line must be part of the diff"
)]
#[case::rate_limited(
    403,
    r#"{"message": "API rate limit exceeded", "documentation_url": "https://docs.github.com"}"#,
    "API rate limit exceeded"
)]
#[case::plain_text(502, "Bad gateway from the proxy", "Bad gateway from the proxy")]
#[case::empty(500, "", "Internal Server Error")]
#[tokio::test]
async fn maps_error_statuses(#[case] status: u16, #[case] body: &str, #[case] expected: &str) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347")))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;

    let err = client(&server)
        .pull_request(&repo(), 1347)
        .await
        .unwrap_err();

    match (expected, err) {
        ("unauthorized", Error::Unauthorized) => {}
        ("not_found", Error::NotFound(path)) => {
            assert_eq!(path, "/repos/octocat/Hello-World/pulls/1347")
        }
        (
            expected,
            Error::Status {
                status: got,
                message,
            },
        ) => {
            assert_eq!(got, status);
            assert_eq!(message, expected);
        }
        (expected, other) => panic!("{expected}: got {other:?}"),
    }
}

#[tokio::test]
async fn an_unparsable_body_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/pulls/1347")))
        .respond_with(json_response(200, "<html>"))
        .mount(&server)
        .await;

    let err = client(&server)
        .pull_request(&repo(), 1347)
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Decode(m) if m.starts_with("/repos/octocat/Hello-World/pulls/1347"))
    );
}
