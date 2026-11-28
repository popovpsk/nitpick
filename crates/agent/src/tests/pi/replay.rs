use std::path::PathBuf;
use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinHandle;

use crate::lines::Lines;
use crate::pi::{Limits, PiDriver};
use crate::session::{Channels, Command, Driver, Transport};
use crate::{Entry, EntryRole, Event, Failure, FailureKind, HarnessKind, Launch, Outcome};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(100);
const STATE_REQUEST_ID: &str = "nitpick-state";
const STOPPED: &str = "The assistant stopped before it finished the reply.";

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
        Self::with(None, Limits::default())
    }

    fn with(resume: Option<&str>, limits: Limits) -> Self {
        Self::with_history(resume, limits, false)
    }

    fn with_history(resume: Option<&str>, limits: Limits, history: bool) -> Self {
        let (driver_end, peer_end) = duplex(1 << 20);
        let (driver_reader, driver_writer) = split(driver_end);
        let (peer_reader, peer_writer) = split(peer_end);
        let (commands, command_receiver) = flume::unbounded();
        let (event_sender, events) = flume::unbounded();
        let launch = Launch {
            kind: HarnessKind::Pi,
            command: "pi".into(),
            cwd: PathBuf::from("/work/toy"),
            model: None,
            resume: resume.map(str::to_owned),
            history,
            instructions: String::new(),
            scratch: PathBuf::new(),
        };
        let driver = tokio::spawn(PiDriver::new().with_limits(limits).run(
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

    async fn ready(session: &str) -> Self {
        let mut rig = Self::start();
        rig.answer_state(&state(session, 0)).await;
        assert_eq!(rig.next().await, session_event(session));
        rig
    }

    async fn answer_state(&mut self, answer: &Value) {
        let request = self.read().await;
        assert_eq!(request["type"], "get_state");
        let id = request["id"].clone();
        let mut answer = answer.clone();
        answer["id"] = id;
        self.write_values(&[answer]).await;
    }

    async fn answer_state_late(&mut self, answer: &Value) {
        let mut answer = answer.clone();
        answer["id"] = json!(STATE_REQUEST_ID);
        self.write_values(&[answer]).await;
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
        serde_json::from_str(&line).expect("the driver wrote JSON")
    }

    async fn reads_nothing(&mut self) -> bool {
        tokio::time::timeout(QUIET, self.peer().reader.next_line())
            .await
            .is_err()
    }

    async fn read_end(&mut self) -> bool {
        matches!(
            tokio::time::timeout(WAIT, self.peer().reader.next_line()).await,
            Ok(Ok(None))
        )
    }

    async fn write(&mut self, lines: &[&str]) {
        for line in lines {
            let writer = &mut self.peer().writer;
            writer.write_all(line.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
        }
    }

    async fn write_values(&mut self, values: &[Value]) {
        let lines: Vec<String> = values.iter().map(Value::to_string).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.write(&refs).await;
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
                return merge_text(events);
            }
        }
    }

    async fn drain(&self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(Ok(event)) = tokio::time::timeout(QUIET, self.events.recv_async()).await {
            events.push(event);
        }
        merge_text(events)
    }

    async fn no_events(&self) -> bool {
        self.drain().await.is_empty()
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

    async fn turn(&mut self, prompt: &str, lines: &[Value]) -> Vec<Event> {
        self.prompt(prompt);
        let request = self.read().await;
        assert_eq!(request["message"], prompt);
        self.write_values(lines).await;
        self.until_finished().await
    }
}

fn merge_text(events: Vec<Event>) -> Vec<Event> {
    let mut merged: Vec<Event> = Vec::new();
    for event in events {
        match (merged.last_mut(), event) {
            (Some(Event::Text(text)), Event::Text(more)) => text.push_str(&more),
            (_, event) => merged.push(event),
        }
    }
    merged
}

fn state(session: &str, count: u64) -> Value {
    json!({"type": "response", "command": "get_state", "success": true,
        "data": {"sessionId": session, "messageCount": count, "isStreaming": false,
            "isCompacting": false, "pendingMessageCount": 0}})
}

fn accepted(id: &str) -> Value {
    json!({"id": id, "type": "response", "command": "prompt", "success": true,
        "data": {"disposition": "started"}})
}

fn delta(kind: &str, text: &str) -> Value {
    json!({"type": "message_update", "assistantMessageEvent": {"type": kind, "contentIndex": 0, "delta": text}})
}

fn assistant_end(stop: &str, error: Option<&str>) -> Value {
    let mut message = json!({"role": "assistant", "stopReason": stop, "content": []});
    if let Some(error) = error {
        message["errorMessage"] = json!(error);
    }
    json!({"type": "message_end", "message": message})
}

fn tool_start(id: &str, name: &str, args: Value) -> Value {
    json!({"type": "tool_execution_start", "toolCallId": id, "toolName": name, "args": args})
}

fn tool_end(id: &str, failed: bool) -> Value {
    json!({"type": "tool_execution_end", "toolCallId": id, "isError": failed,
        "result": {"content": [{"type": "text", "text": "out"}]}})
}

fn settled(aborted: bool) -> Value {
    json!({"type": "agent_settled", "aborted": aborted})
}

fn agent_end() -> Value {
    json!({"type": "agent_end", "messages": [], "willRetry": false})
}

fn session_event(id: &str) -> Event {
    Event::Session { id: id.into() }
}

fn text(text: &str) -> Event {
    Event::Text(text.into())
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

fn completed() -> Event {
    Event::TurnFinished(Outcome::Completed)
}

fn failed(kind: FailureKind, message: &str) -> Event {
    Event::TurnFinished(Outcome::Failed(Failure {
        kind,
        message: message.into(),
    }))
}

fn limits(state: u64, stop: u64) -> Limits {
    Limits {
        state: Duration::from_millis(state),
        stop: Duration::from_millis(stop),
    }
}

#[tokio::test]
async fn a_full_turn_ends_at_agent_settled_and_not_at_agent_end() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("explain the change");
    let request = rig.read().await;
    assert_eq!(request["type"], "prompt");
    assert_eq!(request["message"], "explain the change");
    assert_eq!(request["id"], "nitpick-prompt-1");

    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        json!({"type": "agent_start"}),
        json!({"type": "turn_start"}),
        json!({"type": "message_start", "message": {"role": "user", "content": "explain the change"}}),
        json!({"type": "message_end", "message": {"role": "user", "content": "explain the change"}}),
        json!({"type": "message_start", "message": {"role": "assistant", "content": []}}),
        delta("thinking_delta", "I should look at the log."),
        delta("text_delta", "Let me "),
        delta("text_delta", "check."),
        json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "toolUse",
            "content": [{"type": "thinking", "thinking": "I should look at the log."},
                {"type": "text", "text": "Let me check."},
                {"type": "toolCall", "id": "call_1", "name": "bash", "arguments": {"command": "git log --oneline"}}]}}),
        tool_start("call_1", "bash", json!({"command": "git log --oneline"})),
        json!({"type": "tool_execution_update", "toolCallId": "call_1", "partialResult": {"content": []}}),
        tool_end("call_1", false),
        json!({"type": "message_start", "message": {"role": "toolResult"}}),
        json!({"type": "message_end", "message": {"role": "toolResult", "content": [{"type": "text", "text": "out"}]}}),
        json!({"type": "turn_end"}),
        json!({"type": "turn_start"}),
        json!({"type": "message_start", "message": {"role": "assistant", "content": []}}),
        delta("text_delta", "Two commits."),
        json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "stop",
            "content": [{"type": "text", "text": "Two commits."}]}}),
        json!({"type": "turn_end"}),
        agent_end(),
    ])
    .await;

    assert_eq!(
        rig.drain().await,
        vec![
            Event::Thinking("I should look at the log.".into()),
            text("Let me check."),
            tool_started("call_1", "git log --oneline"),
            tool_finished("call_1", false),
            text("Two commits."),
        ]
    );

    rig.write_values(&[settled(false)]).await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn the_state_is_checked_before_any_prompt_is_written() {
    let mut rig = Rig::start();
    rig.prompt("early");
    let request = rig.read().await;
    assert_eq!(request["type"], "get_state");
    assert!(rig.reads_nothing().await);
    let id = request["id"].clone();
    let mut answer = state("nitpick-s1", 0);
    answer["id"] = id;
    rig.write_values(&[answer]).await;
    assert_eq!(rig.next().await, session_event("nitpick-s1"));
    assert_eq!(rig.read().await["message"], "early");
    rig.stop().await;
}

