use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinHandle;

use crate::claude::ClaudeDriver;
use crate::lines::Lines;
use crate::session::{Channels, Command, Driver, Transport};
use crate::{Event, Failure, FailureKind, HarnessKind, Launch, Outcome};

const BASIC: &str = include_str!("../../fixtures/claude/basic.jsonl");
const WRITE: &str = include_str!("../../fixtures/claude/write.jsonl");
const INTERRUPT: &str = include_str!("../../fixtures/claude/interrupt_then.jsonl");
const RESUME: &str = include_str!("../../fixtures/claude/resume.jsonl");
const RESUME_MISSING: &str = include_str!("../../fixtures/claude/resume_missing.jsonl");
const RESUME_MISSING_STDERR: &str = include_str!("../../fixtures/claude/resume_missing.stderr");

const BASIC_SESSION: &str = "951f14f9-9400-4688-a95a-733f42893977";
const WRITE_SESSION: &str = "83c99404-86db-4d7b-a5c2-5e3b8a3676ae";
const INTERRUPT_SESSION: &str = "545f89c4-5ada-404f-805a-772dc1d6c2c5";
const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(100);

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
        let (driver_end, peer_end) = duplex(1 << 20);
        let (driver_reader, driver_writer) = split(driver_end);
        let (peer_reader, peer_writer) = split(peer_end);
        let (commands, command_receiver) = flume::unbounded();
        let (event_sender, events) = flume::unbounded();
        let launch = Launch {
            kind: HarnessKind::Claude,
            command: "claude".into(),
            cwd: PathBuf::from("/work/toy"),
            model: None,
            resume: None,
            history: false,
            instructions: String::new(),
            scratch: PathBuf::new(),
        };
        let driver = tokio::spawn(ClaudeDriver::new().run(
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
        serde_json::from_str(&line).expect("the driver wrote JSON")
    }

    async fn reads_nothing(&mut self) -> bool {
        tokio::time::timeout(QUIET, self.peer().reader.next_line())
            .await
            .is_err()
    }

    async fn write(&mut self, lines: &[&str]) {
        for line in lines {
            let writer = &mut self.peer().writer;
            writer.write_all(line.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
        }
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

    async fn replay(fixture: &str, prompts: &[&str]) -> Vec<Vec<Event>> {
        let mut rig = Rig::start();
        for prompt in prompts {
            rig.prompt(prompt);
        }
        let mut all = Vec::new();
        for (turn, prompt) in turns(fixture).iter().zip(prompts) {
            assert_eq!(rig.read().await["message"]["content"], *prompt);
            rig.write(turn).await;
            all.push(rig.until_finished().await);
        }
        assert!(rig.no_events().await);
        rig.stop().await;
        all
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

fn turns(fixture: &str) -> Vec<Vec<&str>> {
    let mut turns = vec![Vec::new()];
    for line in fixture.lines() {
        turns.last_mut().unwrap().push(line);
        if line.contains(r#""type":"result""#) {
            turns.push(Vec::new());
        }
    }
    turns.retain(|turn| !turn.is_empty());
    turns
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

fn completed() -> Event {
    Event::TurnFinished(Outcome::Completed)
}

fn failed(kind: FailureKind, message: &str) -> Event {
    Event::TurnFinished(Outcome::Failed(Failure {
        kind,
        message: message.into(),
    }))
}

#[test]
fn the_fixtures_split_into_the_expected_turns() {
    assert_eq!(turns(BASIC).len(), 2);
    assert_eq!(turns(WRITE).len(), 2);
    assert_eq!(turns(INTERRUPT).len(), 2);
    assert_eq!(turns(RESUME).len(), 1);
    assert_eq!(turns(RESUME_MISSING).len(), 1);
}

#[tokio::test]
async fn a_tool_turn_then_a_text_turn() {
    let all = Rig::replay(BASIC, &["list the commits", "run a write"]).await;
    assert_eq!(
        all[0],
        vec![
            session(BASIC_SESSION),
            tool_started(
                "toolu_01AuNYsBweKifLcNeJQxgdvs",
                r#"git log --oneline && echo "---" && git rev-list --count HEAD"#
            ),
            tool_finished("toolu_01AuNYsBweKifLcNeJQxgdvs", false),
            text(
                "There are **2 commits** on the current branch (`main`):\n\n- `dee51e2` Wrap on overflow, add sub()\n- `da36760` Add add()"
            ),
            completed(),
        ]
    );
    match &all[1][..] {
        [Event::Text(answer), Event::TurnFinished(Outcome::Completed)] => {
            assert!(answer.starts_with("I didn't run that."), "{answer}");
        }
        other => panic!("the second turn repeats the init frame and thinks silently: {other:?}"),
    }
}

#[tokio::test]
async fn a_blocked_write_is_an_ordinary_tool_turn() {
    let all = Rig::replay(WRITE, &["write a file", "any rule?"]).await;
    assert_eq!(all[0].len(), 5, "{:?}", all[0]);
    assert_eq!(all[0][0], session(WRITE_SESSION));
    assert_eq!(
        all[0][1],
        tool_started(
            "toolu_01GaF3s4qdeoAmdvHJCoyEWk",
            "echo hi > hello.txt; echo status=$?; touch /tmp/nitpick-probe-outside; echo outside=$?"
        )
    );
    assert_eq!(
        all[0][2],
        tool_finished("toolu_01GaF3s4qdeoAmdvHJCoyEWk", false)
    );
    match &all[0][3] {
        Event::Text(answer) => assert!(answer.contains("operation not permitted"), "{answer}"),
        other => panic!("unexpected event {other:?}"),
    }
    assert_eq!(all[0][4], completed());
    assert_eq!(all[1], vec![text("NO RULE"), completed()]);
}

#[tokio::test]
async fn a_resumed_conversation_replays_like_a_new_one() {
    let all = Rig::replay(RESUME, &["what was my first question?"]).await;
    assert_eq!(
        all[0],
        vec![
            session(BASIC_SESSION),
            text(
                "The first thing you asked was for me to run `git log --oneline` and tell you how many commits there are."
            ),
            completed(),
        ]
    );
}

#[tokio::test]
async fn the_second_prompt_waits_for_the_result_of_the_first() {
    let mut rig = Rig::start();
    let basic = turns(BASIC);
    rig.prompt("first");
    rig.prompt("second");

    assert_eq!(rig.read().await["message"]["content"], "first");
    assert!(rig.reads_nothing().await);

    let (result, rest) = basic[0].split_last().unwrap();
    rig.write(rest).await;
    assert!(rig.reads_nothing().await, "still no result");
    rig.write(&[result]).await;
    assert_eq!(rig.until_finished().await.last(), Some(&completed()));
    assert_eq!(rig.read().await["message"]["content"], "second");

    rig.write(&basic[1]).await;
    assert_eq!(rig.until_finished().await.last(), Some(&completed()));
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_is_not_a_failure_and_the_process_takes_the_next_prompt() {
    let mut rig = Rig::start();
    let interrupt = turns(INTERRUPT);
    let (before, after) = interrupt[0].split_at(25);

    rig.prompt("sleep for a while");
    assert_eq!(rig.read().await["type"], "user");
    rig.write(before).await;
    assert_eq!(rig.next().await, session(INTERRUPT_SESSION));
    assert!(matches!(rig.next().await, Event::ToolStarted { .. }));
    rig.send(Command::Interrupt);

    let request = rig.read().await;
    assert_eq!(request["type"], "control_request");
    assert_eq!(request["request"]["subtype"], "interrupt");
    assert!(
        request["request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );

    rig.write(after).await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            tool_finished("toolu_01XaG773qyGfbToeQiQRifmN", true),
            Event::TurnFinished(Outcome::Interrupted),
        ]
    );

    rig.prompt("say OK");
    assert_eq!(rig.read().await["message"]["content"], "say OK");
    rig.write(&interrupt[1]).await;
    assert_eq!(rig.until_finished().await, vec![text("OK"), completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_with_no_turn_running_writes_nothing() {
    let mut rig = Rig::start();
    rig.send(Command::Interrupt);
    assert!(rig.reads_nothing().await);
    rig.prompt("hello");
    assert_eq!(rig.read().await["type"], "user");
    rig.stop().await;
}

#[tokio::test]
async fn a_missing_session_is_session_lost_with_the_cli_message() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.read().await;
    rig.write(&[RESUME_MISSING.trim_end()]).await;
    let message = RESUME_MISSING_STDERR.trim();
    assert!(message.starts_with("No conversation found"));
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::SessionLost, message)]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_broken_result_line_still_ends_the_turn_and_the_queue_moves_on() {
    let mut rig = Rig::start();
    rig.prompt("one");
    rig.prompt("two");
    rig.read().await;
    rig.write(&[r#"{"type":"result","subtype":"success","result":"unterminated"#])
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "The assistant's final status could not be read."
        )]
    );
    assert_eq!(rig.read().await["message"]["content"], "two");
    rig.stop().await;
}

#[tokio::test]
async fn garbage_between_frames_is_skipped() {
    let mut rig = Rig::start();
    let resume = turns(RESUME);
    rig.prompt("hello");
    rig.read().await;
    rig.write(&["warning: something on stdout", "", "{"]).await;
    rig.write(&resume[0]).await;
    let events = rig.until_finished().await;
    assert_eq!(events.first(), Some(&session(BASIC_SESSION)));
    assert_eq!(events.last(), Some(&completed()));
    rig.stop().await;
}

#[tokio::test]
async fn a_tool_permission_request_is_denied_on_stdin() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.read().await;
    rig.write(&[
        r#"{"type":"control_request","request_id":"req-9","request":{"subtype":"can_use_tool","tool_name":"Write","input":{}}}"#,
    ])
    .await;
    let reply = rig.read().await;
    assert_eq!(reply["type"], "control_response");
    assert_eq!(reply["response"]["request_id"], "req-9");
    assert_eq!(reply["response"]["response"]["behavior"], "deny");
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn end_of_output_mid_turn_fails_the_turn_and_every_queued_prompt() {
    let mut rig = Rig::start();
    let basic = turns(BASIC);
    rig.prompt("one");
    rig.prompt("two");
    rig.read().await;
    let tool_call = basic[0]
        .iter()
        .position(|line| line.starts_with(r#"{"type":"assistant""#))
        .unwrap();
    rig.write(&basic[0][..=tool_call + 2]).await;
    rig.hang_up().await;

    let message = "The assistant stopped before it finished the reply.";
    let first = rig.until_finished().await;
    assert_eq!(
        first,
        vec![
            session(BASIC_SESSION),
            tool_started(
                "toolu_01AuNYsBweKifLcNeJQxgdvs",
                r#"git log --oneline && echo "---" && git rev-list --count HEAD"#
            ),
            tool_finished("toolu_01AuNYsBweKifLcNeJQxgdvs", true),
            failed(FailureKind::Other, message),
        ]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, message)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn end_of_output_between_turns_adds_no_failure() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.read().await;
    rig.write(&turns(RESUME)[0]).await;
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
        vec![failed(
            FailureKind::Other,
            "The assistant stopped before it finished the reply."
        )]
    );
    rig.ends().await;
}

#[tokio::test]
async fn shutdown_returns_at_once_and_emits_nothing_more() {
    let mut rig = Rig::start();
    rig.prompt("hello");
    rig.read().await;
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
