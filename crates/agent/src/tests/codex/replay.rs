use std::path::PathBuf;
use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinHandle;

use crate::codex::{CodexDriver, Limits};
use crate::lines::Lines;
use crate::session::{Channels, Command, Driver, Transport};
use crate::{Event, Failure, FailureKind, HarnessKind, Launch, Outcome};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(100);
const THREAD: &str = "01a00fe7-c301-7d52-ae1f-41ecda48b29b";
const USER_AGENT: &str = "nitpick/0.171.0 (Mac OS 15.1.0; arm64) iTerm.app (nitpick; 0.1.0)";
const STOPPED: &str = "The assistant stopped before it finished the reply.";
const NOT_READY: &str = "Codex stopped before the session was ready.";

fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Codex,
        command: "codex".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: None,
        instructions: "Be brief.".into(),
        scratch: PathBuf::new(),
    }
}

fn absolute_cwd() -> String {
    std::path::absolute("/work/toy")
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn tight() -> Limits {
    Limits {
        initialize: Duration::from_millis(1000),
        thread: Duration::from_millis(1000),
        turn_start: Duration::from_millis(300),
        interrupt: Duration::from_millis(300),
        stop: Duration::from_millis(600),
    }
}

struct Peer {
    reader: Lines<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

struct Rig {
    commands: flume::Sender<Command>,
    events: flume::Receiver<Event>,
    peer: Option<Peer>,
    driver: JoinHandle<()>,
}

impl Rig {
    fn start() -> Self {
        Self::start_with(launch(), Limits::default())
    }

    fn start_with(launch: Launch, limits: Limits) -> Self {
        let (driver_end, peer_end) = duplex(1 << 20);
        let (driver_reader, driver_writer) = split(driver_end);
        let (peer_reader, peer_writer) = split(peer_end);
        let (commands, command_receiver) = flume::unbounded();
        let (event_sender, events) = flume::unbounded();
        let driver = tokio::spawn(CodexDriver::new().with_limits(limits).run(
            launch,
            Transport::new(driver_reader, driver_writer),
            Channels {
                commands: command_receiver,
                events: event_sender,
            },
        ));
        Self {
            commands,
            events,
            peer: Some(Peer {
                reader: Lines::new(peer_reader),
                writer: peer_writer,
            }),
            driver,
        }
    }

    fn peer(&mut self) -> &mut Peer {
        self.peer.as_mut().expect("the peer is connected")
    }

    fn send(&self, command: Command) {
        self.commands.send(command).unwrap();
    }

    fn prompt(&self, text: &str) {
        self.send(Command::Prompt(text.into()));
    }

    async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(WAIT, self.peer().reader.next_line())
            .await
            .expect("the driver wrote in time")
            .unwrap()
            .expect("the driver is still connected");
        let message: Value = serde_json::from_str(&line).expect("the driver wrote JSON");
        assert!(
            message.get("jsonrpc").is_none(),
            "Codex messages carry no jsonrpc tag: {message}"
        );
        message
    }

    async fn reads_nothing(&mut self) -> bool {
        tokio::time::timeout(QUIET, self.peer().reader.next_line())
            .await
            .is_err()
    }

    async fn reads_eof(&mut self) -> bool {
        matches!(
            tokio::time::timeout(WAIT, self.peer().reader.next_line()).await,
            Ok(Ok(None))
        )
    }

    async fn request(&mut self, method: &str) -> (Value, Value) {
        let message = self.read().await;
        assert_eq!(message["method"], method, "{message}");
        assert!(message.get("id").is_some(), "{method} is a request");
        (message["id"].clone(), message["params"].clone())
    }

    async fn notification(&mut self, method: &str) -> Value {
        let message = self.read().await;
        assert_eq!(message["method"], method, "{message}");
        assert!(message.get("id").is_none(), "{method} is a notification");
        message["params"].clone()
    }

    async fn write_value(&mut self, value: Value) {
        let writer = &mut self.peer().writer;
        writer
            .write_all(value.to_string().as_bytes())
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
    }

    async fn write_raw(&mut self, line: &str) {
        let writer = &mut self.peer().writer;
        writer.write_all(line.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
    }

    async fn respond(&mut self, id: Value, result: Value) {
        self.write_value(json!({"id": id, "result": result})).await;
    }

    async fn respond_error(&mut self, id: Value, message: &str) {
        self.write_value(json!({"id": id, "error": {"code": -32600, "message": message}}))
            .await;
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.write_value(json!({"method": method, "params": params}))
            .await;
    }

    async fn hang_up(&mut self) {
        self.peer().writer.shutdown().await.unwrap();
    }

    async fn next(&self) -> Event {
        tokio::time::timeout(WAIT, self.events.recv_async())
            .await
            .expect("an event arrived in time")
            .expect("the event channel is open")
    }

    async fn until_finished(&self) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            let event = self.next().await;
            let done = matches!(event, Event::TurnFinished(_));
            events.push(event);
            if done {
                return merge(events);
            }
        }
    }

    async fn no_events(&self) -> bool {
        !matches!(
            tokio::time::timeout(QUIET, self.events.recv_async()).await,
            Ok(Ok(_))
        )
    }

    async fn ends(self) {
        tokio::time::timeout(WAIT, self.driver)
            .await
            .expect("the driver returned")
            .unwrap();
    }

    async fn stop(self) {
        self.send(Command::Shutdown);
        self.ends().await;
    }

    async fn handshake_with(&mut self, user_agent: &str, thread: &str) -> Value {
        let (id, _) = self.request("initialize").await;
        self.respond(
            id,
            json!({"userAgent": user_agent, "codexHome": "/home/me/.codex", "platformFamily": "unix", "platformOs": "macos"}),
        )
        .await;
        self.notification("initialized").await;
        let (id, params) = self.request("thread/start").await;
        self.respond(id, thread_result(thread)).await;
        params
    }

    async fn handshake(&mut self) {
        self.handshake_with(USER_AGENT, THREAD).await;
    }

    async fn start_turn(&mut self, turn: &str, text: &str) {
        let (id, params) = self.request("turn/start").await;
        assert_eq!(params["input"][0]["text"], text);
        self.respond(
            id,
            json!({"turn": {"id": turn, "status": "inProgress", "items": [], "error": null}}),
        )
        .await;
        self.notify(
            "turn/started",
            json!({"threadId": THREAD, "turn": {"id": turn, "status": "inProgress"}}),
        )
        .await;
    }

    async fn complete(&mut self, turn: &str, status: &str) {
        self.notify(
            "turn/completed",
            json!({"threadId": THREAD, "turn": {"id": turn, "status": status, "error": null}}),
        )
        .await;
    }

    async fn ready(prompt: &str) -> Self {
        let mut rig = Rig::start();
        rig.prompt(prompt);
        rig.handshake().await;
        rig
    }
}