#[tokio::test]
async fn a_new_session_reports_the_id_pi_gave_it() {
    let mut rig = Rig::start();
    rig.answer_state(&state("nitpick-fresh", 0)).await;
    assert_eq!(rig.next().await, session_event("nitpick-fresh"));
    rig.stop().await;
}

#[tokio::test]
async fn a_new_session_with_no_reported_id_works_but_reports_none() {
    let mut rig = Rig::start();
    rig.answer_state(
        &json!({"type": "response", "command": "get_state", "success": true, "data": {}}),
    )
    .await;
    let events = rig
        .turn(
            "hi",
            &[
                accepted("nitpick-prompt-1"),
                delta("text_delta", "yo"),
                settled(false),
            ],
        )
        .await;
    assert_eq!(events, vec![text("yo"), completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn a_resume_of_a_session_with_messages_is_a_normal_session() {
    let mut rig = Rig::with(Some("nitpick-here"), Limits::default());
    rig.answer_state(&state("nitpick-here", 6)).await;
    assert_eq!(rig.next().await, session_event("nitpick-here"));
    let events = rig
        .turn(
            "again",
            &[
                accepted("nitpick-prompt-1"),
                delta("text_delta", "Welcome back."),
                settled(false),
            ],
        )
        .await;
    assert_eq!(events, vec![text("Welcome back."), completed()]);
    rig.stop().await;
}

#[rstest]
#[case::empty_session(json!({"sessionId": "nitpick-gone", "messageCount": 0}))]
#[case::no_count(json!({"sessionId": "nitpick-gone"}))]
#[case::no_data(Value::Null)]
#[tokio::test]
async fn a_resume_without_history_is_a_lost_session(#[case] data: Value) {
    let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
    rig.prompt("hello");
    rig.answer_state(
        &json!({"type": "response", "command": "get_state", "success": true, "data": data}),
    )
    .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::SessionLost,
            "Pi has no history for the session nitpick-gone."
        )]
    );
    assert!(
        rig.read_end().await,
        "the prompt must not reach a fresh session"
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_lost_session_with_no_prompt_pending_still_reports_once() {
    let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
    rig.answer_state(&state("nitpick-gone", 0)).await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::SessionLost,
            "Pi has no history for the session nitpick-gone."
        )]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn every_prompt_queued_during_the_check_gets_the_lost_session() {
    let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
    rig.prompt("one");
    rig.prompt("two");
    rig.answer_state(&state("nitpick-gone", 0)).await;
    let lost = failed(
        FailureKind::SessionLost,
        "Pi has no history for the session nitpick-gone.",
    );
    assert_eq!(rig.until_finished().await, vec![lost.clone()]);
    assert_eq!(rig.until_finished().await, vec![lost]);
    rig.ends().await;
}

#[tokio::test]
async fn a_failed_state_request_fails_the_turn_with_its_classification() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.answer_state(&json!({"type": "response", "command": "get_state", "success": false, "error": "No API key found"}))
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::NotLoggedIn, "No API key found")]
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_silent_get_state_hits_the_bounded_wait() {
    let mut rig = Rig::with(None, limits(150, 150));
    rig.prompt("hello");
    assert_eq!(rig.read().await["type"], "get_state");
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "Pi did not answer get_state within 150ms."
        )]
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_silent_get_state_with_no_prompt_still_reports_once() {
    let mut rig = Rig::with(None, limits(150, 150));
    assert_eq!(rig.read().await["type"], "get_state");
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "Pi did not answer get_state within 150ms."
        )]
    );
    rig.ends().await;
}

