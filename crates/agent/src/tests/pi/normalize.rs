use rstest::rstest;
use serde_json::{Value, json};

use crate::pi::normalize::{Normalizer, STATE_ID, tool_title};
use crate::{Event, Failure, FailureKind, Outcome};

fn begin() -> Normalizer {
    let mut normalizer = Normalizer::default();
    normalizer.begin_turn("p1");
    normalizer
}

fn run(normalizer: &mut Normalizer, frames: &[Value]) -> Vec<Event> {
    frames
        .iter()
        .flat_map(|frame| normalizer.feed(&frame.to_string()).events)
        .collect()
}

fn update(kind: &str, delta: &str) -> Value {
    json!({"type": "message_update", "assistantMessageEvent": {"type": kind, "contentIndex": 0, "delta": delta}})
}

fn assistant_end(stop: &str, error: Option<&str>, content: Value) -> Value {
    let mut message = json!({"role": "assistant", "stopReason": stop, "content": content});
    if let Some(error) = error {
        message["errorMessage"] = json!(error);
    }
    json!({"type": "message_end", "message": message})
}

fn settled(aborted: bool) -> Value {
    json!({"type": "agent_settled", "aborted": aborted})
}

fn outcome_of(events: &[Event]) -> Outcome {
    match events.last() {
        Some(Event::TurnFinished(outcome)) => outcome.clone(),
        other => panic!("the last event is not a turn end: {other:?}"),
    }
}

#[test]
fn deltas_become_text_and_thinking_and_empty_ones_are_dropped() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            update("thinking_delta", "hm"),
            update("thinking_delta", ""),
            update("text_delta", "Hello"),
            update("text_delta", ""),
            update("toolcall_delta", "{"),
            update("text_end", "ignored"),
        ],
    );
    assert_eq!(
        events,
        vec![Event::Thinking("hm".into()), Event::Text("Hello".into())]
    );
}

#[test]
fn nothing_is_emitted_while_no_turn_runs() {
    let mut normalizer = Normalizer::default();
    let events = run(
        &mut normalizer,
        &[
            update("text_delta", "stray"),
            json!({"type": "tool_execution_start", "toolCallId": "t", "toolName": "bash", "args": {}}),
            settled(false),
        ],
    );
    assert!(events.is_empty());
    assert!(!normalizer.active());
}

#[test]
fn agent_end_does_not_end_the_turn_but_agent_settled_does() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            assistant_end("stop", None, json!([])),
            json!({"type": "agent_end", "messages": [], "willRetry": false}),
        ],
    );
    assert!(events.is_empty());
    assert!(normalizer.active());
    let events = run(&mut normalizer, &[settled(false)]);
    assert_eq!(outcome_of(&events), Outcome::Completed);
    assert!(!normalizer.active());
}

#[test]
fn whole_text_is_used_only_when_no_delta_streamed() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            json!({"type": "message_start", "message": {"role": "assistant"}}),
            update("text_delta", "streamed"),
            assistant_end("stop", None, json!([{"type": "text", "text": "streamed"}])),
            json!({"type": "message_start", "message": {"role": "assistant"}}),
            assistant_end(
                "stop",
                None,
                json!([{"type": "thinking", "thinking": "plan"}, {"type": "text", "text": "whole"}]),
            ),
        ],
    );
    assert_eq!(
        events,
        vec![
            Event::Text("streamed".into()),
            Event::Thinking("plan".into()),
            Event::Text("whole".into()),
        ]
    );
}

#[test]
fn an_errored_message_never_contributes_its_text() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[assistant_end(
            "error",
            Some("boom"),
            json!([{"type": "text", "text": "partial"}]),
        )],
    );
    assert!(events.is_empty());
}

#[test]
fn user_and_tool_result_messages_are_ignored() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            json!({"type": "message_end", "message": {"role": "user", "content": "hi"}}),
            json!({"type": "message_end", "message": {"role": "toolResult", "content": [{"type": "text", "text": "out"}]}}),
        ],
    );
    assert!(events.is_empty());
}

#[test]
fn a_tool_runs_from_start_to_end_once() {
    let mut normalizer = begin();
    let start = json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "bash", "args": {"command": "git log"}});
    let events = run(
        &mut normalizer,
        &[
            start.clone(),
            start,
            json!({"type": "tool_execution_update", "toolCallId": "c1", "partialResult": {}}),
            json!({"type": "tool_execution_end", "toolCallId": "c1", "isError": true}),
            json!({"type": "tool_execution_end", "toolCallId": "c1", "isError": false}),
            json!({"type": "tool_execution_end", "toolCallId": "never-started", "isError": false}),
        ],
    );
    assert_eq!(
        events,
        vec![
            Event::ToolStarted {
                id: "c1".into(),
                title: "git log".into()
            },
            Event::ToolFinished {
                id: "c1".into(),
                failed: true
            },
        ]
    );
}