fn thread_result(thread: &str) -> Value {
    json!({
        "thread": {"id": thread, "sessionId": thread, "cliVersion": "0.171.0", "turns": []},
        "model": "gpt-5.6-sol",
        "approvalPolicy": "never",
        "sandbox": {"type": "readOnly"},
        "instructionSources": [],
    })
}

fn merge(events: Vec<Event>) -> Vec<Event> {
    let mut merged: Vec<Event> = Vec::new();
    for event in events {
        match (merged.last_mut(), event) {
            (Some(Event::Text(text)), Event::Text(more)) => text.push_str(&more),
            (Some(Event::Thinking(text)), Event::Thinking(more)) => text.push_str(&more),
            (_, event) => merged.push(event),
        }
    }
    merged
}

fn tool_started(id: &str, title: &str) -> Event {
    Event::ToolStarted {
        id: id.into(),
        title: title.into(),
    }
}

fn tool_finished(id: &str, failed: bool) -> Event {
    Event::ToolFinished {
        id: id.into(),
        failed,
    }
}

fn session(id: &str) -> Event {
    Event::Session { id: id.into() }
}

fn text(text: &str) -> Event {
    Event::Text(text.into())
}

fn thinking(text: &str) -> Event {
    Event::Thinking(text.into())
}

fn completed() -> Event {
    Event::TurnFinished(Outcome::Completed)
}

fn interrupted() -> Event {
    Event::TurnFinished(Outcome::Interrupted)
}

fn failed(kind: FailureKind, message: &str) -> Event {
    Event::TurnFinished(Outcome::Failed(Failure {
        kind,
        message: message.into(),
    }))
}

fn agent_delta(item: &str, delta: &str) -> Value {
    json!({"threadId": THREAD, "turnId": "turn-1", "itemId": item, "delta": delta})
}

fn message_item(item: &str, body: &str) -> Value {
    json!({"threadId": THREAD, "turnId": "turn-1", "item": {"type": "agentMessage", "id": item, "text": body, "phase": "final_answer"}})
}

