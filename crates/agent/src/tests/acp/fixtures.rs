use std::collections::HashSet;

use serde_json::Value;
use tokio::task::JoinHandle;

use super::rig::*;
use crate::session::Command;
use crate::{AcpFlavor, Event, FailureKind, Launch};

const BASIC: &str = include_str!("../../fixtures/opencode/basic.jsonl");
const BASIC_SENT: &str = include_str!("../../fixtures/opencode/basic.sent.jsonl");
const LONG: &str = include_str!("../../fixtures/opencode/long.jsonl");
const LONG_SENT: &str = include_str!("../../fixtures/opencode/long.sent.jsonl");
const CANCEL: &str = include_str!("../../fixtures/opencode/cancel.jsonl");
const CANCEL_SENT: &str = include_str!("../../fixtures/opencode/cancel.sent.jsonl");
const RESUME: &str = include_str!("../../fixtures/opencode/resume.jsonl");
const RESUME_SENT: &str = include_str!("../../fixtures/opencode/resume.sent.jsonl");
const LOAD: &str = include_str!("../../fixtures/opencode/load.jsonl");
const LOAD_SENT: &str = include_str!("../../fixtures/opencode/load.sent.jsonl");
const MISSING: &str = include_str!("../../fixtures/opencode/resume_missing.jsonl");
const MISSING_SENT: &str = include_str!("../../fixtures/opencode/resume_missing.sent.jsonl");
const PERMISSION: &str = include_str!("../../fixtures/opencode/permission_execute.jsonl");
const PERMISSION_SENT: &str = include_str!("../../fixtures/opencode/permission_execute.sent.jsonl");
const EDIT: &str = include_str!("../../fixtures/opencode/edit_tool.jsonl");
const EDIT_SENT: &str = include_str!("../../fixtures/opencode/edit_tool.sent.jsonl");
const PLAN: &str = include_str!("../../fixtures/opencode/plan_mode.jsonl");
const PLAN_SENT: &str = include_str!("../../fixtures/opencode/plan_mode.sent.jsonl");

const BASIC_SESSION: &str = "ses_dfd745da8ffenyHw6F7i7juCEt";
const BASIC_PROMPT: &str = "Run `git log --oneline` and tell me how many commits there are.";
const FIRST_QUESTION: &str = "In one sentence: what was the first thing I asked you?";

struct Script {
    stdout: Vec<Value>,
    sent: Vec<Value>,
}

impl Script {
    fn new(stdout: &str, sent: &str) -> Self {
        let parse = |text: &str| {
            text.lines()
                .map(|line| serde_json::from_str(line).expect("a recorded line is JSON"))
                .collect()
        };
        Self {
            stdout: parse(stdout),
            sent: parse(sent),
        }
    }
}

fn same(mut got: Value, want: &Value) {
    if got["method"] == "initialize" {
        got["params"]["clientInfo"]["version"] = want["params"]["clientInfo"]["version"].clone();
    }
    if got["params"]["cwd"] == absolute_cwd().as_str() {
        got["params"]["cwd"] = want["params"]["cwd"].clone();
    }
    assert_eq!(&got, want, "the driver wrote something else than recorded");
}

fn is_response(line: &Value) -> bool {
    line.get("method").is_none() && line.get("id").is_some()
}

fn is_request(line: &Value) -> bool {
    line.get("method").is_some() && line.get("id").is_some()
}

struct Replay {
    peer: Peer,
    sent: Vec<Value>,
    cursor: usize,
    requests: HashSet<String>,
}

impl Replay {
    async fn read_next(&mut self) -> Value {
        let got = self.peer.read().await;
        let want = self
            .sent
            .get(self.cursor)
            .unwrap_or_else(|| panic!("the driver wrote more than recorded: {got}"))
            .clone();
        self.cursor += 1;
        same(got.clone(), &want);
        if is_request(&got) {
            self.requests.insert(got["id"].to_string());
        }
        got
    }

    async fn until_request(&mut self, id: &Value) {
        while !self.requests.contains(&id.to_string()) {
            self.read_next().await;
        }
    }

    async fn until_reply(&mut self, id: &Value) {
        loop {
            let message = self.read_next().await;
            if is_response(&message) && message["id"] == *id {
                return;
            }
        }
    }

    async fn trailing_notifications(&mut self) {
        while self.cursor < self.sent.len() && self.sent[self.cursor].get("id").is_none() {
            self.read_next().await;
        }
    }
}

