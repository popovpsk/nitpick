use rstest::rstest;
use serde_json::{Value, json};

use super::rig::{completed, interrupted, text, thinking, tool_finished, tool_started};
use crate::acp::normalize::{Normalizer, outcome_from_stop};
use crate::{Event, Failure, FailureKind, Outcome};

fn running() -> Normalizer {
    let mut normalizer = Normalizer::default();
    normalizer.set_session("s");
    normalizer.begin_turn();
    normalizer
}

fn params(update: Value) -> Value {
    json!({"sessionId": "s", "update": update})
}

fn chunk(kind: &str, message: Option<&str>, body: &str) -> Value {
    let mut update = json!({"sessionUpdate": kind, "content": {"type": "text", "text": body}});
    if let Some(message) = message {
        update["messageId"] = json!(message);
    }
    params(update)
}

fn call(id: &str, kind: Option<&str>, status: &str, title: &str) -> Value {
    let mut update =
        json!({"sessionUpdate": "tool_call", "toolCallId": id, "status": status, "title": title});
    if let Some(kind) = kind {
        update["kind"] = json!(kind);
    }
    params(update)
}

fn change(id: &str, status: Option<&str>, title: Option<&str>) -> Value {
    let mut update = json!({"sessionUpdate": "tool_call_update", "toolCallId": id});
    if let Some(status) = status {
        update["status"] = json!(status);
    }
    if let Some(title) = title {
        update["title"] = json!(title);
    }
    params(update)
}

#[test]
fn nothing_is_read_while_no_turn_runs() {
    let mut normalizer = Normalizer::default();
    assert!(
        normalizer
            .feed(&chunk("agent_message_chunk", None, "late"))
            .is_empty()
    );
    normalizer.begin_turn();
    normalizer.finish(Outcome::Completed);
    assert!(
        normalizer
            .feed(&chunk("agent_message_chunk", None, "late"))
            .is_empty()
    );
    assert!(normalizer.finish(Outcome::Completed).is_empty());
}

#[test]
fn a_pending_call_starts_when_it_runs_and_ends_once() {
    let mut normalizer = running();
    assert!(
        normalizer
            .feed(&call("t", Some("execute"), "pending", "bash"))
            .is_empty()
    );
    assert_eq!(normalizer.open_tools(), 1);
    assert_eq!(
        normalizer.feed(&change("t", Some("in_progress"), Some("git log"))),
        vec![tool_started("t", "git log")]
    );
    for _ in 0..3 {
        assert!(
            normalizer
                .feed(&change("t", Some("in_progress"), Some("git log")))
                .is_empty()
        );
    }
    assert_eq!(
        normalizer.feed(&change("t", Some("completed"), None)),
        vec![tool_finished("t", false)]
    );
    assert!(
        normalizer
            .feed(&change("t", Some("completed"), None))
            .is_empty()
    );
    assert!(
        normalizer
            .feed(&change("t", Some("in_progress"), Some("again")))
            .is_empty()
    );
    assert_eq!(normalizer.open_tools(), 0);
}

#[test]
fn the_latest_title_wins_before_the_start() {
    let mut normalizer = running();
    normalizer.feed(&call("t", None, "pending", "bash"));
    normalizer.feed(&change("t", None, Some("cat lib.rs")));
    assert_eq!(
        normalizer.feed(&change("t", Some("in_progress"), None)),
        vec![tool_started("t", "cat lib.rs")]
    );
}

#[test]
fn a_failed_call_is_reported_failed() {
    let mut normalizer = running();
    normalizer.feed(&call("t", None, "in_progress", "make"));
    assert_eq!(
        normalizer.feed(&change("t", Some("failed"), None)),
        vec![tool_finished("t", true)]
    );
}

#[test]
fn a_call_that_arrives_running_starts_at_once() {
    let mut normalizer = running();
    assert_eq!(
        normalizer.feed(&call("t", Some("read"), "in_progress", "Read lib.rs")),
        vec![tool_started("t", "Read lib.rs")]
    );
}

#[test]
fn a_call_that_never_ran_before_it_finished_starts_and_ends() {
    let mut normalizer = running();
    normalizer.feed(&call("t", None, "pending", "bash"));
    assert_eq!(
        normalizer.feed(&change("t", Some("completed"), Some("ls"))),
        vec![tool_started("t", "ls"), tool_finished("t", false)]
    );
}

#[rstest]
#[case::running(Some("in_progress"), vec![tool_started("t", "ls")])]
#[case::pending(Some("pending"), vec![tool_started("t", "ls")])]
#[case::no_status(None, vec![tool_started("t", "ls")])]
#[case::completed(Some("completed"), vec![tool_started("t", "ls"), tool_finished("t", false)])]
#[case::failed(Some("failed"), vec![tool_started("t", "ls"), tool_finished("t", true)])]
fn an_update_without_a_call_is_the_start(
    #[case] status: Option<&str>,
    #[case] expected: Vec<Event>,
) {
    let mut normalizer = running();
    assert_eq!(normalizer.feed(&change("t", status, Some("ls"))), expected);
}