#[tokio::test]
async fn exit_before_the_state_answer_fails_the_queued_prompt() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.hang_up().await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "Pi stopped before the session was ready."
        )]
    );
    rig.ends().await;
}

#[tokio::test]
async fn dialog_requests_are_cancelled_and_notifications_are_not_answered() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("go");
    rig.read().await;
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        json!({"type": "extension_ui_request", "id": "n-1", "method": "notify", "message": "hi"}),
        json!({"type": "extension_ui_request", "id": "ui-1", "method": "confirm", "title": "Run?", "message": "ok?"}),
    ])
    .await;
    assert_eq!(
        rig.read().await,
        json!({"type": "extension_ui_response", "id": "ui-1", "cancelled": true})
    );
    assert!(rig.reads_nothing().await);
    rig.write_values(&[delta("text_delta", "done"), settled(false)])
        .await;
    assert_eq!(rig.until_finished().await, vec![text("done"), completed()]);

    rig.write_values(&[
        json!({"type": "extension_ui_request", "id": "ui-2", "method": "input", "title": "t"}),
    ])
    .await;
    assert_eq!(
        rig.read().await,
        json!({"type": "extension_ui_response", "id": "ui-2", "cancelled": true})
    );
    rig.stop().await;
}

#[tokio::test]
async fn stop_sends_clear_queue_then_abort_and_the_turn_ends_interrupted() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("long");
    rig.prompt("after");
    rig.read().await;
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        tool_start("c1", "bash", json!({"command": "sleep 100"})),
    ])
    .await;
    assert_eq!(rig.next().await, tool_started("c1", "sleep 100"));

    rig.send(Command::Interrupt);
    assert_eq!(rig.read().await["type"], "clear_queue");
    assert_eq!(rig.read().await["type"], "abort");
    assert!(rig.reads_nothing().await);

    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await, "one stop per turn");

    rig.write_values(&[assistant_end("aborted", None), agent_end(), settled(true)])
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            tool_finished("c1", true),
            Event::TurnFinished(Outcome::Interrupted)
        ]
    );

    let next = rig.read().await;
    assert_eq!(next["type"], "prompt");
    assert_eq!(next["message"], "after");
    rig.write_values(&[settled(false)]).await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_with_no_turn_running_writes_nothing() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await);
    rig.prompt("hello");
    assert_eq!(rig.read().await["type"], "prompt");
    rig.stop().await;
}