fn play(peer: Peer, script: Script, tweak: fn(Value) -> Value) -> JoinHandle<Peer> {
    tokio::spawn(async move {
        let mut replay = Replay {
            peer,
            sent: script.sent,
            cursor: 0,
            requests: HashSet::new(),
        };
        for (at, line) in script.stdout.iter().enumerate() {
            let line = tweak(line.clone());
            let next_response = script.stdout[at..]
                .iter()
                .find(|ahead| is_response(ahead))
                .map(|ahead| ahead["id"].clone());
            if let Some(id) = next_response {
                replay.until_request(&id).await;
            }
            if is_response(&line) {
                replay.trailing_notifications().await;
            }
            replay.peer.write_value(line.clone()).await;
            if is_request(&line) {
                replay.until_reply(&line["id"]).await;
            }
        }
        assert_eq!(
            replay.cursor,
            replay.sent.len(),
            "the driver wrote less than recorded"
        );
        replay.peer
    })
}

fn untouched(line: Value) -> Value {
    line
}

fn without_resume(mut line: Value) -> Value {
    if let Some(Value::Object(capabilities)) =
        line.pointer_mut("/result/agentCapabilities/sessionCapabilities")
    {
        capabilities.remove("resume");
    }
    line
}

fn resuming(id: &str) -> Launch {
    Launch {
        resume: Some(id.into()),
        ..launch()
    }
}

fn started(
    flavor: AcpFlavor,
    launch: Launch,
    stdout: &str,
    sent: &str,
    tweak: fn(Value) -> Value,
) -> (Rig, JoinHandle<Peer>) {
    let mut rig = Rig::start_with(flavor, launch, tight());
    let played = play(rig.peer.take().unwrap(), Script::new(stdout, sent), tweak);
    (rig, played)
}