fn command_item(id: &str, status: &str, exit_code: Value) -> Value {
    json!({
        "threadId": THREAD,
        "turnId": "turn-1",
        "item": {
            "type": "commandExecution",
            "id": id,
            "command": "/bin/zsh -lc 'git log --oneline'",
            "cwd": "/work/toy",
            "status": status,
            "commandActions": [{"type": "unknown", "command": "git log --oneline"}],
            "aggregatedOutput": null,
            "exitCode": exit_code,
        },
    })
}

#[tokio::test]
async fn a_turn_with_reasoning_a_command_and_two_messages() {
    let mut rig = Rig::ready("what changed?").await;
    rig.start_turn("turn-1", "what changed?").await;
    rig.notify(
        "item/reasoning/summaryTextDelta",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "r1", "summaryIndex": 0, "delta": "Checking the "}),
    )
    .await;
    rig.notify(
        "item/reasoning/summaryTextDelta",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "r1", "summaryIndex": 0, "delta": "log."}),
    )
    .await;
    rig.notify(
        "item/reasoning/summaryPartAdded",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "r1", "summaryIndex": 1}),
    )
    .await;
    rig.notify(
        "item/reasoning/summaryTextDelta",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "r1", "summaryIndex": 1, "delta": "Then reading."}),
    )
    .await;
    rig.notify(
        "item/started",
        command_item("exec-1", "inProgress", Value::Null),
    )
    .await;
    rig.notify(
        "item/commandExecution/outputDelta",
        json!({"threadId": THREAD, "itemId": "exec-1", "delta": "abc"}),
    )
    .await;
    rig.notify(
        "item/completed",
        command_item("exec-1", "completed", json!(0)),
    )
    .await;
    rig.notify("item/agentMessage/delta", agent_delta("m1", "I found "))
        .await;
    rig.notify("item/agentMessage/delta", agent_delta("m1", "two commits."))
        .await;
    rig.notify("item/completed", message_item("m1", "I found two commits."))
        .await;
    rig.notify("item/agentMessage/delta", agent_delta("m2", "That is all."))
        .await;
    rig.notify("item/completed", message_item("m2", "That is all."))
        .await;
    rig.complete("turn-1", "completed").await;

    assert_eq!(
        rig.until_finished().await,
        vec![
            session(THREAD),
            thinking("Checking the log.\n\nThen reading."),
            tool_started("exec-1", "git log --oneline"),
            tool_finished("exec-1", false),
            text("I found two commits.\n\nThat is all."),
            completed(),
        ]
    );
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn the_second_turn_starts_without_a_leading_separator() {
    let mut rig = Rig::ready("one").await;
    rig.prompt("two");
    rig.start_turn("turn-1", "one").await;
    rig.notify("item/agentMessage/delta", agent_delta("m1", "First."))
        .await;
    rig.notify("item/completed", message_item("m1", "First."))
        .await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), text("First."), completed()]
    );
    rig.start_turn("turn-2", "two").await;
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": THREAD, "turnId": "turn-2", "itemId": "m2", "delta": "Second."}),
    )
    .await;
    rig.complete("turn-2", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![text("Second."), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_message_that_only_arrives_complete_is_used_and_separated() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify("item/agentMessage/delta", agent_delta("m1", "Streamed."))
        .await;
    rig.notify("item/completed", message_item("m1", "Streamed."))
        .await;
    rig.notify("item/completed", message_item("m2", "Whole."))
        .await;
    rig.notify("item/completed", message_item("m3", "")).await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), text("Streamed.\n\nWhole."), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_text_delta_may_be_named_text_delta() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "m1", "textDelta": "Hi"}),
    )
    .await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), text("Hi"), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn the_requests_carry_the_read_only_parameters() {
    let mut launch = launch();
    launch.model = Some("gpt-5.6-sol".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    rig.prompt("explain");

    let (id, initialize) = rig.request("initialize").await;
    assert_eq!(
        initialize,
        json!({
            "clientInfo": {"name": "nitpick", "title": "Nitpick", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true},
        })
    );
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    assert_eq!(rig.notification("initialized").await, json!({}));

    let (id, start) = rig.request("thread/start").await;
    assert_eq!(
        start,
        json!({
            "cwd": absolute_cwd(),
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "model": "gpt-5.6-sol",
            "developerInstructions": "Be brief.",
        })
    );
    rig.respond(id, thread_result(THREAD)).await;

    let (id, turn) = rig.request("turn/start").await;
    assert_eq!(
        turn,
        json!({
            "threadId": THREAD,
            "input": [{"type": "text", "text": "explain"}],
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "readOnly"},
            "summary": "auto",
        })
    );
    rig.respond(id, json!({"turn": {"id": "turn-1"}})).await;
    for sent in [&initialize, &start, &turn] {
        let sent = sent.to_string();
        assert!(!sent.contains("baseInstructions"), "{sent}");
        assert!(!sent.contains("untrusted"), "{sent}");
    }
    rig.stop().await;
}

