use rstest::rstest;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use crate::client::MAX_PAGES;
use crate::models_tests::{
    DIFF_DISCUSSION, GENERAL_DISCUSSION, MERGE_REQUEST, SYSTEM_DISCUSSION, json_list,
};
use crate::{Client, Error, MrQuery, MrState, NewPosition, ProjectRef};

const TOKEN: &str = "glpat-test-token";

const USER: &str = r#"{
  "id": 1,
  "username": "john_smith",
  "name": "John Smith",
  "state": "active",
  "locked": false,
  "avatar_url": "http://localhost:3000/uploads/user/avatar/1/cd8.jpeg",
  "web_url": "http://localhost:3000/john_smith",
  "created_at": "2012-05-23T08:00:58Z",
  "bio": "",
  "is_admin": false,
  "two_factor_enabled": true
}"#;

const PROJECT: &str = r#"{
  "id": 3,
  "description": "Lorem ipsum",
  "name": "Project",
  "name_with_namespace": "Group / Sub / Project",
  "path": "project",
  "path_with_namespace": "group/sub/project",
  "created_at": "2013-09-30T13:46:02Z",
  "default_branch": "main",
  "tag_list": [],
  "ssh_url_to_repo": "git@example.com:group/sub/project.git",
  "http_url_to_repo": "https://example.com/group/sub/project.git",
  "web_url": "https://example.com/group/sub/project",
  "namespace": {"id": 3, "name": "Sub", "path": "sub", "kind": "group", "full_path": "group/sub"},
  "permissions": {"project_access": {"access_level": 30, "notification_level": 3}}
}"#;

const NOTE: &str = r#"{
  "id": 302,
  "type": "DiscussionNote",
  "body": "Thanks, will do.",
  "attachment": null,
  "author": {"id": 1, "username": "john_smith", "name": "John Smith", "avatar_url": null},
  "created_at": "2013-10-02T09:22:45Z",
  "updated_at": "2013-10-02T10:22:45Z",
  "system": false,
  "noteable_id": 377,
  "noteable_type": "MergeRequest",
  "resolvable": true,
  "resolved": false,
  "confidential": false,
  "internal": false
}"#;

const MR_PATH: &str = "/api/v4/projects/group%2Fsub%2Fproject/merge_requests/133";

fn project() -> ProjectRef {
    ProjectRef::Path("group/sub/project".into())
}

fn client(server: &MockServer) -> Client {
    Client::new(&server.uri(), TOKEN).unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

fn discussion_with_id(id: &str) -> String {
    let mut value: Value = serde_json::from_str(GENERAL_DISCUSSION).unwrap();
    value["id"] = json!(id);
    value.to_string()
}

async fn received(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

#[tokio::test]
async fn current_user_sends_token_and_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .and(header("PRIVATE-TOKEN", TOKEN))
        .and(header("user-agent", "slopcop"))
        .respond_with(json_response(200, USER))
        .expect(1)
        .mount(&server)
        .await;

    let user = client(&server).current_user().await.unwrap();

    assert_eq!(user.id, 1);
    assert_eq!(user.username, "john_smith");
    assert_eq!(user.name, "John Smith");
    assert!(user.avatar_url.is_some());
}

#[rstest]
#[case::trailing_slash("/gitlab/")]
#[case::pasted_api_root("/gitlab/api/v4")]
#[case::no_slash("/gitlab")]
#[tokio::test]
async fn relative_root_is_kept(#[case] suffix: &str) {
    let server = MockServer::start().await;
    Mock::given(path("/gitlab/api/v4/user"))
        .respond_with(json_response(200, USER))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new(&format!("{}{suffix}", server.uri()), TOKEN).unwrap();

    assert_eq!(client.base_url(), format!("{}/gitlab", server.uri()));
    client.current_user().await.unwrap();
}

#[rstest]
#[case::empty("")]
#[case::no_scheme("gitlab.example.com")]
#[case::ftp("ftp://gitlab.example.com")]
#[case::credentials("https://user:pass@gitlab.example.com")]
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
        Client::new("https://gitlab.example.com", token),
        Err(Error::InvalidToken)
    ));
}

#[test]
fn debug_output_hides_the_token() {
    let client = Client::new("https://gitlab.example.com/", TOKEN).unwrap();
    let debug = format!("{client:?}");
    assert!(debug.contains("https://gitlab.example.com"));
    assert!(!debug.contains(TOKEN));
}

