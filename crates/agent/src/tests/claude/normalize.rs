use rstest::rstest;
use serde_json::{Value, json};

use crate::claude::normalize::{Normalizer, looks_like_result, tool_title};
use crate::{Event, Failure, FailureKind, Outcome};

fn feed_all(normalizer: &mut Normalizer, lines: &[&str]) -> Vec<Event> {
    lines
        .iter()
        .flat_map(|line| normalizer.feed(line).events)
        .collect()
}

fn run(lines: &[&str]) -> Vec<Event> {
    feed_all(&mut Normalizer::default(), lines)
}

fn last_outcome(events: &[Event]) -> Outcome {
    match events.last() {
        Some(Event::TurnFinished(outcome)) => outcome.clone(),
        other => panic!("the last event is not a turn end: {other:?}"),
    }
}

fn delta(kind: &str, field: &str, text: &str) -> String {
    json!({"type": "stream_event", "event": {"type": "content_block_delta",
        "delta": {"type": kind, field: text}}, "parent_tool_use_id": null})
    .to_string()
}

fn message_start() -> String {
    json!({"type": "stream_event", "event": {"type": "message_start"}}).to_string()
}

fn assistant(blocks: Value) -> String {
    json!({"type": "assistant", "message": {"content": blocks}, "parent_tool_use_id": null})
        .to_string()
}

fn tool_use(id: &str, name: &str, input: Value) -> String {
    assistant(json!([{"type": "tool_use", "id": id, "name": name, "input": input}]))
}

fn tool_result(id: &str, is_error: bool) -> String {
    json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": id, "is_error": is_error, "content": ""}]}})
    .to_string()
}

#[test]
fn text_arrives_once_when_it_is_streamed_and_repeated_in_the_assistant_frame() {
    let events = run(&[
        &message_start(),
        &delta("text_delta", "text", "Hel"),
        &delta("text_delta", "text", "lo"),
        &assistant(json!([{"type": "text", "text": "Hello"}])),
    ]);
    assert_eq!(
        events,
        vec![Event::Text("Hel".into()), Event::Text("lo".into())]
    );
}

#[test]
fn text_without_streaming_comes_from_the_assistant_frame() {
    let events = run(&[
        &message_start(),
        &assistant(json!([{"type": "text", "text": "Hello"}])),
        &message_start(),
        &assistant(json!([{"type": "text", "text": "Again"}])),
    ]);
    assert_eq!(
        events,
        vec![Event::Text("Hello".into()), Event::Text("Again".into())]
    );
}

#[test]
fn streaming_in_one_message_does_not_hide_the_next_message() {
    let events = run(&[
        &message_start(),
        &delta("text_delta", "text", "one"),
        &assistant(json!([{"type": "text", "text": "one"}])),
        &message_start(),
        &assistant(json!([{"type": "text", "text": "two"}])),
    ]);
    assert_eq!(
        events,
        vec![Event::Text("one".into()), Event::Text("two".into())]
    );
}

#[test]
fn thinking_text_is_forwarded_and_empty_thinking_is_not() {
    let events = run(&[
        &message_start(),
        &delta("thinking_delta", "thinking", ""),
        &delta("thinking_delta", "thinking", "hmm"),
        &assistant(json!([{"type": "thinking", "thinking": "hmm"}])),
        &message_start(),
        &assistant(json!([{"type": "thinking", "thinking": ""}])),
    ]);
    assert_eq!(events, vec![Event::Thinking("hmm".into())]);
}

#[test]
fn a_repeated_init_reports_the_session_once() {
    let init =
        |id: &str| json!({"type": "system", "subtype": "init", "session_id": id}).to_string();
    let mut normalizer = Normalizer::default();
    assert_eq!(
        feed_all(&mut normalizer, &[&init("a"), &init("a")]),
        vec![Event::Session { id: "a".into() }]
    );
    assert_eq!(
        feed_all(&mut normalizer, &[&init("b")]),
        vec![Event::Session { id: "b".into() }]
    );
}