#[tokio::test]
async fn a_resumed_thread_sends_the_same_policy_and_the_thread_id() {
    let mut launch = launch();
    launch.resume = Some("thread-old".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    let (id, resume) = rig.request("thread/resume").await;
    assert_eq!(
        resume,
        json!({
            "threadId": "thread-old",
            "excludeTurns": true,
            "cwd": absolute_cwd(),
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "developerInstructions": "Be brief.",
        })
    );
    let sent = resume.to_string();
    assert!(!sent.contains("baseInstructions") && !sent.contains("untrusted"));
    rig.respond(id, thread_result("thread-old")).await;
    assert_eq!(rig.next().await, session("thread-old"));

    rig.prompt("again");
    let (_, params) = rig.request("turn/start").await;
    assert_eq!(params["threadId"], "thread-old");
    rig.stop().await;
}

#[tokio::test]
async fn a_resume_reply_without_a_thread_keeps_the_requested_id() {
    let mut launch = launch();
    launch.resume = Some("thread-old".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({})).await;
    rig.notification("initialized").await;
    let (id, _) = rig.request("thread/resume").await;
    rig.respond(id, json!({})).await;
    assert_eq!(rig.next().await, session("thread-old"));
    rig.stop().await;
}

async fn answer_to(method: &str, id: Value) -> (Value, Rig) {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.write_value(
        json!({"id": id, "method": method, "params": {"threadId": THREAD, "itemId": "x"}}),
    )
    .await;
    let reply = rig.read().await;
    (reply, rig)
}

#[rstest]
#[case::command("item/commandExecution/requestApproval", json!({"decision": "decline"}))]
#[case::file_change("item/fileChange/requestApproval", json!({"decision": "decline"}))]
#[case::permissions("item/permissions/requestApproval", json!({"permissions": {}, "scope": "turn"}))]
#[case::user_input("item/tool/requestUserInput", json!({"answers": {}}))]
#[case::elicitation("mcpServer/elicitation/request", json!({"action": "decline"}))]
#[tokio::test]
async fn a_server_request_gets_its_documented_answer(#[case] method: &str, #[case] result: Value) {
    let (reply, rig) = answer_to(method, json!(41)).await;
    assert_eq!(reply, json!({"id": 41, "result": result}));
    assert_eq!(rig.next().await, session(THREAD));
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[rstest]
#[case::dynamic_tool("item/tool/call")]
#[case::auth_refresh("account/chatgptAuthTokens/refresh")]
#[case::made_up("nitpick/nothing")]
#[tokio::test]
async fn an_unknown_server_request_gets_method_not_found(#[case] method: &str) {
    let (reply, rig) = answer_to(method, json!("req-7")).await;
    assert_eq!(reply["id"], "req-7");
    assert_eq!(reply["error"]["code"], -32601);
    assert!(reply.get("result").is_none());
    rig.stop().await;
}

#[tokio::test]
async fn a_server_request_before_the_handshake_ends_is_answered_too() {
    let mut rig = Rig::start();
    let (id, _) = rig.request("initialize").await;
    rig.write_value(json!({"id": 5, "method": "item/tool/requestUserInput", "params": {}}))
        .await;
    assert_eq!(
        rig.read().await,
        json!({"id": 5, "result": {"answers": {}}})
    );
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    rig.stop().await;
}

#[tokio::test]
async fn the_second_prompt_waits_for_the_end_of_the_first_turn() {
    let mut rig = Rig::ready("first").await;
    rig.prompt("second");
    rig.prompt("third");
    rig.start_turn("turn-1", "first").await;
    assert!(rig.reads_nothing().await);
    rig.notify("item/agentMessage/delta", agent_delta("m1", "one"))
        .await;
    assert!(rig.reads_nothing().await, "still no second turn");
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), text("one"), completed()]
    );

    rig.start_turn("turn-2", "second").await;
    assert!(rig.reads_nothing().await);
    rig.complete("turn-2", "completed").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);

    rig.start_turn("turn-3", "third").await;
    rig.complete("turn-3", "completed").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_stops_the_turn_and_the_queued_prompt_still_runs() {
    let mut rig = Rig::ready("slow").await;
    rig.prompt("next");
    rig.start_turn("turn-1", "slow").await;
    rig.notify(
        "item/started",
        command_item("exec-1", "inProgress", Value::Null),
    )
    .await;
    assert_eq!(rig.next().await, session(THREAD));
    assert!(matches!(rig.next().await, Event::ToolStarted { .. }));

    rig.send(Command::Interrupt);
    let (id, params) = rig.request("turn/interrupt").await;
    assert_eq!(params, json!({"threadId": THREAD, "turnId": "turn-1"}));
    rig.respond(id, json!({})).await;
    assert!(
        rig.reads_nothing().await,
        "nothing runs until the turn ends"
    );
    rig.complete("turn-1", "interrupted").await;
    assert_eq!(
        rig.until_finished().await,
        vec![tool_finished("exec-1", true), interrupted()]
    );

    rig.start_turn("turn-2", "next").await;
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": THREAD, "turnId": "turn-2", "itemId": "m", "delta": "OK"}),
    )
    .await;
    rig.complete("turn-2", "completed").await;
    assert_eq!(rig.until_finished().await, vec![text("OK"), completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_sent_before_the_turn_is_known_goes_out_with_its_id() {
    let mut rig = Rig::ready("slow").await;
    let (id, _) = rig.request("turn/start").await;
    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await);
    rig.respond(id, json!({"turn": {"id": "turn-1"}})).await;
    let (id, params) = rig.request("turn/interrupt").await;
    assert_eq!(params["turnId"], "turn-1");
    rig.respond(id, json!({})).await;
    rig.complete("turn-1", "interrupted").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), interrupted()]
    );
    assert!(rig.reads_nothing().await, "one interrupt only");
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_with_no_turn_running_writes_nothing() {
    let mut rig = Rig::ready("hello").await;
    rig.start_turn("turn-1", "hello").await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), completed()]
    );
    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await);
    rig.prompt("again");
    rig.request("turn/start").await;
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_that_loses_the_race_with_the_end_of_the_turn_is_dropped() {
    let mut rig = Rig::ready("quick").await;
    let (id, _) = rig.request("turn/start").await;
    rig.send(Command::Interrupt);
    rig.respond(id, json!({"turn": {"id": "turn-1"}})).await;
    rig.complete("turn-1", "completed").await;
    let events = rig.until_finished().await;
    assert_eq!(events.last(), Some(&completed()));
    rig.prompt("after");
    let message = rig.read().await;
    if message["method"] == "turn/interrupt" {
        rig.respond(message["id"].clone(), json!({})).await;
        let (_, params) = rig.request("turn/start").await;
        assert_eq!(params["input"][0]["text"], "after");
    } else {
        assert_eq!(message["method"], "turn/start");
    }
    rig.stop().await;
}