#[tokio::test]
async fn project_path_reaches_server_as_one_encoded_segment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/group%2Fsub%2Fproject"))
        .respond_with(json_response(200, PROJECT))
        .expect(1)
        .mount(&server)
        .await;

    let project = client(&server).project(&project()).await.unwrap();

    assert_eq!(project.id, 3);
    assert_eq!(project.path_with_namespace, "group/sub/project");
    assert_eq!(project.default_branch.as_deref(), Some("main"));
    let requests = received(&server).await;
    assert_eq!(
        requests[0].url.path(),
        "/api/v4/projects/group%2Fsub%2Fproject"
    );
}

#[tokio::test]
async fn project_by_numeric_id() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v4/projects/3"))
        .respond_with(json_response(200, PROJECT))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).project(&ProjectRef::Id(3)).await.unwrap();
}

#[tokio::test]
async fn merge_request_with_diff_refs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(MR_PATH))
        .respond_with(json_response(200, MERGE_REQUEST))
        .expect(1)
        .mount(&server)
        .await;

    let mr = client(&server)
        .merge_request(&project(), 133)
        .await
        .unwrap();

    assert_eq!(mr.iid, 133);
    assert_eq!(mr.title, "Manual job rules");
    assert_eq!(mr.source_branch, "manual-job-rules");
    assert_eq!(mr.author.username, "marcel.amirault");
    let refs = mr.diff_refs.unwrap();
    assert_eq!(refs.start_sha, "1162f719d711319a2efb2a35566f3bfdadee8bab");
}

#[tokio::test]
async fn merge_requests_send_the_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v4/projects/group%2Fsub%2Fproject/merge_requests",
        ))
        .and(query_param("state", "merged"))
        .and(query_param("order_by", "updated_at"))
        .and(query_param("per_page", "20"))
        .and(query_param("search", "fix login"))
        .and(query_param("source_branch", "feature/a"))
        .and(query_param_is_missing("target_branch"))
        .respond_with(json_response(200, &json_list(&[MERGE_REQUEST])))
        .expect(1)
        .mount(&server)
        .await;
    let query = MrQuery {
        state: MrState::Merged,
        search: Some("fix login".into()),
        source_branch: Some("feature/a".into()),
        target_branch: None,
        per_page: 20,
    };

    let mrs = client(&server)
        .merge_requests(&project(), &query)
        .await
        .unwrap();

    assert_eq!(mrs.len(), 1);
}

#[tokio::test]
async fn merge_requests_default_query() {
    let server = MockServer::start().await;
    Mock::given(path(
        "/api/v4/projects/group%2Fsub%2Fproject/merge_requests",
    ))
    .and(query_param("state", "opened"))
    .and(query_param("per_page", "50"))
    .and(query_param_is_missing("search"))
    .respond_with(json_response(200, "[]"))
    .expect(1)
    .mount(&server)
    .await;

    let mrs = client(&server)
        .merge_requests(&project(), &MrQuery::default())
        .await
        .unwrap();

    assert!(mrs.is_empty());
}

#[tokio::test]
async fn discussions_follow_x_next_page_across_three_pages() {
    let server = MockServer::start().await;
    let discussions_path = format!("{MR_PATH}/discussions");
    let pages = [
        ("1", "2", json_list(&[DIFF_DISCUSSION, GENERAL_DISCUSSION])),
        ("2", "3", json_list(&[SYSTEM_DISCUSSION])),
        ("3", "", json_list(&[&discussion_with_id("last")])),
    ];
    for (page, next, body) in pages {
        Mock::given(method("GET"))
            .and(path(discussions_path.as_str()))
            .and(query_param("page", page))
            .and(query_param("per_page", "100"))
            .respond_with(json_response(200, &body).insert_header("x-next-page", next))
            .expect(1)
            .mount(&server)
            .await;
    }

    let discussions = client(&server).discussions(&project(), 133).await.unwrap();

    let ids: Vec<&str> = discussions.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "6a9c1750b37d513a43987b574953fceb50b03ce7",
            "87805b7c09016a7058e91bdbe7b29d1f284a39e6",
            "11aa",
            "last"
        ]
    );
    assert!(discussions[2].notes[0].system);
}