#[test]
fn open_tools_are_closed_as_failed_when_the_turn_ends() {
    let mut normalizer = begin();
    run(
        &mut normalizer,
        &[
            json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "read", "args": {"path": "a.rs"}}),
        ],
    );
    let events = normalizer.end(Outcome::Interrupted).events;
    assert_eq!(
        events,
        vec![
            Event::ToolFinished {
                id: "c1".into(),
                failed: true
            },
            Event::TurnFinished(Outcome::Interrupted),
        ]
    );
}

#[test]
fn a_retry_start_is_a_notice() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            json!({"type": "auto_retry_start", "attempt": 2, "maxAttempts": 3, "delayMs": 4000, "errorMessage": "overloaded\nplease wait"}),
        ],
    );
    assert_eq!(
        events,
        vec![Event::Notice(
            "Retrying the request (attempt 2 of 3): overloaded please wait".into()
        )]
    );
}

#[test]
fn an_error_followed_by_a_good_message_is_not_a_failure() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            assistant_end("error", Some("temporary"), json!([])),
            json!({"type": "agent_end", "willRetry": true}),
            json!({"type": "auto_retry_end", "success": true, "attempt": 1}),
            assistant_end("stop", None, json!([])),
            settled(false),
        ],
    );
    assert_eq!(outcome_of(&events), Outcome::Completed);
}

#[test]
fn an_error_that_sticks_fails_with_its_classification() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            assistant_end("error", Some("429 rate limit"), json!([])),
            settled(false),
        ],
    );
    assert_eq!(
        outcome_of(&events),
        Outcome::Failed(Failure {
            kind: FailureKind::UsageLimit,
            message: "429 rate limit".into()
        })
    );
}

#[test]
fn an_error_without_text_gets_a_generic_message() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[assistant_end("error", None, json!([])), settled(false)],
    );
    assert_eq!(
        outcome_of(&events),
        Outcome::Failed(Failure {
            kind: FailureKind::Other,
            message: "The assistant stopped with an error.".into()
        })
    );
}

#[test]
fn a_final_retry_error_stands_in_when_no_message_carried_one() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[
            json!({"type": "auto_retry_end", "success": false, "attempt": 3, "finalError": "No API key found"}),
            settled(false),
        ],
    );
    assert_eq!(
        outcome_of(&events),
        Outcome::Failed(Failure {
            kind: FailureKind::NotLoggedIn,
            message: "No API key found".into()
        })
    );
}

#[rstest]
#[case::settled_flag(json!([]), true)]
#[case::aborted_message(json!([assistant_end("aborted", None, json!([]))]), false)]
fn an_abort_is_an_interruption_even_after_an_error(#[case] before: Value, #[case] flag: bool) {
    let mut normalizer = begin();
    let mut frames = vec![assistant_end("error", Some("boom"), json!([]))];
    frames.extend(before.as_array().unwrap().iter().cloned());
    frames.push(settled(flag));
    let events = run(&mut normalizer, &frames);
    assert_eq!(outcome_of(&events), Outcome::Interrupted);
}

#[test]
fn the_length_stop_reason_is_a_completed_turn() {
    let mut normalizer = begin();
    let events = run(
        &mut normalizer,
        &[assistant_end("length", None, json!([])), settled(false)],
    );
    assert_eq!(outcome_of(&events), Outcome::Completed);
}

#[test]
fn a_rejected_prompt_fails_the_turn_at_once() {
    let mut normalizer = begin();
    let out = normalizer.feed(
        &json!({"id": "p1", "type": "response", "command": "prompt", "success": false, "error": "No API key found for openai"})
            .to_string(),
    );
    assert!(out.finished);
    assert_eq!(
        outcome_of(&out.events),
        Outcome::Failed(Failure {
            kind: FailureKind::NotLoggedIn,
            message: "No API key found for openai".into()
        })
    );
}

#[test]
fn a_handled_prompt_completes_without_waiting_for_a_run() {
    let mut normalizer = begin();
    let out = normalizer.feed(
        &json!({"id": "p1", "type": "response", "command": "prompt", "success": true, "data": {"disposition": "handled"}})
            .to_string(),
    );
    assert!(out.finished);
    assert_eq!(outcome_of(&out.events), Outcome::Completed);
}

#[rstest]
#[case::started(json!({"disposition": "started"}))]
#[case::queued(json!({"disposition": "queued"}))]
#[case::none(Value::Null)]
fn an_accepted_prompt_keeps_the_turn_open(#[case] data: Value) {
    let mut normalizer = begin();
    let out = normalizer.feed(
        &json!({"id": "p1", "type": "response", "command": "prompt", "success": true, "data": data})
            .to_string(),
    );
    assert!(!out.finished);
    assert!(out.events.is_empty());
    assert!(normalizer.active());
}