#[tokio::test]
async fn an_abort_that_never_takes_effect_ends_the_turn_and_the_session() {
    let mut rig = Rig::with(None, limits(5_000, 200));
    rig.answer_state(&state("nitpick-s1", 0)).await;
    assert_eq!(rig.next().await, session_event("nitpick-s1"));
    rig.prompt("long");
    rig.prompt("queued");
    rig.read().await;
    rig.send(Command::Interrupt);
    assert_eq!(rig.read().await["type"], "clear_queue");
    assert_eq!(rig.read().await["type"], "abort");
    assert_eq!(
        rig.next().await,
        Event::Notice("Pi did not stop in time. The session was closed.".into())
    );
    assert_eq!(
        rig.until_finished().await,
        vec![Event::TurnFinished(Outcome::Interrupted)]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_stop_that_works_in_time_leaves_the_timer_behind() {
    let mut rig = Rig::with(None, limits(5_000, 300));
    rig.answer_state(&state("nitpick-s1", 0)).await;
    rig.next().await;
    rig.prompt("one");
    rig.read().await;
    rig.send(Command::Interrupt);
    rig.read().await;
    rig.read().await;
    rig.write_values(&[assistant_end("aborted", None), settled(true)])
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![Event::TurnFinished(Outcome::Interrupted)]
    );
    rig.prompt("two");
    rig.read().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        rig.no_events().await,
        "the old deadline must not fire in the next turn"
    );
    rig.stop().await;
}