#[tokio::test]
async fn a_refused_interrupt_is_not_fatal() {
    let mut rig = Rig::ready("slow").await;
    rig.start_turn("turn-1", "slow").await;
    rig.send(Command::Interrupt);
    let (id, _) = rig.request("turn/interrupt").await;
    rig.respond_error(id, "turn not found").await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), completed()]
    );
    rig.stop().await;
}

#[rstest]
#[case::unauthorized(json!("unauthorized"), "Please sign in", FailureKind::NotLoggedIn)]
#[case::http_401(json!({"httpConnectionFailed": {"httpStatusCode": 401}}), "request failed", FailureKind::NotLoggedIn)]
#[case::usage_limit(json!("usageLimitExceeded"), "You've hit your usage limit", FailureKind::UsageLimit)]
#[case::http_429(json!({"httpConnectionFailed": {"httpStatusCode": 429}}), "slow down", FailureKind::UsageLimit)]
#[case::context(json!("contextWindowExceeded"), "too much text", FailureKind::ContextTooLong)]
#[case::bad_request(json!("badRequest"), "invalid parameter", FailureKind::Other)]
#[case::sandbox(json!("sandboxError"), "blocked", FailureKind::Other)]
#[tokio::test]
async fn a_failed_turn_is_classified_by_its_error_code(
    #[case] code: Value,
    #[case] message: &str,
    #[case] kind: FailureKind,
) {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify(
        "turn/completed",
        json!({"threadId": THREAD, "turn": {"id": "turn-1", "status": "failed", "error": {"message": message, "codexErrorInfo": code}}}),
    )
    .await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), failed(kind, message)]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_fatal_error_notification_is_delivered_with_the_end_of_the_turn() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify(
        "error",
        json!({"threadId": THREAD, "turnId": "turn-1", "willRetry": false, "error": {"message": "401 Unauthorized", "codexErrorInfo": "unauthorized"}}),
    )
    .await;
    assert_eq!(rig.next().await, session(THREAD));
    assert!(rig.no_events().await, "nothing before the turn ends");
    rig.notify(
        "turn/completed",
        json!({"threadId": THREAD, "turn": {"id": "turn-1", "status": "failed"}}),
    )
    .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::NotLoggedIn, "401 Unauthorized")]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_retrying_error_is_a_notice_and_the_turn_goes_on() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify(
        "error",
        json!({"threadId": THREAD, "turnId": "turn-1", "willRetry": true, "error": {"message": "Reconnecting... 1/5"}}),
    )
    .await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(THREAD),
            Event::Notice("Reconnecting... 1/5".into()),
            completed()
        ]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_rejected_turn_start_fails_that_turn_and_the_session_goes_on() {
    let mut rig = Rig::ready("one").await;
    rig.prompt("two");
    let (id, _) = rig.request("turn/start").await;
    rig.respond_error(id, "Not logged in: run codex login")
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(THREAD),
            failed(FailureKind::NotLoggedIn, "Not logged in: run codex login")
        ]
    );
    rig.start_turn("turn-2", "two").await;
    rig.complete("turn-2", "completed").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn unknown_messages_other_threads_and_stale_turns_are_ignored() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.write_raw("not json at all").await;
    rig.write_raw("").await;
    rig.write_raw("{").await;
    rig.notify(
        "thread/status/changed",
        json!({"threadId": THREAD, "status": {"type": "active"}}),
    )
    .await;
    rig.notify("something/new", json!({"x": [1, 2, 3]})).await;
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": "child-thread", "turnId": "turn-9", "itemId": "c", "delta": "child"}),
    )
    .await;
    rig.notify(
        "turn/completed",
        json!({"threadId": "child-thread", "turn": {"id": "turn-9", "status": "completed"}}),
    )
    .await;
    rig.notify(
        "turn/completed",
        json!({"threadId": THREAD, "turn": {"id": "turn-0", "status": "completed"}}),
    )
    .await;
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "m", "delta": "mine", "extra": {"deep": true}}),
    )
    .await;
    rig.complete("turn-1", "completed").await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(THREAD), text("mine"), completed()]
    );
    assert!(rig.no_events().await, "the duplicate completion is ignored");
    rig.stop().await;
}