#[test]
fn unknown_system_subtypes_and_frames_are_ignored() {
    let events = run(&[
        r#"{"type":"system","subtype":"status","status":"requesting"}"#,
        r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":50}"#,
        r#"{"type":"something_new","payload":[1,2,3]}"#,
        r#"{"no_type":true}"#,
        r#"{"type":"control_response","response":{"subtype":"success"}}"#,
        "not json at all",
        "",
        "[1,2]",
        r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"thinking","thinking":""}}}"#,
    ]);
    assert!(events.is_empty());
}

#[test]
fn frames_with_odd_field_types_still_parse() {
    let events = run(&[
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}},"parent_tool_use_id":7}"#,
        r#"{"type":"assistant","message":{"content":"plain"},"error":5}"#,
        r#"{"type":"result","subtype":3,"is_error":"yes","errors":"x","terminal_reason":[]}"#,
    ]);
    assert_eq!(
        events,
        vec![
            Event::Text("ok".into()),
            Event::TurnFinished(Outcome::Completed)
        ]
    );
}

#[test]
fn subagent_frames_are_ignored() {
    let sidechain = json!({"type": "stream_event", "parent_tool_use_id": "toolu_1",
        "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "x"}}})
    .to_string();
    let assistant = json!({"type": "assistant", "parent_tool_use_id": "toolu_1",
        "message": {"content": [{"type": "tool_use", "id": "t", "name": "Bash",
        "input": {"command": "ls"}}]}})
    .to_string();
    assert!(run(&[&sidechain, &assistant]).is_empty());
}

#[test]
fn a_tool_starts_once_and_finishes_with_its_error_flag() {
    let call = tool_use("t1", "Bash", json!({"command": "git log"}));
    let events = run(&[
        &call,
        &call,
        &tool_result("t1", true),
        &tool_result("t1", false),
        &tool_result("unknown", false),
    ]);
    assert_eq!(
        events,
        vec![
            Event::ToolStarted {
                id: "t1".into(),
                title: "git log".into()
            },
            Event::ToolFinished {
                id: "t1".into(),
                failed: true
            },
        ]
    );
}

#[test]
fn a_tool_left_open_is_closed_as_failed_when_the_turn_ends() {
    let events = run(&[
        &tool_use("t1", "Read", json!({"file_path": "a.rs"})),
        r#"{"type":"result","subtype":"success","is_error":false}"#,
    ]);
    assert_eq!(
        events,
        vec![
            Event::ToolStarted {
                id: "t1".into(),
                title: "a.rs".into()
            },
            Event::ToolFinished {
                id: "t1".into(),
                failed: true
            },
            Event::TurnFinished(Outcome::Completed),
        ]
    );
}

#[rstest]
#[case::bash("Bash", json!({"command": "git log --oneline", "description": "x"}), "git log --oneline")]
#[case::bash_lines("Bash", json!({"command": "git log\n  | head   -5\n"}), "git log | head -5")]
#[case::read("Read", json!({"file_path": "/work/toy/src/lib.rs"}), "/work/toy/src/lib.rs")]
#[case::read_path("Read", json!({"path": "src/lib.rs"}), "src/lib.rs")]
#[case::grep("Grep", json!({"pattern": "fn main", "path": "src"}), "fn main")]
#[case::glob("Glob", json!({"pattern": "**/*.rs"}), "**/*.rs")]
#[case::unknown_tool("Other", json!({"command": "x"}), "Other")]
#[case::missing_field("Bash", json!({}), "Bash")]
#[case::blank_field("Bash", json!({"command": "  \n "}), "Bash")]
#[case::wrong_type("Bash", json!({"command": 3}), "Bash")]
fn titles_name_what_the_tool_works_on(
    #[case] name: &str,
    #[case] input: Value,
    #[case] expected: &str,
) {
    assert_eq!(tool_title(name, Some(&input)), expected);
}

#[test]
fn a_tool_without_input_is_titled_by_its_name() {
    assert_eq!(tool_title("Bash", None), "Bash");
}

#[test]
fn long_titles_are_cut_to_one_short_line() {
    let command = "echo ".to_owned() + &"x".repeat(500);
    let title = tool_title("Bash", Some(&json!({"command": command})));
    assert_eq!(title.chars().count(), 121);
    assert!(title.ends_with('…'));
    assert!(!title.contains('\n'));
}

#[test]
fn an_api_retry_becomes_a_notice() {
    let events = run(&[
        r#"{"type":"system","subtype":"api_retry","attempt":2,"max_retries":10,"error":"rate_limit"}"#,
        r#"{"type":"system","subtype":"api_retry"}"#,
    ]);
    assert_eq!(
        events,
        vec![
            Event::Notice("Retrying the request (attempt 2 of 10): rate_limit".into()),
            Event::Notice("Retrying the request (attempt 0)".into()),
        ]
    );
}

#[test]
fn any_tool_permission_request_is_denied() {
    let output = Normalizer::default().feed(
        r#"{"type":"control_request","request_id":"r-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf x"}}}"#,
    );
    assert!(output.events.is_empty());
    assert!(!output.finished);
    let reply: Value = serde_json::from_str(&output.reply.expect("a reply")).unwrap();
    assert_eq!(reply["type"], "control_response");
    assert_eq!(reply["response"]["subtype"], "success");
    assert_eq!(reply["response"]["request_id"], "r-1");
    assert_eq!(reply["response"]["response"]["behavior"], "deny");
}

#[test]
fn other_control_requests_are_answered_with_an_error() {
    let output = Normalizer::default().feed(
        r#"{"type":"control_request","request_id":"r-2","request":{"subtype":"hook_callback"}}"#,
    );
    let reply: Value = serde_json::from_str(&output.reply.expect("a reply")).unwrap();
    assert_eq!(reply["response"]["subtype"], "error");
    assert_eq!(reply["response"]["request_id"], "r-2");
}

#[rstest]
#[case::success(
    r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
    Outcome::Completed
)]
#[case::no_flags(r#"{"type":"result"}"#, Outcome::Completed)]
#[case::aborted_tools(
    r#"{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_tools","result":null}"#,
    Outcome::Interrupted
)]
#[case::aborted_streaming(
    r#"{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming"}"#,
    Outcome::Interrupted
)]
fn a_result_ends_the_turn(#[case] line: &str, #[case] expected: Outcome) {
    assert_eq!(last_outcome(&run(&[line])), expected);
}