#[tokio::test]
async fn an_error_followed_by_a_successful_retry_completes() {
    let mut rig = Rig::ready("nitpick-s1").await;
    let events = rig
        .turn(
            "go",
            &[
                accepted("nitpick-prompt-1"),
                json!({"type": "message_start", "message": {"role": "assistant"}}),
                assistant_end("error", Some("529 overloaded")),
                json!({"type": "agent_end", "messages": [], "willRetry": true}),
                json!({"type": "auto_retry_start", "attempt": 1, "maxAttempts": 3, "delayMs": 2000, "errorMessage": "529 overloaded"}),
                json!({"type": "auto_retry_end", "success": true, "attempt": 1}),
                json!({"type": "message_start", "message": {"role": "assistant"}}),
                delta("text_delta", "Recovered."),
                assistant_end("stop", None),
                agent_end(),
                settled(false),
            ],
        )
        .await;
    assert_eq!(
        events,
        vec![
            Event::Notice("Retrying the request (attempt 1 of 3): 529 overloaded".into()),
            text("Recovered."),
            completed(),
        ]
    );
    rig.stop().await;
}

#[rstest]
#[case::auth("401 Unauthorized: invalid x-api-key", FailureKind::NotLoggedIn)]
#[case::missing_key("No API key found for anthropic", FailureKind::NotLoggedIn)]
#[case::usage("429 rate limit exceeded", FailureKind::UsageLimit)]
#[case::context(
    "prompt is too long: 250000 tokens > 200000 maximum",
    FailureKind::ContextTooLong
)]
#[case::model("Model not found: sonet-9", FailureKind::ModelNotFound)]
#[case::other("connection reset by peer", FailureKind::Other)]
#[tokio::test]
async fn an_error_that_sticks_fails_with_the_classified_kind(
    #[case] message: &str,
    #[case] kind: FailureKind,
) {
    let mut rig = Rig::ready("nitpick-s1").await;
    let events = rig
        .turn(
            "go",
            &[
                accepted("nitpick-prompt-1"),
                assistant_end("error", Some(message)),
                json!({"type": "auto_retry_start", "attempt": 1, "maxAttempts": 1, "delayMs": 10, "errorMessage": message}),
                json!({"type": "auto_retry_end", "success": false, "attempt": 1, "finalError": message}),
                assistant_end("error", Some(message)),
                agent_end(),
                settled(false),
            ],
        )
        .await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(events[0], Event::Notice(_)));
    assert_eq!(events[1], failed(kind, message));
    rig.stop().await;
}

#[tokio::test]
async fn a_rejected_prompt_fails_its_turn_and_the_next_one_goes_out() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("one");
    rig.prompt("two");
    rig.read().await;
    rig.write_values(&[
        json!({"id": "nitpick-prompt-1", "type": "response", "command": "prompt",
        "success": false, "error": "No API key found for openai"}),
    ])
    .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::NotLoggedIn,
            "No API key found for openai"
        )]
    );
    let next = rig.read().await;
    assert_eq!(next["message"], "two");
    assert_eq!(next["id"], "nitpick-prompt-2");
    rig.stop().await;
}