#[test]
fn a_repeated_call_is_not_a_second_start() {
    let mut normalizer = running();
    assert_eq!(
        normalizer.feed(&call("t", None, "in_progress", "ls")),
        vec![tool_started("t", "ls")]
    );
    assert!(
        normalizer
            .feed(&call("t", None, "in_progress", "ls"))
            .is_empty()
    );
}

#[rstest]
#[case::kind_stands_in("", Some("execute"), "execute")]
#[case::blank_title_too("   ", Some("read"), "read")]
#[case::nothing_at_all("", None, "tool")]
#[case::lines_are_joined("git log\n  --oneline\n", None, "git log --oneline")]
fn a_missing_title_is_replaced(
    #[case] title: &str,
    #[case] kind: Option<&str>,
    #[case] shown: &str,
) {
    let mut normalizer = running();
    assert_eq!(
        normalizer.feed(&call("t", kind, "in_progress", title)),
        vec![tool_started("t", shown)]
    );
}

#[test]
fn a_long_title_is_cut_on_one_line() {
    let mut normalizer = running();
    let long = format!("echo {}", "x".repeat(300));
    let events = normalizer.feed(&call("t", None, "in_progress", &long));
    let Some(Event::ToolStarted { title, .. }) = events.first() else {
        panic!("no start: {events:?}");
    };
    assert_eq!(title.chars().count(), 121);
    assert!(title.ends_with('…'));
}

#[test]
fn the_kind_is_remembered_from_the_call_and_updates() {
    let mut normalizer = running();
    normalizer.feed(&call("a", Some("edit"), "pending", "write"));
    normalizer.feed(&change("a", Some("in_progress"), None));
    assert_eq!(normalizer.kind_of("a"), Some("edit"));
    normalizer.feed(&params(
        json!({"sessionUpdate": "tool_call_update", "toolCallId": "b", "kind": "fetch"}),
    ));
    assert_eq!(normalizer.kind_of("b"), Some("fetch"));
    normalizer.feed(&call("c", None, "pending", "x"));
    assert_eq!(normalizer.kind_of("c"), None);
    assert_eq!(normalizer.kind_of("never"), None);
}

#[test]
fn open_started_tools_are_closed_as_failed_when_the_turn_ends() {
    let mut normalizer = running();
    normalizer.feed(&call("b", None, "in_progress", "second"));
    normalizer.feed(&call("a", None, "in_progress", "first"));
    normalizer.feed(&call("c", None, "pending", "never ran"));
    normalizer.feed(&call("d", None, "in_progress", "done"));
    normalizer.feed(&change("d", Some("completed"), None));
    assert_eq!(
        normalizer.finish(Outcome::Interrupted),
        vec![
            tool_finished("a", true),
            tool_finished("b", true),
            interrupted()
        ]
    );
    assert_eq!(normalizer.open_tools(), 0);
}

#[test]
fn text_and_thinking_stream_as_deltas() {
    let mut normalizer = running();
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m"), "Hel")),
        vec![text("Hel")]
    );
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m"), "lo")),
        vec![text("lo")]
    );
    assert_eq!(
        normalizer.feed(&chunk("agent_thought_chunk", Some("p"), "hm")),
        vec![thinking("hm")]
    );
}

#[test]
fn a_new_message_id_starts_a_new_paragraph() {
    let mut normalizer = running();
    normalizer.feed(&chunk("agent_message_chunk", Some("m1"), "one"));
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m2"), "two")),
        vec![text("\n\ntwo")]
    );
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m2"), " more")),
        vec![text(" more")]
    );
}

#[test]
fn thoughts_get_paragraphs_by_their_own_ids() {
    let mut normalizer = running();
    normalizer.feed(&chunk("agent_thought_chunk", Some("p1"), "one"));
    normalizer.feed(&chunk("agent_message_chunk", Some("m1"), "said"));
    assert_eq!(
        normalizer.feed(&chunk("agent_thought_chunk", Some("p2"), "two")),
        vec![thinking("\n\ntwo")]
    );
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m1"), " on")),
        vec![text(" on")]
    );
}

#[test]
fn text_without_ids_is_split_only_by_a_tool_call() {
    let mut normalizer = running();
    normalizer.feed(&chunk("agent_message_chunk", None, "one"));
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", None, " and")),
        vec![text(" and")]
    );
    normalizer.feed(&call("t", None, "in_progress", "ls"));
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", None, "two")),
        vec![text("\n\ntwo")]
    );
}

