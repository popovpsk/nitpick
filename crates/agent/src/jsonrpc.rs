use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::lines::Lines;

const CLOSE_WAIT: Duration = Duration::from_secs(2);
const LOG_EXCERPT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Id {
    Number(i64),
    String(String),
}

impl Id {
    pub fn to_value(&self) -> Value {
        match self {
            Id::Number(number) => json!(number),
            Id::String(text) => json!(text),
        }
    }

    pub fn from_value(value: &Value) -> Option<Id> {
        match value {
            Value::Number(number) => number.as_i64().map(Id::Number),
            Value::String(text) => Some(Id::String(text.clone())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("rpc error {code}: {message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CallError {
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("the connection is closed")]
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Id,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub tag_version: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { tag_version: true }
    }
}

type Pending = HashMap<i64, oneshot::Sender<Result<Value, RpcError>>>;

struct Shared {
    pending: Mutex<Option<Pending>>,
    next_id: AtomicI64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        let pending = self.lock().take();
        drop(pending);
    }
}

enum Out {
    Line(String),
    Close,
}

struct PendingGuard<'a> {
    shared: &'a Shared,
    id: i64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if let Some(pending) = self.shared.lock().as_mut() {
            pending.remove(&self.id);
        }
    }
}

#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
    out: flume::Sender<Out>,
    options: Options,
}

impl Client {
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        match self.shared.lock().as_mut() {
            Some(pending) => {
                pending.insert(id, sender);
            }
            None => return Err(CallError::Closed),
        }
        let _guard = PendingGuard {
            shared: &self.shared,
            id,
        };
        let mut message = self.envelope();
        message.insert("id".into(), json!(id));
        message.insert("method".into(), json!(method));
        insert_params(&mut message, params);
        self.send(message)?;
        match receiver.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(CallError::Rpc(error)),
            Err(_) => Err(CallError::Closed),
        }
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), CallError> {
        let mut message = self.envelope();
        message.insert("method".into(), json!(method));
        insert_params(&mut message, params);
        self.send(message)
    }

    pub fn respond(&self, id: &Id, result: Value) -> Result<(), CallError> {
        let mut message = self.envelope();
        message.insert("id".into(), id.to_value());
        message.insert("result".into(), result);
        self.send(message)
    }

    pub fn respond_error(&self, id: &Id, code: i64, message: &str) -> Result<(), CallError> {
        let mut reply = self.envelope();
        reply.insert("id".into(), id.to_value());
        reply.insert("error".into(), json!({"code": code, "message": message}));
        self.send(reply)
    }

    fn envelope(&self) -> Map<String, Value> {
        let mut message = Map::new();
        if self.options.tag_version {
            message.insert("jsonrpc".into(), json!("2.0"));
        }
        message
    }

    fn send(&self, message: Map<String, Value>) -> Result<(), CallError> {
        let line = Value::Object(message).to_string();
        self.out
            .send(Out::Line(line))
            .map_err(|_| CallError::Closed)
    }
}

fn insert_params(message: &mut Map<String, Value>, params: Value) {
    if !params.is_null() {
        message.insert("params".into(), params);
    }
}

pub struct Connection {
    client: Client,
    incoming: flume::Receiver<Incoming>,
    reader: JoinHandle<()>,
    writer: Option<JoinHandle<()>>,
}

pub fn connect<R, W>(reader: R, writer: W, options: Options) -> Connection
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let shared = Arc::new(Shared {
        pending: Mutex::new(Some(HashMap::new())),
        next_id: AtomicI64::new(1),
    });
    let (out_sender, out_receiver) = flume::unbounded();
    let (incoming_sender, incoming_receiver) = flume::unbounded();
    let handle = net_runtime::handle();
    let reader_task = handle.spawn(read_loop(reader, Arc::clone(&shared), incoming_sender));
    let writer_task = handle.spawn(write_loop(writer, out_receiver, Arc::clone(&shared)));
    Connection {
        client: Client {
            shared,
            out: out_sender,
            options,
        },
        incoming: incoming_receiver,
        reader: reader_task,
        writer: Some(writer_task),
    }
}

impl Connection {
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub async fn next(&self) -> Option<Incoming> {
        self.incoming.recv_async().await.ok()
    }

    pub async fn close(mut self) {
        let _ = self.client.out.send(Out::Close);
        if let Some(mut writer) = self.writer.take()
            && tokio::time::timeout(CLOSE_WAIT, &mut writer).await.is_err()
        {
            writer.abort();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
        self.client.shared.close();
    }
}

async fn write_loop<W>(mut writer: W, out: flume::Receiver<Out>, shared: Arc<Shared>)
where
    W: AsyncWrite + Unpin,
{
    while let Ok(item) = out.recv_async().await {
        let Out::Line(mut line) = item else {
            break;
        };
        line.push('\n');
        let written = async {
            writer.write_all(line.as_bytes()).await?;
            writer.flush().await
        }
        .await;
        if let Err(error) = written {
            log::debug!("writing to the peer: {error}");
            shared.close();
            break;
        }
    }
}

async fn read_loop<R>(reader: R, shared: Arc<Shared>, incoming: flume::Sender<Incoming>)
where
    R: AsyncRead + Unpin,
{
    let mut lines = Lines::new(reader);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => dispatch(&line, &shared, &incoming),
            Ok(None) => break,
            Err(error) => {
                log::debug!("reading from the peer: {error}");
                break;
            }
        }
    }
    shared.close();
}

fn dispatch(line: &str, shared: &Shared, incoming: &flume::Sender<Incoming>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let Ok(Value::Object(mut message)) = serde_json::from_str::<Value>(line) else {
        log::debug!(
            "skipping a line that is not a JSON object: {}",
            excerpt(line)
        );
        return;
    };
    let id = message.get("id").and_then(Id::from_value);
    let method = match message.remove("method") {
        Some(Value::String(method)) => Some(method),
        _ => None,
    };
    match (method, id) {
        (Some(method), Some(id)) => {
            let params = message.remove("params").unwrap_or(Value::Null);
            let _ = incoming.send(Incoming::Request { id, method, params });
        }
        (Some(method), None) => {
            let params = message.remove("params").unwrap_or(Value::Null);
            let _ = incoming.send(Incoming::Notification { method, params });
        }
        (None, Some(id)) => respond_to_call(shared, &id, message),
        (None, None) => {
            log::debug!(
                "skipping a message without a method or id: {}",
                excerpt(line)
            );
        }
    }
}

fn respond_to_call(shared: &Shared, id: &Id, mut message: Map<String, Value>) {
    let Id::Number(number) = id else {
        log::debug!("skipping a response with a non-numeric id: {id:?}");
        return;
    };
    let sender = shared
        .lock()
        .as_mut()
        .and_then(|pending| pending.remove(number));
    let Some(sender) = sender else {
        log::debug!("skipping a response to an unknown request {number}");
        return;
    };
    let outcome = match message.remove("error") {
        Some(Value::Object(error)) => Err(RpcError {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            data: error.get("data").cloned(),
        }),
        Some(other) if !other.is_null() => Err(RpcError {
            code: 0,
            message: other.to_string(),
            data: None,
        }),
        _ => Ok(message.remove("result").unwrap_or(Value::Null)),
    };
    let _ = sender.send(outcome);
}

fn excerpt(line: &str) -> String {
    line.chars().take(LOG_EXCERPT).collect()
}