#[tokio::test]
async fn a_prompt_an_extension_handled_completes_without_a_run() {
    let mut rig = Rig::ready("nitpick-s1").await;
    let events = rig
        .turn(
            "/status",
            &[
                json!({"id": "nitpick-prompt-1", "type": "response", "command": "prompt",
                "success": true, "data": {"disposition": "handled"}}),
            ],
        )
        .await;
    assert_eq!(events, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn prompts_go_out_one_at_a_time_in_order() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("first");
    rig.prompt("second");
    rig.prompt("third");

    assert_eq!(rig.read().await["message"], "first");
    assert!(rig.reads_nothing().await);
    rig.write_values(&[accepted("nitpick-prompt-1"), agent_end()])
        .await;
    assert!(
        rig.reads_nothing().await,
        "agent_end is not the end of the turn"
    );
    rig.write_values(&[settled(false)]).await;
    assert_eq!(rig.until_finished().await, vec![completed()]);

    assert_eq!(rig.read().await["message"], "second");
    assert!(rig.reads_nothing().await);
    rig.write_values(&[settled(false)]).await;
    assert_eq!(rig.until_finished().await, vec![completed()]);

    assert_eq!(rig.read().await["message"], "third");
    rig.write_values(&[settled(false)]).await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn a_stray_settle_with_no_turn_running_is_ignored() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.write_values(&[settled(false), settled(true)]).await;
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn garbage_and_unknown_events_between_frames_are_skipped() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("hello");
    rig.read().await;
    rig.write(&["warning: something on stdout", "", "{", "[1]"])
        .await;
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        json!({"type": "queue_update", "steering": [], "followUp": []}),
        json!({"type": "compaction_start", "reason": "threshold"}),
        json!({"type": "extension_error", "extensionPath": "/x.ts", "event": "tool_call", "error": "boom"}),
        json!({"type": "something_new", "payload": {"deep": [1, 2, 3]}}),
        delta("text_delta", "fine"),
        settled(false),
    ])
    .await;
    assert_eq!(rig.until_finished().await, vec![text("fine"), completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn end_of_output_mid_turn_fails_the_turn_and_every_queued_prompt() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("one");
    rig.prompt("two");
    rig.prompt("three");
    rig.read().await;
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        tool_start("c1", "read", json!({"path": "src/lib.rs"})),
    ])
    .await;
    rig.hang_up().await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            tool_started("c1", "src/lib.rs"),
            tool_finished("c1", true),
            failed(FailureKind::Other, STOPPED),
        ]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn prompts_still_unread_when_the_output_ends_each_get_a_failure() {
    for _ in 0..20 {
        let mut rig = Rig::ready("nitpick-s1").await;
        rig.prompt("one");
        rig.prompt("two");
        rig.prompt("three");
        rig.hang_up().await;
        let mut failures = 0;
        while failures < 3 {
            let event = rig.next().await;
            assert_eq!(event, failed(FailureKind::Other, STOPPED));
            failures += 1;
        }
        rig.ends().await;
    }
}

