use rstest::rstest;
use serde_json::{Value, json};

use crate::codex::normalize::{Normalizer, shown_command, tool_title};
use crate::codex::wire::{ErrorInfo, Item};
use crate::{Event, Failure, FailureKind, Outcome};

fn item(value: Value) -> Item {
    serde_json::from_value(value).unwrap()
}

fn normalizer() -> Normalizer {
    let mut normalizer = Normalizer::default();
    normalizer.set_thread("t");
    normalizer.begin_turn();
    normalizer
}

#[rstest]
#[case::zsh("/usr/bin/zsh -lc 'git log --oneline'", "git log --oneline")]
#[case::bash("/bin/bash -lc 'echo '\\''hi'\\'' && ls'", "echo 'hi' && ls")]
#[case::double("bash -c \"cargo watch\"", "cargo watch")]
#[case::sh_ic("sh -ic 'rg foo'", "rg foo")]
#[case::bare("echo beta", "echo beta")]
#[case::not_a_shell_flag("bash script.sh", "bash script.sh")]
#[case::other_program("python -c 'print(1)'", "python -c 'print(1)'")]
#[case::unquoted_body("zsh -lc ls", "ls")]
#[case::empty("", "")]
#[case::spaces("   ", "")]
fn the_shell_wrapper_is_dropped(#[case] command: &str, #[case] shown: &str) {
    assert_eq!(shown_command(command), shown);
}

#[test]
fn the_title_comes_from_the_command_actions() {
    let title = tool_title(&item(json!({
        "type": "commandExecution",
        "command": "/usr/bin/zsh -lc 'cat src/a.rs | head'",
        "commandActions": [
            {"type": "read", "command": "cat src/a.rs", "name": "a.rs"},
            {"type": "read", "command": "cat src/a.rs", "name": "a.rs"},
            {"type": "unknown", "command": "  "},
            {"type": "search", "command": "rg todo"},
            {"type": "unknown"},
        ],
    })));
    assert_eq!(title, "cat src/a.rs && rg todo");
}

#[rstest]
#[case::no_actions_key(json!({"command": "/bin/zsh -lc 'git status'"}), "git status")]
#[case::empty_actions(json!({"command": "/bin/zsh -lc 'git status'", "commandActions": []}), "git status")]
#[case::bad_actions(json!({"command": "ls -la", "commandActions": "oops"}), "ls -la")]
#[case::nothing(json!({}), "command")]
fn the_title_falls_back_to_the_command(#[case] value: Value, #[case] title: &str) {
    assert_eq!(tool_title(&item(value)), title);
}

#[test]
fn a_long_title_is_cut_on_one_line() {
    let long = format!("echo {}\n{}", "x".repeat(200), "y");
    let title = tool_title(&item(json!({"command": long})));
    assert_eq!(title.chars().count(), 121);
    assert!(title.ends_with('…') && !title.contains('\n'));
}

#[test]
fn exit_codes_and_statuses_decide_whether_a_tool_failed() {
    let finished = |status: &str, exit: Value| {
        let mut normalizer = normalizer();
        normalizer.feed(
            "item/started",
            &json!({"threadId": "t", "item": {"type": "commandExecution", "id": "a", "command": "x"}}),
        );
        let out = normalizer.feed(
            "item/completed",
            &json!({"threadId": "t", "item": {"type": "commandExecution", "id": "a", "status": status, "exit_code": exit}}),
        );
        out.events.into_iter().find_map(|event| match event {
            Event::ToolFinished { failed, .. } => Some(failed),
            _ => None,
        })
    };
    assert_eq!(finished("completed", json!(0)), Some(false));
    assert_eq!(finished("completed", Value::Null), Some(false));
    assert_eq!(finished("completed", json!(1)), Some(true));
    assert_eq!(finished("failed", Value::Null), Some(true));
    assert_eq!(finished("declined", Value::Null), Some(true));
}

#[test]
fn the_snake_case_spellings_are_understood() {
    let mut normalizer = normalizer();
    let started = normalizer.feed(
        "item/started",
        &json!({"item": {"type": "command_execution", "id": "a", "command": "ls", "command_actions": [{"command": "ls"}]}}),
    );
    assert_eq!(
        started.events,
        vec![Event::ToolStarted {
            id: "a".into(),
            title: "ls".into()
        }]
    );
    let message = normalizer.feed(
        "item/completed",
        &json!({"item": {"type": "agent_message", "id": "m", "text": "hello"}}),
    );
    assert_eq!(message.events, vec![Event::Text("hello".into())]);
}

#[test]
fn nothing_is_emitted_when_no_turn_is_running() {
    let mut normalizer = Normalizer::default();
    let out = normalizer.feed(
        "item/agentMessage/delta",
        &json!({"itemId": "m", "delta": "stale"}),
    );
    assert!(out.events.is_empty() && !out.finished);
    let out = normalizer.feed("turn/completed", &json!({"turn": {"status": "completed"}}));
    assert!(out.events.is_empty() && !out.finished);
}