#[tokio::test]
async fn discussions_follow_link_header() {
    let server = MockServer::start().await;
    let discussions_path = format!("{MR_PATH}/discussions");
    let next = format!(
        "<{}{discussions_path}?page=2&per_page=100>; rel=\"next\", <{}{discussions_path}?page=1&per_page=100>; rel=\"first\"",
        server.uri(),
        server.uri()
    );
    Mock::given(path(discussions_path.as_str()))
        .and(query_param("page", "1"))
        .respond_with(
            json_response(200, &json_list(&[DIFF_DISCUSSION])).insert_header("link", next),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(discussions_path.as_str()))
        .and(query_param("page", "2"))
        .respond_with(json_response(200, &json_list(&[GENERAL_DISCUSSION])))
        .expect(1)
        .mount(&server)
        .await;

    let discussions = client(&server).discussions(&project(), 133).await.unwrap();

    assert_eq!(discussions.len(), 2);
}

#[tokio::test]
async fn discussions_ignore_link_to_another_host() {
    let server = MockServer::start().await;
    let next = "<https://elsewhere.example.com/api/v4/x?page=2>; rel=\"next\"";
    Mock::given(path(format!("{MR_PATH}/discussions")))
        .respond_with(
            json_response(200, &json_list(&[DIFF_DISCUSSION])).insert_header("link", next),
        )
        .expect(1)
        .mount(&server)
        .await;

    let discussions = client(&server).discussions(&project(), 133).await.unwrap();

    assert_eq!(discussions.len(), 1);
}

#[tokio::test]
async fn discussions_stop_at_the_page_cap() {
    let server = MockServer::start().await;
    Mock::given(path(format!("{MR_PATH}/discussions")))
        .respond_with(
            json_response(200, &json_list(&[GENERAL_DISCUSSION])).insert_header("x-next-page", "2"),
        )
        .mount(&server)
        .await;

    let discussions = client(&server).discussions(&project(), 133).await.unwrap();

    assert_eq!(discussions.len(), MAX_PAGES);
    assert_eq!(received(&server).await.len(), MAX_PAGES);
}

fn position(old_line: Option<u32>, new_line: Option<u32>) -> NewPosition {
    NewPosition {
        base_sha: "base".into(),
        start_sha: "start".into(),
        head_sha: "head".into(),
        old_path: "src/old.rs".into(),
        new_path: "src/new.rs".into(),
        old_line,
        new_line,
    }
}

#[rstest]
#[case::added_line(None, Some(12), json!({"new_line": 12}))]
#[case::removed_line(Some(7), None, json!({"old_line": 7}))]
#[case::context_line(Some(7), Some(9), json!({"old_line": 7, "new_line": 9}))]
#[tokio::test]
async fn create_diff_discussion_body(
    #[case] old_line: Option<u32>,
    #[case] new_line: Option<u32>,
    #[case] lines: Value,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{MR_PATH}/discussions")))
        .and(header("content-type", "application/json"))
        .respond_with(json_response(201, DIFF_DISCUSSION))
        .expect(1)
        .mount(&server)
        .await;

    let discussion = client(&server)
        .create_discussion(
            &project(),
            133,
            "Please rename",
            Some(&position(old_line, new_line)),
        )
        .await
        .unwrap();

    assert!(discussion.position().is_some());
    let mut expected_position = json!({
        "position_type": "text",
        "base_sha": "base",
        "start_sha": "start",
        "head_sha": "head",
        "old_path": "src/old.rs",
        "new_path": "src/new.rs"
    });
    for (key, value) in lines.as_object().unwrap() {
        expected_position[key] = value.clone();
    }
    let sent: Value = received(&server).await[0].body_json().unwrap();
    assert_eq!(
        sent,
        json!({"body": "Please rename", "position": expected_position})
    );
}

#[tokio::test]
async fn create_general_discussion_has_no_position() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{MR_PATH}/discussions")))
        .and(body_json(json!({"body": "Overall LGTM"})))
        .respond_with(json_response(201, GENERAL_DISCUSSION))
        .expect(1)
        .mount(&server)
        .await;

    let discussion = client(&server)
        .create_discussion(&project(), 133, "Overall LGTM", None)
        .await
        .unwrap();

    assert!(discussion.individual_note);
}