#[tokio::test]
async fn end_of_output_between_turns_adds_no_failure() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.turn("hello", &[accepted("nitpick-prompt-1"), settled(false)])
        .await;
    rig.hang_up().await;
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_closed_stdin_fails_the_turn_instead_of_hanging() {
    let mut rig = Rig::ready("nitpick-s1").await;
    let Peer { reader, writer } = rig.peer.take().unwrap();
    drop(reader);
    rig.prompt("hello");
    drop(writer);
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn shutdown_returns_at_once_and_emits_nothing_more() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("hello");
    rig.read().await;
    rig.send(Command::Shutdown);
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn shutdown_before_the_state_answer_emits_nothing() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Shutdown);
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn dropping_the_command_channel_ends_the_driver() {
    let rig = Rig::ready("nitpick-s1").await;
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

fn interrupted() -> Event {
    Event::TurnFinished(Outcome::Interrupted)
}

fn cancelled() -> Event {
    Event::TurnFinished(Outcome::Cancelled)
}

#[tokio::test]
async fn an_interrupt_while_get_state_is_pending_cancels_the_queued_prompt() {
    let mut rig = Rig::start();
    rig.prompt("overview");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Interrupt);
    assert_eq!(rig.until_finished().await, vec![cancelled()]);

    rig.answer_state_late(&state("nitpick-s1", 0)).await;
    assert_eq!(rig.next().await, session_event("nitpick-s1"));
    assert!(
        rig.reads_nothing().await,
        "the cancelled prompt is never written"
    );
    assert!(rig.no_events().await);

    let events = rig
        .turn("again", &[accepted("nitpick-prompt-1"), settled(false)])
        .await;
    assert_eq!(events, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn only_the_oldest_queued_prompt_is_cancelled() {
    let mut rig = Rig::start();
    rig.prompt("first");
    rig.prompt("second");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Interrupt);
    assert_eq!(rig.until_finished().await, vec![cancelled()]);

    rig.answer_state_late(&state("nitpick-s1", 0)).await;
    assert_eq!(rig.next().await, session_event("nitpick-s1"));
    let request = rig.read().await;
    assert_eq!(request["message"], "second");
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        delta("text_delta", "ok"),
        settled(false),
    ])
    .await;
    assert_eq!(rig.until_finished().await, vec![text("ok"), completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_arriving_with_the_state_answer_never_lets_the_prompt_through() {
    for _ in 0..20 {
        let mut rig = Rig::start();
        rig.prompt("first");
        rig.prompt("second");
        assert_eq!(rig.read().await["type"], "get_state");
        rig.answer_state_late(&state("nitpick-s1", 0)).await;
        rig.send(Command::Interrupt);
        let first = rig.next().await;
        let second = rig.next().await;
        assert!(
            (first == session_event("nitpick-s1") && second == cancelled())
                || (first == cancelled() && second == session_event("nitpick-s1")),
            "{first:?} {second:?}"
        );
        assert_eq!(rig.read().await["message"], "second");
        rig.stop().await;
    }
}

#[tokio::test]
async fn an_interrupt_after_the_prompt_is_written_stops_the_turn_before_pis_response() {
    let mut rig = Rig::ready("nitpick-s1").await;
    rig.prompt("overview");
    assert_eq!(rig.read().await["type"], "prompt");
    rig.send(Command::Interrupt);
    assert_eq!(rig.read().await["type"], "clear_queue");
    assert_eq!(rig.read().await["type"], "abort");
    rig.write_values(&[
        accepted("nitpick-prompt-1"),
        assistant_end("aborted", None),
        settled(true),
    ])
    .await;
    assert_eq!(rig.until_finished().await, vec![interrupted()]);
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_with_nothing_queued_is_a_no_op() {
    let mut rig = Rig::start();
    rig.send(Command::Interrupt);
    rig.answer_state(&state("nitpick-s1", 0)).await;
    assert_eq!(rig.next().await, session_event("nitpick-s1"));
    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await);
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_unread_when_the_session_turns_out_lost_cancels_instead_of_failing() {
    for _ in 0..20 {
        let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
        rig.prompt("overview");
        assert_eq!(rig.read().await["type"], "get_state");
        rig.send(Command::Interrupt);
        rig.answer_state_late(&state("nitpick-gone", 0)).await;
        assert_eq!(rig.next().await, cancelled());
        assert_eq!(
            rig.next().await,
            failed(
                FailureKind::SessionLost,
                "Pi has no history for the session nitpick-gone."
            )
        );
        rig.ends().await;
    }
}

#[tokio::test]
async fn an_interrupt_unread_when_the_stream_ends_cancels_the_oldest_prompt_only() {
    for _ in 0..20 {
        let mut rig = Rig::start();
        rig.prompt("first");
        rig.prompt("second");
        assert_eq!(rig.read().await["type"], "get_state");
        rig.send(Command::Interrupt);
        rig.hang_up().await;
        assert_eq!(rig.next().await, cancelled());
        assert_eq!(
            rig.next().await,
            failed(
                FailureKind::Other,
                "Pi stopped before the session was ready."
            )
        );
        assert!(rig.no_events().await);
        rig.ends().await;
    }
}

#[tokio::test]
async fn a_lost_session_after_a_cancel_still_reports_a_prompt_that_is_on_its_way() {
    let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
    rig.prompt("overview");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Interrupt);
    assert_eq!(rig.until_finished().await, vec![cancelled()]);
    rig.answer_state_late(&state("nitpick-gone", 0)).await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::SessionLost,
            "Pi has no history for the session nitpick-gone."
        )]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_prompt_sent_after_a_cancel_gets_the_setup_failure_exactly_once() {
    let mut rig = Rig::with(Some("nitpick-gone"), Limits::default());
    rig.prompt("overview");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Interrupt);
    assert_eq!(rig.until_finished().await, vec![cancelled()]);
    rig.prompt("again");
    rig.answer_state_late(&state("nitpick-gone", 0)).await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::SessionLost,
            "Pi has no history for the session nitpick-gone."
        )]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_state_timeout_after_a_cancel_still_reports_a_prompt_that_is_on_its_way() {
    let mut rig = Rig::with(None, limits(150, 150));
    rig.prompt("overview");
    assert_eq!(rig.read().await["type"], "get_state");
    rig.send(Command::Interrupt);
    assert_eq!(rig.until_finished().await, vec![cancelled()]);
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "Pi did not answer get_state within 150ms."
        )]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