#[tokio::test]
async fn a_late_tool_completion_without_a_start_shows_both() {
    let mut rig = Rig::ready("go").await;
    rig.start_turn("turn-1", "go").await;
    rig.notify("item/completed", command_item("exec-9", "failed", json!(2)))
        .await;
    rig.notify("item/completed", command_item("exec-9", "failed", json!(2)))
        .await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(THREAD),
            tool_started("exec-9", "git log --oneline"),
            tool_finished("exec-9", true),
            completed()
        ]
    );
    rig.stop().await;
}

#[rstest]
#[case::no_rollout("no rollout found for thread id 01a0", true)]
#[case::not_found("thread not found: 01a0", true)]
#[case::session("Session not found", true)]
#[case::invalid("invalid session id: x", true)]
#[case::permission("failed to read thread: permission denied (os error 13)", false)]
#[case::locked("rollout at /home/me/.codex/x.jsonl is locked", false)]
#[case::config("error loading config.toml: invalid type", false)]
#[case::entity("entity not found", false)]
#[tokio::test]
async fn a_failed_resume_is_session_lost_only_when_the_thread_is_gone(
    #[case] message: &str,
    #[case] lost: bool,
) {
    let mut launch = launch();
    launch.resume = Some("thread-old".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    let (id, _) = rig.request("thread/resume").await;
    rig.respond_error(id, message).await;

    let kind = if lost {
        FailureKind::SessionLost
    } else {
        FailureKind::Other
    };
    assert_eq!(rig.until_finished().await, vec![failed(kind, message)]);
    assert!(rig.reads_eof().await, "no new thread is started silently");
    rig.ends().await;
}

#[tokio::test]
async fn an_archived_thread_is_unarchived_and_resumed_once() {
    let mut launch = launch();
    launch.resume = Some("thread-old".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    let (id, _) = rig.request("thread/resume").await;
    rig.respond_error(id, "thread thread-old is archived").await;
    let (id, params) = rig.request("thread/unarchive").await;
    assert_eq!(params, json!({"threadId": "thread-old"}));
    rig.respond(id, json!({})).await;
    let (id, params) = rig.request("thread/resume").await;
    assert_eq!(params["threadId"], "thread-old");
    rig.respond(id, thread_result("thread-old")).await;
    assert_eq!(rig.next().await, session("thread-old"));
    rig.stop().await;
}

#[tokio::test]
async fn a_thread_that_stays_archived_is_an_ordinary_failure() {
    let mut launch = launch();
    launch.resume = Some("thread-old".into());
    let mut rig = Rig::start_with(launch, Limits::default());
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    let (id, _) = rig.request("thread/resume").await;
    rig.respond_error(id, "thread thread-old is archived").await;
    let (id, _) = rig.request("thread/unarchive").await;
    rig.respond(id, json!({})).await;
    let (id, _) = rig.request("thread/resume").await;
    rig.respond_error(id, "thread thread-old is archived").await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, "thread thread-old is archived")]
    );
    assert!(rig.reads_eof().await, "it is not unarchived a second time");
    rig.ends().await;
}