#[rstest]
#[case::auth_code(
    &[
        r#"{"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"Invalid API key · Please run /login"}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"Invalid API key · Please run /login"}"#,
    ],
    FailureKind::NotLoggedIn,
    "Invalid API key · Please run /login"
)]
#[case::auth_status(
    &[r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"oops"}"#],
    FailureKind::NotLoggedIn,
    "oops"
)]
#[case::not_logged_in_text(
    &[r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#],
    FailureKind::NotLoggedIn,
    "Not logged in · Please run /login"
)]
#[case::rate_limit_status(
    &[r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"slow down"}"#],
    FailureKind::UsageLimit,
    "slow down"
)]
#[case::rate_limit_code(
    &[
        r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"You've hit your limit"}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":null}"#,
    ],
    FailureKind::UsageLimit,
    "You've hit your limit"
)]
#[case::rejected_limit_event(
    &[
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","overageStatus":"rejected","isUsingOverage":false}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":"limit reached"}"#,
    ],
    FailureKind::UsageLimit,
    "limit reached"
)]
#[case::overage_is_not_a_limit(
    &[
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","isUsingOverage":true}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":"something else"}"#,
    ],
    FailureKind::Other,
    "something else"
)]
#[case::allowed_with_rejected_overage(
    &[
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","overageStatus":"rejected","isUsingOverage":false}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":"network down"}"#,
    ],
    FailureKind::Other,
    "network down"
)]
#[case::prompt_too_long_reason(
    &[r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"prompt_too_long","result":"x"}"#],
    FailureKind::ContextTooLong,
    "x"
)]
#[case::blocking_limit_reason(
    &[r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"blocking_limit","result":"y"}"#],
    FailureKind::ContextTooLong,
    "y"
)]
#[case::prompt_too_long_text(
    &[r#"{"type":"result","subtype":"success","is_error":true,"result":"Prompt is too long"}"#],
    FailureKind::ContextTooLong,
    "Prompt is too long"
)]
#[case::model_not_found(
    &[
        r#"{"type":"assistant","error":"model_not_found","message":{"content":[{"type":"text","text":"There's an issue with the selected model (nope)."}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":"There's an issue with the selected model (nope)."}"#,
    ],
    FailureKind::ModelNotFound,
    "There's an issue with the selected model (nope)."
)]
#[case::session_lost(
    &[r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"errors":["No conversation found with session ID: abc"]}"#],
    FailureKind::SessionLost,
    "No conversation found with session ID: abc"
)]
#[case::error_flag_on_success(
    &[r#"{"type":"result","subtype":"success","is_error":true,"result":"boom"}"#],
    FailureKind::Other,
    "boom"
)]
#[case::error_subtype_without_flag(
    &[r#"{"type":"result","subtype":"error_max_turns"}"#],
    FailureKind::Other,
    "The assistant stopped with an error (error_max_turns)."
)]
#[case::flag_without_text(
    &[r#"{"type":"result","is_error":true}"#],
    FailureKind::Other,
    "The assistant stopped with an error."
)]
fn failures_are_classified_from_the_codes_not_only_the_text(
    #[case] lines: &[&str],
    #[case] kind: FailureKind,
    #[case] message: &str,
) {
    let events = run(lines);
    assert_eq!(
        last_outcome(&events),
        Outcome::Failed(Failure {
            kind,
            message: message.into()
        })
    );
    assert!(
        !events.iter().any(|event| matches!(event, Event::Text(_))),
        "the error text is not shown as an answer: {events:?}"
    );
}