#[test]
fn a_response_to_another_prompt_is_ignored() {
    let mut normalizer = begin();
    let out = normalizer.feed(
        &json!({"id": "old", "type": "response", "command": "prompt", "success": false, "error": "x"})
            .to_string(),
    );
    assert!(!out.finished);
    assert!(normalizer.active());
}

#[test]
fn the_state_answer_is_read_with_or_without_a_turn() {
    let mut normalizer = Normalizer::default();
    let out = normalizer.feed(
        &json!({"id": STATE_ID, "type": "response", "command": "get_state", "success": true,
            "data": {"sessionId": "nitpick-1", "messageCount": 4, "isStreaming": false}})
        .to_string(),
    );
    let state = out.state.unwrap().unwrap();
    assert_eq!(state.session_id.as_deref(), Some("nitpick-1"));
    assert_eq!(state.message_count, Some(4));

    let out = normalizer.feed(
        &json!({"id": STATE_ID, "type": "response", "command": "get_state", "success": false, "error": "boom"})
            .to_string(),
    );
    assert_eq!(out.state.unwrap().unwrap_err().message, "boom");

    let out = normalizer.feed(
        &json!({"id": "other", "type": "response", "command": "get_state", "success": true, "data": {}})
            .to_string(),
    );
    assert!(out.state.is_none());
}

#[rstest]
#[case::select("select", true)]
#[case::confirm("confirm", true)]
#[case::input("input", true)]
#[case::editor("editor", true)]
#[case::unknown_future_method("dialog2", true)]
#[case::notify("notify", false)]
#[case::status("setStatus", false)]
#[case::widget("setWidget", false)]
#[case::title("setTitle", false)]
#[case::editor_text("set_editor_text", false)]
fn dialogs_are_cancelled_and_notifications_are_not_answered(
    #[case] method: &str,
    #[case] answered: bool,
) {
    let mut normalizer = Normalizer::default();
    let out = normalizer.feed(
        &json!({"type": "extension_ui_request", "id": "ui-1", "method": method, "title": "t"})
            .to_string(),
    );
    match (answered, out.reply) {
        (true, Some(reply)) => {
            let reply: Value = serde_json::from_str(&reply).unwrap();
            assert_eq!(
                reply,
                json!({"type": "extension_ui_response", "id": "ui-1", "cancelled": true})
            );
        }
        (false, None) => {}
        (_, other) => panic!("unexpected reply {other:?}"),
    }
}

#[test]
fn a_dialog_without_an_id_cannot_be_answered() {
    let mut normalizer = Normalizer::default();
    let out =
        normalizer.feed(&json!({"type": "extension_ui_request", "method": "select"}).to_string());
    assert!(out.reply.is_none());
}

#[rstest]
#[case::garbage("not json")]
#[case::empty("")]
#[case::array("[1, 2]")]
#[case::number("5")]
#[case::unknown_type(r#"{"type":"queue_update","steering":[]}"#)]
#[case::no_type(r#"{"hello":1}"#)]
#[case::mistyped_fields(r#"{"type":"message_update","assistantMessageEvent":"nope","message":7}"#)]
fn unreadable_and_unknown_lines_change_nothing(#[case] line: &str) {
    let mut normalizer = begin();
    let out = normalizer.feed(line);
    assert!(out.events.is_empty());
    assert!(out.reply.is_none());
    assert!(!out.finished);
    assert!(normalizer.active());
}

#[rstest]
#[case::bash("bash", json!({"command": "git log --oneline"}), "git log --oneline")]
#[case::bash_multiline("bash", json!({"command": "git log\n  | head"}), "git log | head")]
#[case::read("read", json!({"path": "src/x.rs"}), "src/x.rs")]
#[case::grep("grep", json!({"pattern": "TODO", "path": "src"}), "TODO")]
#[case::find("find", json!({"pattern": "*.rs"}), "*.rs")]
#[case::ls("ls", json!({"path": "src"}), "src")]
#[case::ls_without_args("ls", json!({}), "ls")]
#[case::unknown_tool_with_a_path("fancy", json!({"path": "a"}), "a")]
#[case::unknown_tool("fancy", json!({"x": 1}), "fancy")]
#[case::mistyped_command("bash", json!({"command": 5}), "bash")]
fn tool_titles(#[case] name: &str, #[case] args: Value, #[case] expected: &str) {
    assert_eq!(tool_title(name, Some(&args)), expected);
}

#[test]
fn a_tool_without_args_is_titled_by_its_name() {
    assert_eq!(tool_title("bash", None), "bash");
}

#[test]
fn a_long_title_is_cut() {
    let command = "x".repeat(300);
    let title = tool_title("bash", Some(&json!({"command": command})));
    assert_eq!(title.chars().count(), 121);
    assert!(title.ends_with('…'));
}
