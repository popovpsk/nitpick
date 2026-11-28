use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::duplex;

use crate::claude::ClaudeDriver;
use crate::claude::history::{parse, read};
use crate::session::{Channels, Driver, Transport};
use crate::{Entry, EntryRole, Event, HarnessKind, Launch};

const HISTORY: &str = include_str!("../../fixtures/claude/history.jsonl");
const ID: &str = "1f27673b-2082-4678-a679-076eeab335e0";

fn user(text: &str) -> Entry {
    Entry {
        role: EntryRole::User,
        text: text.into(),
    }
}

fn assistant(text: &str) -> Entry {
    Entry {
        role: EntryRole::Assistant,
        text: text.into(),
    }
}

fn lines(records: &[serde_json::Value]) -> String {
    records.iter().map(|record| format!("{record}\n")).collect()
}

fn said(text: &str) -> serde_json::Value {
    json!({"type": "user", "message": {"role": "user", "content": text}})
}

fn answered(id: &str, text: &str) -> serde_json::Value {
    json!({"type": "assistant", "message": {"role": "assistant", "id": id, "model": "m",
        "content": [{"type": "text", "text": text}]}})
}

#[test]
fn a_recorded_session_reads_as_prompts_and_answers() {
    assert_eq!(
        parse(HISTORY),
        vec![
            user("Reply with exactly the two words: first answer"),
            assistant("first answer"),
            user("Use the Read tool on note.txt, then reply with its content as one word."),
            assistant("pineapple"),
        ]
    );
}

#[test]
fn messages_of_one_turn_are_joined_as_paragraphs() {
    let text = lines(&[
        said("q"),
        answered("m1", "Looking."),
        json!({"type": "user", "message": {"role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "t", "content": "out"}]}}),
        answered("m2", "Found it."),
        answered("m2", " Twice."),
    ]);
    assert_eq!(
        parse(&text),
        vec![user("q"), assistant("Looking.\n\nFound it. Twice.")]
    );
}

#[test]
fn side_threads_meta_errors_and_markers_are_skipped() {
    let text = lines(&[
        said("q"),
        json!({"type": "user", "isMeta": true, "message": {"role": "user", "content": "meta"}}),
        json!({"type": "assistant", "isSidechain": true, "message": {"role": "assistant",
            "id": "s", "content": [{"type": "text", "text": "sub-agent"}]}}),
        json!({"type": "assistant", "isApiErrorMessage": true, "message": {"role": "assistant",
            "id": "e", "content": [{"type": "text", "text": "API Error: 500"}]}}),
        json!({"type": "assistant", "error": "rate_limit", "message": {"role": "assistant",
            "id": "e2", "content": [{"type": "text", "text": "limited"}]}}),
        json!({"type": "assistant", "message": {"role": "assistant", "id": "x",
            "model": "<synthetic>", "content": [{"type": "text", "text": "No response requested."}]}}),
        json!({"type": "user", "message": {"role": "user",
            "content": [{"type": "text", "text": "[Request interrupted by user]"}]}}),
        json!({"type": "user", "isCompactSummary": true,
            "message": {"role": "user", "content": "summary"}}),
        said("next"),
        answered("m", "answer"),
    ]);
    assert_eq!(
        parse(&text),
        vec![user("q"), user("next"), assistant("answer")]
    );
}

#[test]
fn unreadable_lines_and_other_records_are_ignored() {
    let text = format!(
        "not json\n{}\n{}\n{{\"type\":\"user\"}}\n{}",
        json!({"type": "attachment"}),
        said("q"),
        answered("m", "a"),
    );
    assert_eq!(parse(&text), vec![user("q"), assistant("a")]);
}

#[test]
fn text_blocks_of_a_prompt_are_joined() {
    let text = lines(&[
        json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "text", "text": "one"}, {"type": "text", "text": "two"}]}}),
    ]);
    assert_eq!(parse(&text), vec![user("one\n\ntwo")]);
}

#[test]
fn a_missing_file_has_no_history() {
    let folder = tempfile::tempdir().unwrap();
    assert!(read(&folder.path().join("none.jsonl")).is_empty());
}

#[tokio::test]
async fn a_resume_that_wants_history_sends_it_first() {
    let config = tempfile::tempdir().unwrap();
    let file = config.path().join(format!("{ID}.jsonl"));
    std::fs::write(&file, HISTORY).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let found = file.clone();
    let driver = ClaudeDriver::with_lookup(Arc::new(move |_| Some(found.clone())));
    let launch = Launch {
        kind: HarnessKind::Claude,
        command: "claude".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: Some(ID.into()),
        history: true,
        instructions: String::new(),
        scratch: scratch.path().to_path_buf(),
    };
    let plan = driver.plan(&launch).unwrap();
    assert!(plan.refusal.is_none());
    let (driver_end, _peer) = duplex(1 << 16);
    let (reader, writer) = tokio::io::split(driver_end);
    let (_commands, command_receiver) = flume::unbounded();
    let (events, received) = flume::unbounded();
    let run = tokio::spawn(driver.run(
        launch,
        Transport::new(reader, writer),
        Channels {
            commands: command_receiver,
            events,
        },
    ));
    let first = tokio::time::timeout(Duration::from_secs(5), received.recv_async())
        .await
        .expect("the history came in time")
        .unwrap();
    match first {
        Event::History(entries) => assert_eq!(entries.len(), 4),
        other => panic!("expected the history first, got {other:?}"),
    }
    run.abort();
}

#[test]
fn a_resume_without_history_reads_no_file() {
    let scratch = tempfile::tempdir().unwrap();
    let driver = ClaudeDriver::with_lookup(Arc::new(|_| Some(PathBuf::from("/nowhere.jsonl"))));
    let launch = Launch {
        kind: HarnessKind::Claude,
        command: "claude".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: Some(ID.into()),
        history: false,
        instructions: String::new(),
        scratch: scratch.path().to_path_buf(),
    };
    let plan = driver.plan(&launch).unwrap();
    assert!(plan.args.iter().any(|arg| arg == &format!("--resume={ID}")));
}