#[test]
fn the_failure_context_does_not_leak_into_the_next_turn() {
    let mut normalizer = Normalizer::default();
    feed_all(
        &mut normalizer,
        &[
            r#"{"type":"assistant","error":"rate_limit","message":{"content":[]}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected"}}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"limit"}"#,
        ],
    );
    let events = feed_all(
        &mut normalizer,
        &[r#"{"type":"result","subtype":"success","is_error":true,"result":"other"}"#],
    );
    assert_eq!(
        last_outcome(&events),
        Outcome::Failed(Failure {
            kind: FailureKind::Other,
            message: "other".into()
        })
    );
}

#[rstest]
#[case::compact(r#"{"type":"result","subtype":"succ"#, true)]
#[case::spaced(
    r#"{"type": "result", "subtype": "success", "result": "unterminated"#,
    true
)]
#[case::other_frame(r#"{"type":"assistant","message":{"content":[{"type":"te"#, false)]
fn a_broken_result_line_still_ends_the_turn(#[case] line: &str, #[case] ends: bool) {
    assert_eq!(looks_like_result(line), ends);
    let output = Normalizer::default().feed(line);
    assert_eq!(output.finished, ends);
    if ends {
        match last_outcome(&output.events) {
            Outcome::Failed(failure) => assert_eq!(failure.kind, FailureKind::Other),
            other => panic!("unexpected outcome {other:?}"),
        }
    } else {
        assert!(output.events.is_empty());
    }
}

#[test]
fn aborting_closes_open_tools_and_fails_the_turn() {
    let mut normalizer = Normalizer::default();
    feed_all(
        &mut normalizer,
        &[&tool_use("t1", "Bash", json!({"command": "ls"}))],
    );
    let output = normalizer.abort(Failure {
        kind: FailureKind::Other,
        message: "gone".into(),
    });
    assert!(output.finished);
    assert_eq!(
        output.events,
        vec![
            Event::ToolFinished {
                id: "t1".into(),
                failed: true
            },
            Event::TurnFinished(Outcome::Failed(Failure {
                kind: FailureKind::Other,
                message: "gone".into()
            })),
        ]
    );
}