#[test]
fn the_first_text_of_a_turn_has_no_separator() {
    let mut normalizer = running();
    normalizer.feed(&call("t", None, "in_progress", "ls"));
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m"), "first")),
        vec![text("first")]
    );
    normalizer.finish(Outcome::Completed);
    normalizer.begin_turn();
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("other"), "next turn")),
        vec![text("next turn")]
    );
}

#[rstest]
#[case::underscore(params(json!({"sessionUpdate": "_vendor/thing", "content": {"type": "text", "text": "x"}})))]
#[case::unknown(params(json!({"sessionUpdate": "teleport"})))]
#[case::plan(params(json!({"sessionUpdate": "plan", "entries": []})))]
#[case::usage(params(json!({"sessionUpdate": "usage_update", "used": 1, "size": 2})))]
#[case::history(chunk("user_message_chunk", Some("u"), "replayed"))]
#[case::commands(params(json!({"sessionUpdate": "available_commands_update", "availableCommands": []})))]
#[case::image(params(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "image", "data": "x"}})))]
#[case::empty_text(chunk("agent_message_chunk", None, ""))]
#[case::no_content(params(json!({"sessionUpdate": "agent_message_chunk"})))]
#[case::no_update(json!({"sessionId": "s"}))]
#[case::mistyped(params(json!({"sessionUpdate": 7, "toolCallId": []})))]
#[case::call_without_id(params(json!({"sessionUpdate": "tool_call", "status": "in_progress"})))]
#[case::another_session(json!({"sessionId": "child", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}}))]
fn what_is_not_ours_is_ignored(#[case] message: Value) {
    let mut normalizer = running();
    assert!(normalizer.feed(&message).is_empty());
    assert_eq!(normalizer.open_tools(), 0);
}

#[rstest]
#[case::whole(&["The request was cancelled by the client."])]
#[case::split(&["The request", " was cancel", "led by the client."])]
#[case::other_spelling(&["the request was canceled by the client."])]
#[case::padded(&["\n", "The request was cancelled by the client.", "\n"])]
fn the_cancellation_note_is_not_text(#[case] chunks: &[&str]) {
    let mut normalizer = running();
    normalizer.mark_cancelled();
    for part in chunks {
        let events = normalizer.feed(&chunk("agent_message_chunk", Some("m"), part));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Event::Text(text) if text.contains("cancel"))),
            "{events:?}"
        );
    }
    assert_eq!(normalizer.finish(Outcome::Interrupted), vec![interrupted()]);
}

#[test]
fn other_text_after_a_cancel_is_released_whole() {
    let mut normalizer = running();
    normalizer.mark_cancelled();
    assert!(
        normalizer
            .feed(&chunk("agent_message_chunk", Some("m"), "The request"))
            .is_empty()
    );
    assert_eq!(
        normalizer.feed(&chunk(
            "agent_message_chunk",
            Some("m"),
            " is being stopped"
        )),
        vec![text("The request is being stopped")]
    );
    assert_eq!(
        normalizer.feed(&chunk("agent_message_chunk", Some("m"), " now")),
        vec![text(" now")]
    );
}

#[test]
fn the_note_is_text_when_no_cancel_was_asked() {
    let mut normalizer = running();
    assert_eq!(
        normalizer.feed(&chunk(
            "agent_message_chunk",
            None,
            "The request was cancelled by the client."
        )),
        vec![text("The request was cancelled by the client.")]
    );
}

#[rstest]
#[case::end_turn(Some("end_turn"), Outcome::Completed)]
#[case::max_tokens(Some("max_tokens"), Outcome::Completed)]
#[case::max_turn_requests(Some("max_turn_requests"), Outcome::Completed)]
#[case::cancelled(Some("cancelled"), Outcome::Interrupted)]
#[case::unknown(Some("something_new"), Outcome::Completed)]
#[case::missing(None, Outcome::Completed)]
fn the_stop_reason_is_the_outcome(#[case] reason: Option<&str>, #[case] expected: Outcome) {
    assert_eq!(outcome_from_stop(reason), expected);
}

#[test]
fn a_refusal_fails_the_turn() {
    let Outcome::Failed(Failure { kind, message }) = outcome_from_stop(Some("refusal")) else {
        panic!("a refusal is a failure");
    };
    assert_eq!(kind, FailureKind::Other);
    assert!(!message.is_empty());
}

#[test]
fn finishing_twice_ends_the_turn_once() {
    let mut normalizer = running();
    assert_eq!(normalizer.finish(Outcome::Completed), vec![completed()]);
    assert!(!normalizer.active());
    assert!(normalizer.finish(Outcome::Completed).is_empty());
}