#[tokio::test]
async fn reply_posts_a_note_to_the_discussion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{MR_PATH}/discussions/6a9c1750b3/notes")))
        .and(body_json(json!({"body": "Thanks, will do."})))
        .respond_with(json_response(201, NOTE))
        .expect(1)
        .mount(&server)
        .await;

    let note = client(&server)
        .reply(&project(), 133, "6a9c1750b3", "Thanks, will do.")
        .await
        .unwrap();

    assert_eq!(note.id, 302);
    assert_eq!(note.body, "Thanks, will do.");
    assert!(note.resolvable);
}

#[rstest]
#[case::resolve(true, "true")]
#[case::unresolve(false, "false")]
#[tokio::test]
async fn set_resolved_puts_the_flag(#[case] resolved: bool, #[case] param: &str) {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!("{MR_PATH}/discussions/6a9c1750b3")))
        .and(query_param("resolved", param))
        .respond_with(json_response(200, DIFF_DISCUSSION))
        .expect(1)
        .mount(&server)
        .await;

    let discussion = client(&server)
        .set_resolved(&project(), 133, "6a9c1750b3", resolved)
        .await
        .unwrap();

    assert!(discussion.is_resolvable());
}

#[tokio::test]
async fn unauthorized_maps_to_its_own_variant() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v4/user"))
        .respond_with(json_response(401, r#"{"message":"401 Unauthorized"}"#))
        .mount(&server)
        .await;

    let error = client(&server).current_user().await.unwrap_err();

    assert!(matches!(error, Error::Unauthorized));
}

#[tokio::test]
async fn not_found_names_what_was_requested() {
    let server = MockServer::start().await;
    Mock::given(path(MR_PATH))
        .respond_with(json_response(404, r#"{"message":"404 Not found"}"#))
        .mount(&server)
        .await;

    let error = client(&server)
        .merge_request(&project(), 133)
        .await
        .unwrap_err();

    match error {
        Error::NotFound(what) => {
            assert_eq!(what, "/projects/group%2Fsub%2Fproject/merge_requests/133")
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[rstest]
#[case::message_object(
    422,
    r#"{"message":{"base":["Line code can't be blank"]}}"#,
    r#"{"base":["Line code can't be blank"]}"#
)]
#[case::message_array(
    400,
    r#"{"message":["position is invalid"]}"#,
    r#"["position is invalid"]"#
)]
#[case::message_string(403, r#"{"message":"403 Forbidden"}"#, "403 Forbidden")]
#[case::error_field(400, r#"{"error":"resolved is invalid"}"#, "resolved is invalid")]
#[case::plain_body(502, "Bad gateway from proxy", "Bad gateway from proxy")]
#[case::empty_body(503, "", "Service Unavailable")]
#[tokio::test]
async fn other_statuses_carry_gitlabs_message(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: &str,
) {
    let server = MockServer::start().await;
    Mock::given(path(format!("{MR_PATH}/discussions")))
        .respond_with(json_response(status, body))
        .mount(&server)
        .await;

    let error = client(&server)
        .create_discussion(&project(), 133, "x", None)
        .await
        .unwrap_err();

    match error {
        Error::Status {
            status: got,
            message,
        } => {
            assert_eq!(got, status);
            assert_eq!(message, expected);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn long_plain_bodies_are_truncated() {
    let server = MockServer::start().await;
    let html = format!("<html>{}</html>", "x".repeat(2000));
    Mock::given(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(500).set_body_string(html))
        .mount(&server)
        .await;

    let error = client(&server).current_user().await.unwrap_err();

    match error {
        Error::Status { status, message } => {
            assert_eq!(status, 500);
            assert_eq!(message.chars().count(), 301);
            assert!(message.starts_with("<html>"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn malformed_json_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v4/user"))
        .respond_with(json_response(200, r#"{"username":"no id"}"#))
        .mount(&server)
        .await;

    let error = client(&server).current_user().await.unwrap_err();

    assert!(matches!(error, Error::Decode(message) if message.starts_with("/user")));
}

#[tokio::test]
async fn foreign_redirects_are_not_followed() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v4/user"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://elsewhere.invalid/api/v4/user"),
        )
        .mount(&server)
        .await;

    let error = client(&server).current_user().await.unwrap_err();

    assert!(matches!(error, Error::Status { status: 302, .. }));
}