fn history_entry(role: EntryRole, text: &str) -> Entry {
    Entry {
        role,
        text: text.into(),
    }
}

#[tokio::test]
async fn a_resume_for_history_reads_the_messages_before_the_session() {
    let mut rig = Rig::with_history(Some("nitpick-stored"), Limits::default(), true);
    rig.answer_state(&state("nitpick-stored", 6)).await;
    let request = rig.read().await;
    assert_eq!(
        request,
        json!({"id": "nitpick-history", "type": "get_messages"})
    );
    rig.write_values(&[json!({"id": "nitpick-history", "type": "response",
        "command": "get_messages", "success": true, "data": {"messages": [
            {"role": "system", "content": "rules"},
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": [{"type": "thinking", "thinking": "hm"},
                {"type": "text", "text": "Look"}, {"type": "text", "text": "ing."},
                {"type": "toolCall", "id": "t", "name": "bash", "arguments": {}}],
                "stopReason": "toolUse"},
            {"role": "toolResult", "toolCallId": "t", "content": [{"type": "text", "text": "out"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Done."}], "stopReason": "stop"},
            {"role": "user", "content": [{"type": "text", "text": "second"}, {"type": "image", "data": ""}]},
            {"role": "assistant", "content": [{"type": "text", "text": "API error"}],
                "stopReason": "error", "errorMessage": "boom"},
            {"role": "bashExecution", "command": "ls"},
        ]}})])
    .await;
    assert_eq!(
        rig.next().await,
        Event::History(vec![
            history_entry(EntryRole::User, "first"),
            history_entry(EntryRole::Assistant, "Looking.\n\nDone."),
            history_entry(EntryRole::User, "second"),
        ])
    );
    assert_eq!(rig.next().await, session_event("nitpick-stored"));
    rig.stop().await;
}

#[tokio::test]
async fn a_refused_message_list_still_resumes() {
    let mut rig = Rig::with_history(Some("nitpick-stored"), Limits::default(), true);
    rig.answer_state(&state("nitpick-stored", 2)).await;
    rig.read().await;
    rig.write_values(&[json!({"id": "nitpick-history", "type": "response",
        "command": "get_messages", "success": false, "error": "no"})])
        .await;
    assert_eq!(rig.next().await, Event::History(Vec::new()));
    assert_eq!(rig.next().await, session_event("nitpick-stored"));
    rig.stop().await;
}

#[tokio::test]
async fn a_resume_without_history_asks_for_no_messages() {
    let mut rig = Rig::with(Some("nitpick-stored"), Limits::default());
    rig.answer_state(&state("nitpick-stored", 2)).await;
    assert_eq!(rig.next().await, session_event("nitpick-stored"));
    assert!(rig.reads_nothing().await);
    rig.stop().await;
}
