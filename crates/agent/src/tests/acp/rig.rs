use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinHandle;

use crate::AcpFlavor;
use crate::acp::{AcpDriver, Limits};
use crate::lines::Lines;
use crate::session::{Channels, Command, Driver, Transport};
use crate::{Event, Failure, FailureKind, HarnessKind, Launch, Outcome};

pub const WAIT: Duration = Duration::from_secs(5);
pub const QUIET: Duration = Duration::from_millis(100);
pub const SESSION: &str = "ses_test";

pub const RESUME_AND_LOAD: &str = r#"{"loadSession":true,"sessionCapabilities":{"resume":{}}}"#;
pub const LOAD_ONLY: &str = r#"{"loadSession":true}"#;
pub const NOTHING: &str = "{}";
pub const SYSTEM_PROMPT: &str = r#"{"sessionCapabilities":{"systemPrompt":{}}}"#;

pub fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Acp(AcpFlavor::Generic),
        command: "agent acp".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: None,
        instructions: String::new(),
        scratch: PathBuf::new(),
    }
}

pub fn absolute_cwd() -> String {
    std::path::absolute("/work/toy")
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

pub fn relaxed() -> Limits {
    Limits {
        initialize: Duration::from_secs(20),
        session: Duration::from_secs(20),
        load: Duration::from_secs(20),
        option: Duration::from_secs(20),
        idle: Duration::from_secs(60),
        cancel: Duration::from_secs(60),
    }
}

pub fn tight() -> Limits {
    Limits {
        initialize: Duration::from_millis(2000),
        session: Duration::from_millis(500),
        load: Duration::from_millis(500),
        option: Duration::from_millis(500),
        idle: Duration::from_millis(500),
        cancel: Duration::from_millis(500),
    }
}

pub struct Peer {
    pub reader: Lines<ReadHalf<DuplexStream>>,
    pub writer: WriteHalf<DuplexStream>,
}

impl Peer {
    pub async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(WAIT, self.reader.next_line())
            .await
            .expect("the driver wrote in time")
            .unwrap()
            .expect("the driver is still connected");
        serde_json::from_str(&line).expect("the driver wrote JSON")
    }

    pub async fn reads_nothing(&mut self) -> bool {
        tokio::time::timeout(QUIET, self.reader.next_line())
            .await
            .is_err()
    }

    pub async fn reads_eof(&mut self) -> bool {
        matches!(
            tokio::time::timeout(WAIT, self.reader.next_line()).await,
            Ok(Ok(None))
        )
    }

    pub async fn write_value(&mut self, value: Value) {
        self.write_raw(&value.to_string()).await;
    }

    pub async fn write_raw(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
    }

    pub async fn hang_up(&mut self) {
        self.writer.shutdown().await.unwrap();
    }
}

pub struct Rig {
    pub commands: flume::Sender<Command>,
    pub events: flume::Receiver<Event>,
    pub peer: Option<Peer>,
    pub driver: JoinHandle<()>,
}

impl Rig {
    pub fn start(flavor: AcpFlavor) -> Self {
        Self::start_with(flavor, launch(), relaxed())
    }