#[test]
fn a_turn_that_is_aborted_closes_open_tools_and_resets() {
    let mut normalizer = normalizer();
    normalizer.feed(
        "item/started",
        &json!({"threadId": "t", "item": {"type": "commandExecution", "id": "a", "command": "ls"}}),
    );
    let failure = Failure {
        kind: FailureKind::Other,
        message: "gone".into(),
    };
    let out = normalizer.abort(Outcome::Failed(failure.clone()));
    assert_eq!(
        out.events,
        vec![
            Event::ToolFinished {
                id: "a".into(),
                failed: true
            },
            Event::TurnFinished(Outcome::Failed(failure))
        ]
    );
    assert!(out.finished && !normalizer.active());
}

#[rstest]
#[case::completed("completed", None, Outcome::Completed)]
#[case::interrupted("interrupted", None, Outcome::Interrupted)]
#[case::interrupted_with_error("interrupted", Some("x"), Outcome::Interrupted)]
#[case::completed_with_error("completed", Some("x"), Outcome::Completed)]
fn the_turn_status_decides_the_outcome(
    #[case] status: &str,
    #[case] error: Option<&str>,
    #[case] expected: Outcome,
) {
    let mut normalizer = normalizer();
    let out = normalizer.feed(
        "turn/completed",
        &json!({"threadId": "t", "turn": {"status": status, "error": error.map(|message| json!({"message": message}))}}),
    );
    assert_eq!(out.events, vec![Event::TurnFinished(expected)]);
}

#[rstest]
#[case::no_status_no_error(json!({"turn": {}}), Outcome::Completed)]
#[case::no_turn(json!({}), Outcome::Completed)]
#[case::error_only(
    json!({"turn": {"error": {"message": "boom"}}}),
    Outcome::Failed(Failure { kind: FailureKind::Other, message: "boom".into() })
)]
#[case::failed_without_details(
    json!({"turn": {"status": "failed"}}),
    Outcome::Failed(Failure { kind: FailureKind::Other, message: "Codex stopped with an error.".into() })
)]
#[case::failed_with_details(
    json!({"turn": {"status": "failed", "error": {"additionalDetails": "details only"}}}),
    Outcome::Failed(Failure { kind: FailureKind::Other, message: "details only".into() })
)]
#[case::weird_types(
    json!({"turn": {"status": 5, "error": 7, "id": []}}),
    Outcome::Completed
)]
fn odd_completions_are_read_tolerantly(#[case] params: Value, #[case] expected: Outcome) {
    let mut normalizer = normalizer();
    let out = normalizer.feed("turn/completed", &params);
    assert_eq!(out.events, vec![Event::TurnFinished(expected)]);
}

#[test]
fn the_flat_error_shape_is_held_and_classified() {
    let mut normalizer = normalizer();
    let out = normalizer.feed(
        "error",
        &json!({"threadId": "t", "message": "429 Too Many Requests", "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 429}}, "willRetry": false}),
    );
    assert!(out.events.is_empty());
    let out = normalizer.feed(
        "turn/completed",
        &json!({"threadId": "t", "turn": {"status": "failed"}}),
    );
    assert_eq!(
        out.events,
        vec![Event::TurnFinished(Outcome::Failed(Failure {
            kind: FailureKind::UsageLimit,
            message: "429 Too Many Requests".into()
        }))]
    );
}

#[test]
fn an_error_held_in_one_turn_does_not_leak_into_the_next() {
    let mut normalizer = normalizer();
    normalizer.feed(
        "error",
        &json!({"error": {"message": "boom"}, "willRetry": false}),
    );
    normalizer.feed("turn/completed", &json!({"turn": {"status": "failed"}}));
    normalizer.begin_turn();
    let out = normalizer.feed("turn/completed", &json!({"turn": {"status": "failed"}}));
    assert_eq!(
        out.events,
        vec![Event::TurnFinished(Outcome::Failed(Failure {
            kind: FailureKind::Other,
            message: "Codex stopped with an error.".into()
        }))]
    );
}

#[test]
fn a_failure_built_from_an_rpc_error_reads_its_code() {
    let info = ErrorInfo::from_rpc("nope", Some(&json!({"codexErrorInfo": "unauthorized"})));
    let mut normalizer = normalizer();
    let out = normalizer.fail_with(&info);
    assert_eq!(
        out.events,
        vec![Event::TurnFinished(Outcome::Failed(Failure {
            kind: FailureKind::NotLoggedIn,
            message: "nope".into()
        }))]
    );
}

#[test]
fn the_turn_id_is_taken_from_the_first_announcement_only() {
    let mut normalizer = normalizer();
    assert_eq!(normalizer.turn_id(), None);
    normalizer.feed(
        "turn/started",
        &json!({"threadId": "t", "turn": {"id": "one"}}),
    );
    normalizer.set_turn("two");
    assert_eq!(normalizer.turn_id(), Some("one"));
}