#[rstest]
#[case::old("nitpick/0.120.3 (Linux; x86_64)", true)]
#[case::just_below("nitpick/0.145.9 (Linux; x86_64)", true)]
#[case::minimum("nitpick/0.146.0 (Linux; x86_64)", false)]
#[case::alpha_of_the_minimum("nitpick/0.146.0-alpha.2 (Linux; x86_64)", false)]
#[case::new("nitpick/0.171.0 (Linux; x86_64)", false)]
#[case::unknown("some-wrapper build 7", false)]
#[tokio::test]
async fn an_old_cli_fails_with_a_clear_message_and_nothing_starts(
    #[case] user_agent: &str,
    #[case] too_old: bool,
) {
    let mut rig = Rig::start();
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": user_agent})).await;
    if too_old {
        let events = rig.until_finished().await;
        match &events[..] {
            [Event::TurnFinished(Outcome::Failed(failure))] => {
                assert_eq!(failure.kind, FailureKind::Other);
                assert!(failure.message.contains("too old"), "{}", failure.message);
                assert!(failure.message.contains("0.146.0"), "{}", failure.message);
            }
            other => panic!("unexpected events {other:?}"),
        }
        assert!(rig.reads_eof().await);
        rig.ends().await;
    } else {
        rig.notification("initialized").await;
        rig.request("thread/start").await;
        rig.stop().await;
    }
}

#[tokio::test]
async fn an_old_cli_is_also_caught_by_the_version_in_the_thread() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({})).await;
    rig.notification("initialized").await;
    let (id, _) = rig.request("thread/start").await;
    let mut result = thread_result(THREAD);
    result["thread"]["cliVersion"] = json!("0.100.0");
    rig.respond(id, result).await;
    let events = rig.until_finished().await;
    assert!(
        matches!(&events[..], [Event::TurnFinished(Outcome::Failed(failure))] if failure.message.contains("too old")),
        "no session is reported for a CLI that is refused: {events:?}"
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_failed_handshake_without_a_prompt_still_reports_once() {
    let mut rig = Rig::start();
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": "nitpick/0.100.0 (x)"}))
        .await;
    let events = rig.until_finished().await;
    assert_eq!(events.len(), 1);
    rig.ends().await;
}

#[tokio::test]
async fn an_initialize_error_is_an_ordinary_failure() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond_error(id, "Already initialized").await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, "Already initialized")]
    );
    rig.ends().await;
}

#[tokio::test]
async fn an_unanswered_initialize_does_not_hang_the_session() {
    let mut rig = Rig::start_with(launch(), tight());
    rig.prompt("hello");
    rig.request("initialize").await;
    let events = rig.until_finished().await;
    match &events[..] {
        [Event::TurnFinished(Outcome::Failed(failure))] => {
            assert!(
                failure.message.contains("initialize"),
                "{}",
                failure.message
            );
        }
        other => panic!("unexpected events {other:?}"),
    }
    rig.ends().await;
}

#[tokio::test]
async fn an_unanswered_thread_start_does_not_hang_the_session() {
    let mut rig = Rig::start_with(launch(), tight());
    rig.prompt("hello");
    let (id, _) = rig.request("initialize").await;
    rig.respond(id, json!({"userAgent": USER_AGENT})).await;
    rig.notification("initialized").await;
    rig.request("thread/start").await;
    let events = rig.until_finished().await;
    assert!(
        matches!(&events[..], [Event::TurnFinished(Outcome::Failed(failure))] if failure.message.contains("thread/start")),
        "{events:?}"
    );
    rig.ends().await;
}