    pub fn start_with(flavor: AcpFlavor, launch: Launch, limits: Limits) -> Self {
        let (driver_end, peer_end) = duplex(1 << 20);
        let (driver_reader, driver_writer) = split(driver_end);
        let (peer_reader, peer_writer) = split(peer_end);
        let (commands, command_receiver) = flume::unbounded();
        let (event_sender, events) = flume::unbounded();
        let driver = AcpDriver::new(flavor)
            .with_limits(limits)
            .with_probe(Arc::new(|_| None))
            .run(
                launch,
                Transport::new(driver_reader, driver_writer),
                Channels {
                    commands: command_receiver,
                    events: event_sender,
                },
            );
        let driver = tokio::spawn(driver);
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

    pub fn peer(&mut self) -> &mut Peer {
        self.peer.as_mut().expect("the peer is connected")
    }

    pub fn send(&self, command: Command) {
        self.commands.send(command).unwrap();
    }

    pub fn prompt(&self, text: &str) {
        self.send(Command::Prompt(text.into()));
    }

    pub async fn read(&mut self) -> Value {
        self.peer().read().await
    }

    pub async fn request(&mut self, method: &str) -> (Value, Value) {
        let message = self.read().await;
        assert_eq!(message["method"], method, "{message}");
        assert!(message.get("id").is_some(), "{method} is a request");
        (message["id"].clone(), message["params"].clone())
    }

    pub async fn notification(&mut self, method: &str) -> Value {
        let message = self.read().await;
        assert_eq!(message["method"], method, "{message}");
        assert!(message.get("id").is_none(), "{method} is a notification");
        message["params"].clone()
    }

    pub async fn write(&mut self, value: Value) {
        self.peer().write_value(value).await;
    }

    pub async fn respond(&mut self, id: Value, result: Value) {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await;
    }

    pub async fn respond_error(&mut self, id: Value, code: i64, message: &str, data: Value) {
        self.write(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message, "data": data},
        }))
        .await;
    }

    pub async fn update(&mut self, update: Value) {
        self.update_in(SESSION, update).await;
    }

    pub async fn update_in(&mut self, session: &str, update: Value) {
        self.write(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": session, "update": update},
        }))
        .await;
    }

    pub async fn say(&mut self, message: &str, text: &str) {
        self.update(json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": message,
            "content": {"type": "text", "text": text},
        }))
        .await;
    }

    pub async fn tool(&mut self, id: &str, kind: Option<&str>, status: &str, title: &str) {
        let mut update = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": title,
            "status": status,
        });
        if let Some(kind) = kind {
            update["kind"] = json!(kind);
        }
        self.update(update).await;
    }

    pub async fn tool_update(&mut self, id: &str, status: &str, title: &str) {
        self.update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": id,
            "title": title,
            "status": status,
        }))
        .await;
    }

    pub async fn hang_up(&mut self) {
        self.peer().hang_up().await;
    }

    pub async fn initialize(&mut self, capabilities: &str) -> Value {
        let (id, params) = self.request("initialize").await;
        let capabilities: Value = serde_json::from_str(capabilities).unwrap();
        self.respond(
            id,
            json!({
                "protocolVersion": 1,
                "agentCapabilities": capabilities,
                "authMethods": [],
                "agentInfo": {"name": "Agent", "version": "1.0.0"},
            }),
        )
        .await;
        params
    }

    pub async fn new_session(&mut self, options: Value) -> Value {
        let (id, params) = self.request("session/new").await;
        self.respond(id, json!({"sessionId": SESSION, "configOptions": options}))
            .await;
        params
    }

    pub async fn ready(&mut self, capabilities: &str) {
        self.initialize(capabilities).await;
        self.new_session(json!([])).await;
    }

    pub async fn read_prompt(&mut self) -> (Value, Value) {
        self.request("session/prompt").await
    }

    pub async fn finish_prompt(&mut self, id: Value, stop: &str) {
        self.respond(id, json!({"stopReason": stop})).await;
    }

    pub async fn next(&self) -> Event {
        tokio::time::timeout(WAIT, self.events.recv_async())
            .await
            .expect("an event arrived in time")
            .expect("the event channel is open")
    }

    pub async fn until_finished(&self) -> Vec<Event> {
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

    pub async fn no_events(&self) -> bool {
        !matches!(
            tokio::time::timeout(QUIET, self.events.recv_async()).await,
            Ok(Ok(_))
        )
    }

    pub async fn ends(self) {
        tokio::time::timeout(WAIT, self.driver)
            .await
            .expect("the driver returned")
            .unwrap();
    }

    pub async fn stop(self) {
        self.send(Command::Shutdown);
        self.ends().await;
    }
}

pub fn merge(events: Vec<Event>) -> Vec<Event> {
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

pub fn tool_started(id: &str, title: &str) -> Event {
    Event::ToolStarted {
        id: id.into(),
        title: title.into(),
    }
}

pub fn tool_finished(id: &str, failed: bool) -> Event {
    Event::ToolFinished {
        id: id.into(),
        failed,
    }
}

pub fn session(id: &str) -> Event {
    Event::Session { id: id.into() }
}

pub fn text(text: &str) -> Event {
    Event::Text(text.into())
}

pub fn thinking(text: &str) -> Event {
    Event::Thinking(text.into())
}

pub fn completed() -> Event {
    Event::TurnFinished(Outcome::Completed)
}

pub fn interrupted() -> Event {
    Event::TurnFinished(Outcome::Interrupted)
}

pub fn failed(kind: FailureKind, message: &str) -> Event {
    Event::TurnFinished(Outcome::Failed(Failure {
        kind,
        message: message.into(),
    }))
}