fn tool_events(events: &[Event]) -> Vec<(String, bool)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::ToolStarted { title, .. } => Some((title.clone(), false)),
            Event::ToolFinished { failed, .. } => Some((String::new(), *failed)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_turn_with_a_shell_command() {
    let (rig, played) = started(AcpFlavor::Generic, launch(), BASIC, BASIC_SENT, untouched);
    rig.prompt(BASIC_PROMPT);
    let tool = "chatcmpl-tool-a8159a26d0fd2671";
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(BASIC_SESSION),
            text("I'll run that command for you."),
            tool_started(tool, "git log --oneline"),
            tool_finished(tool, false),
            thinking("We need answer concise. Count 2 commits. Need maybe mention.\n"),
            text("\n\n2 commits."),
            completed(),
        ]
    );
    assert!(rig.no_events().await);
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn a_long_turn_ends_once_after_the_last_text() {
    let (rig, played) = started(AcpFlavor::Generic, launch(), LONG, LONG_SENT, untouched);
    rig.prompt("Do these steps one at a time, each as a separate shell command, and narrate briefly between them: 1) git log --oneline 2) git show --stat HEAD 3) cat lib.rs 4) git diff HEAD~1 HEAD 5) wc -l README.md. Finish with a final line that says exactly: ALL DONE.");
    let events = rig.until_finished().await;
    assert_eq!(events.last(), Some(&completed()));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::TurnFinished(_)))
            .count(),
        1
    );
    let last_text = events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::Text(text) => Some(text.clone()),
            _ => None,
        })
        .expect("the turn has text");
    assert!(last_text.trim_end().ends_with("ALL DONE."), "{last_text}");
    let tools = tool_events(&events);
    let titles: Vec<&str> = tools
        .iter()
        .filter(|(title, _)| !title.is_empty())
        .map(|(title, _)| title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "git log --oneline",
            "git show --stat HEAD",
            "cat lib.rs",
            "git diff HEAD~1 HEAD",
            "wc -l README.md",
        ]
    );
    assert_eq!(
        tools.iter().filter(|(title, _)| title.is_empty()).count(),
        5
    );
    assert!(tools.iter().all(|(_, failed)| !failed));
    assert!(rig.no_events().await);
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn a_cancelled_turn_is_interrupted_and_the_next_one_runs() {
    let (rig, played) = started(AcpFlavor::Generic, launch(), CANCEL, CANCEL_SENT, untouched);
    rig.prompt("Run the shell command `sleep 40 && echo done` and report its output.");
    let tool = "chatcmpl-tool-a9667b9866c23400";
    assert_eq!(rig.next().await, session("ses_dfd737020ffeWOCUCbWnGmSQ8v"));
    loop {
        match rig.next().await {
            Event::Thinking(_) | Event::Text(_) => {}
            event => {
                assert_eq!(event, tool_started(tool, "sleep 40 && echo done"));
                break;
            }
        }
    }
    rig.send(Command::Interrupt);
    assert_eq!(
        rig.until_finished().await,
        vec![tool_finished(tool, false), interrupted()]
    );
    rig.prompt("Reply with the single word OK.");
    let second = rig.until_finished().await;
    assert_eq!(second.last(), Some(&completed()));
    assert!(
        second
            .iter()
            .any(|event| matches!(event, Event::Text(text) if text.contains("OK"))),
        "{second:?}"
    );
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn resume_picks_up_the_session_without_a_replay() {
    let (rig, played) = started(
        AcpFlavor::Generic,
        resuming(BASIC_SESSION),
        RESUME,
        RESUME_SENT,
        untouched,
    );
    rig.prompt(FIRST_QUESTION);
    let events = rig.until_finished().await;
    assert_eq!(events.first(), Some(&session(BASIC_SESSION)));
    assert_eq!(events.last(), Some(&completed()));
    assert!(matches!(&events[2], Event::Text(text) if text.starts_with("You first asked me")));
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn the_history_replayed_by_load_never_reaches_the_events() {
    let (rig, played) = started(
        AcpFlavor::Generic,
        resuming(BASIC_SESSION),
        LOAD,
        LOAD_SENT,
        without_resume,
    );
    rig.prompt(FIRST_QUESTION);
    let events = rig.until_finished().await;
    assert_eq!(events.first(), Some(&session(BASIC_SESSION)));
    assert_eq!(events.last(), Some(&completed()));
    assert!(
        events.iter().all(|event| !matches!(
            event,
            Event::ToolStarted { .. } | Event::ToolFinished { .. }
        )),
        "{events:?}"
    );
    let said: String = events
        .iter()
        .filter_map(|event| match event {
            Event::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        said,
        "You first asked me to run `git log --oneline` and tell you how many commits there were."
    );
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn resuming_an_unknown_session_is_a_lost_session() {
    let (rig, played) = started(
        AcpFlavor::OpenCode,
        resuming("ses_00000000000000000000000000"),
        MISSING,
        MISSING_SENT,
        untouched,
    );
    rig.prompt("hello");
    let events = rig.until_finished().await;
    assert_eq!(
        events,
        vec![failed(
            FailureKind::SessionLost,
            "Internal error: OpenCode service failure"
        )]
    );
    played.await.unwrap();
    rig.ends().await;
}

#[tokio::test]
async fn an_allowed_shell_command_is_answered_with_allow_once() {
    let (rig, played) = started(
        AcpFlavor::Generic,
        launch(),
        PERMISSION,
        PERMISSION_SENT,
        untouched,
    );
    rig.prompt("Sandbox test. Step 1: use your file editing/writing tool (not the shell) to create hello.txt containing hi. Step 2: run the shell command: echo hi > hello2.txt; echo status=$? . Report exactly what happened for each step, including any refusal or error text.");
    let events = rig.until_finished().await;
    let tool = "chatcmpl-tool-99908db89cdc2138";
    assert!(events.contains(&tool_started(tool, "echo hi > hello2.txt; echo status=$?")));
    assert!(events.contains(&tool_finished(tool, false)));
    assert_eq!(events.last(), Some(&completed()));
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn the_edit_tool_is_one_started_and_one_finished_call() {
    let (rig, played) = started(AcpFlavor::Generic, launch(), EDIT, EDIT_SENT, untouched);
    rig.prompt("Sandbox test. Step 1: use your file editing/writing tool (not the shell) to create hello.txt containing hi. Step 2: run the shell command: echo hi > hello2.txt; echo status=$? . Report exactly what happened for each step, including any refusal or error text.");
    let events = rig.until_finished().await;
    assert_eq!(
        tool_events(&events),
        vec![
            ("write".to_owned(), false),
            (String::new(), false),
            ("echo hi > hello2.txt; echo status=$?".to_owned(), false),
            (String::new(), false),
        ]
    );
    assert_eq!(events.last(), Some(&completed()));
    played.await.unwrap();
    rig.stop().await;
}

#[tokio::test]
async fn opencode_switches_to_plan_mode_after_the_session_is_created() {
    let (rig, played) = started(AcpFlavor::OpenCode, launch(), PLAN, PLAN_SENT, untouched);
    rig.prompt("Sandbox test. Step 1: use your file editing/writing tool (not the shell) to create hello.txt containing hi. Step 2: run the shell command: echo hi > hello2.txt; echo status=$? . Report exactly what happened for each step, including any refusal or error text.");
    let events = rig.until_finished().await;
    assert_eq!(
        events.first(),
        Some(&session("ses_dfd71f820ffeJanfZHt2iYXreF"))
    );
    assert_eq!(events.last(), Some(&completed()));
    assert!(tool_events(&events).is_empty());
    played.await.unwrap();
    rig.stop().await;
}