#[tokio::test]
async fn an_unanswered_turn_start_fails_the_turn_and_the_queue_and_ends() {
    let mut rig = Rig::start_with(launch(), tight());
    rig.prompt("one");
    rig.prompt("two");
    rig.handshake().await;
    rig.request("turn/start").await;
    let first = rig.until_finished().await;
    assert_eq!(first[0], session(THREAD));
    assert!(
        matches!(&first[1..], [Event::TurnFinished(Outcome::Failed(failure))] if failure.message.contains("did not accept")),
        "{first:?}"
    );
    let second = rig.until_finished().await;
    assert!(matches!(
        &second[..],
        [Event::TurnFinished(Outcome::Failed(_))]
    ));
    rig.ends().await;
}

#[tokio::test]
async fn an_interrupt_that_is_never_honored_closes_the_session() {
    let mut rig = Rig::start_with(launch(), tight());
    rig.prompt("slow");
    rig.prompt("queued");
    rig.handshake().await;
    rig.start_turn("turn-1", "slow").await;
    rig.send(Command::Interrupt);
    rig.request("turn/interrupt").await;
    let events = rig.until_finished().await;
    assert_eq!(events[0], session(THREAD));
    assert!(matches!(events[1], Event::Notice(_)), "{events:?}");
    assert_eq!(events[2], interrupted());
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn end_of_output_mid_turn_fails_the_turn_and_every_queued_prompt() {
    let mut rig = Rig::ready("one").await;
    rig.prompt("two");
    rig.prompt("three");
    rig.start_turn("turn-1", "one").await;
    rig.notify(
        "item/started",
        command_item("exec-1", "inProgress", Value::Null),
    )
    .await;
    rig.hang_up().await;

    let first = rig.until_finished().await;
    assert_eq!(
        first,
        vec![
            session(THREAD),
            tool_started("exec-1", "git log --oneline"),
            tool_finished("exec-1", true),
            failed(FailureKind::Other, STOPPED),
        ]
    );
    for _ in 0..2 {
        assert_eq!(
            rig.until_finished().await,
            vec![failed(FailureKind::Other, STOPPED)]
        );
    }
    rig.ends().await;
}

#[tokio::test]
async fn a_prompt_sent_just_before_the_end_of_output_still_gets_its_failure() {
    let mut rig = Rig::ready("one").await;
    rig.start_turn("turn-1", "one").await;
    rig.prompt("two");
    rig.prompt("three");
    rig.hang_up().await;
    let mut finished = 0;
    while finished < 3 {
        if matches!(rig.next().await, Event::TurnFinished(Outcome::Failed(_))) {
            finished += 1;
        }
    }
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn end_of_output_between_turns_adds_no_failure() {
    let mut rig = Rig::ready("hello").await;
    rig.start_turn("turn-1", "hello").await;
    rig.complete("turn-1", "completed").await;
    assert_eq!(rig.until_finished().await.last(), Some(&completed()));
    rig.hang_up().await;
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_closed_stdin_fails_the_turn_instead_of_hanging() {
    let mut rig = Rig::start();
    rig.peer = None;
    rig.prompt("hello");
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, NOT_READY)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn shutdown_returns_at_once_and_emits_nothing_more() {
    let mut rig = Rig::ready("hello").await;
    rig.start_turn("turn-1", "hello").await;
    rig.prompt("queued");
    rig.send(Command::Shutdown);
    assert_eq!(rig.next().await, session(THREAD));
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn shutdown_during_the_handshake_returns_without_events() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.request("initialize").await;
    rig.send(Command::Shutdown);
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn dropping_the_command_channel_ends_the_driver() {
    let rig = Rig::start();
    let Rig {
        commands,
        driver,
        events,
        peer,
    } = rig;
    drop(commands);
    tokio::time::timeout(WAIT, driver).await.unwrap().unwrap();
    assert!(events.try_recv().is_err());
    drop(peer);
}

#[tokio::test]
async fn the_driver_never_emits_exited() {
    let mut rig = Rig::ready("hello").await;
    rig.start_turn("turn-1", "hello").await;
    rig.hang_up().await;
    let events = rig.until_finished().await;
    assert!(!events.iter().any(|event| matches!(event, Event::Exited(_))));
    rig.ends().await;
}
