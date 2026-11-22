use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, DuplexStream, duplex};

use crate::jsonrpc::{CallError, Connection, Id, Incoming, Options, RpcError, connect};
use crate::lines::Lines;

struct Peer {
    lines: Lines<DuplexStream>,
    writer: DuplexStream,
}

impl Peer {
    async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .expect("the client wrote in time")
            .unwrap()
            .expect("the client is still connected");
        serde_json::from_str(&line).expect("the client wrote JSON")
    }

    async fn write(&mut self, value: Value) {
        self.write_raw(&format!("{value}\n")).await;
    }

    async fn write_raw(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).await.unwrap();
    }
}

fn pair(options: Options) -> (Connection, Peer) {
    let (client_reader, peer_writer) = duplex(8192);
    let (peer_reader, client_writer) = duplex(8192);
    let connection = connect(client_reader, client_writer, options);
    let peer = Peer {
        lines: Lines::new(peer_reader),
        writer: peer_writer,
    };
    (connection, peer)
}

async fn next(connection: &Connection) -> Incoming {
    tokio::time::timeout(Duration::from_secs(5), connection.next())
        .await
        .expect("a message arrived in time")
        .expect("the connection is open")
}

#[tokio::test]
async fn a_request_gets_its_response() {
    let (connection, mut peer) = pair(Options::default());
    let client = connection.client();
    let call = tokio::spawn(async move { client.request("ping", json!({"a": 1})).await });

    let request = peer.read().await;
    assert_eq!(request["jsonrpc"], "2.0");
    assert_eq!(request["method"], "ping");
    assert_eq!(request["params"], json!({"a": 1}));
    peer.write(json!({"jsonrpc": "2.0", "id": request["id"], "result": {"ok": true}}))
        .await;

    assert_eq!(call.await.unwrap(), Ok(json!({"ok": true})));
}

#[tokio::test]
async fn concurrent_requests_are_matched_by_id() {
    let (connection, mut peer) = pair(Options::default());
    let first = tokio::spawn({
        let client = connection.client();
        async move { client.request("first", Value::Null).await }
    });
    let second = tokio::spawn({
        let client = connection.client();
        async move { client.request("second", Value::Null).await }
    });

    let one = peer.read().await;
    let two = peer.read().await;
    for request in [two, one] {
        let method = request["method"].clone();
        peer.write(json!({"id": request["id"], "result": method}))
            .await;
    }

    assert_eq!(first.await.unwrap(), Ok(json!("first")));
    assert_eq!(second.await.unwrap(), Ok(json!("second")));
}

#[tokio::test]
async fn an_error_response_becomes_an_rpc_error() {
    let (connection, mut peer) = pair(Options::default());
    let client = connection.client();
    let call = tokio::spawn(async move { client.request("boom", Value::Null).await });
    let request = peer.read().await;
    assert!(request.get("params").is_none());
    peer.write(json!({
        "id": request["id"],
        "error": {"code": -32002, "message": "session not found", "data": {"id": "x"}}
    }))
    .await;
    assert_eq!(
        call.await.unwrap(),
        Err(CallError::Rpc(RpcError {
            code: -32002,
            message: "session not found".into(),
            data: Some(json!({"id": "x"})),
        }))
    );
}

#[tokio::test]
async fn notifications_and_server_requests_arrive_in_order() {
    let (connection, mut peer) = pair(Options::default());
    peer.write(json!({"method": "session/update", "params": {"n": 1}}))
        .await;
    peer.write(json!({"id": 7, "method": "fs/read", "params": {"path": "x"}}))
        .await;
    peer.write(json!({"id": "abc", "method": "ask"})).await;

    assert_eq!(
        next(&connection).await,
        Incoming::Notification {
            method: "session/update".into(),
            params: json!({"n": 1}),
        }
    );
    assert_eq!(
        next(&connection).await,
        Incoming::Request {
            id: Id::Number(7),
            method: "fs/read".into(),
            params: json!({"path": "x"}),
        }
    );
    assert_eq!(
        next(&connection).await,
        Incoming::Request {
            id: Id::String("abc".into()),
            method: "ask".into(),
            params: Value::Null,
        }
    );
}

#[tokio::test]
async fn replies_to_server_requests_carry_the_same_id() {
    let (connection, mut peer) = pair(Options::default());
    let client = connection.client();
    client
        .respond(&Id::Number(7), json!({"decision": "decline"}))
        .unwrap();
    client
        .respond_error(&Id::String("abc".into()), -32601, "no such method")
        .unwrap();
    client.notify("initialized", Value::Null).unwrap();

    assert_eq!(
        peer.read().await,
        json!({"jsonrpc": "2.0", "id": 7, "result": {"decision": "decline"}})
    );
    assert_eq!(
        peer.read().await,
        json!({"jsonrpc": "2.0", "id": "abc", "error": {"code": -32601, "message": "no such method"}})
    );
    assert_eq!(
        peer.read().await,
        json!({"jsonrpc": "2.0", "method": "initialized"})
    );
}

#[tokio::test]
async fn the_version_tag_can_be_left_out() {
    let (connection, mut peer) = pair(Options { tag_version: false });
    connection.client().notify("ping", json!([1])).unwrap();
    assert_eq!(peer.read().await, json!({"method": "ping", "params": [1]}));
}

#[tokio::test]
async fn unknown_ids_garbage_and_odd_messages_are_skipped() {
    let (connection, mut peer) = pair(Options::default());
    let client = connection.client();
    let call = tokio::spawn(async move { client.request("ping", Value::Null).await });
    let request = peer.read().await;

    peer.write(json!({"id": 4242, "result": "nobody asked"}))
        .await;
    peer.write_raw("this is not json\n").await;
    peer.write_raw("[1, 2, 3]\n").await;
    peer.write_raw("\n").await;
    peer.write(json!({"nothing": "useful"})).await;
    peer.write(json!({"id": "text-id", "result": 1})).await;
    peer.write(json!({"id": request["id"], "result": "late but right"}))
        .await;
    peer.write(json!({"method": "after"})).await;

    assert_eq!(call.await.unwrap(), Ok(json!("late but right")));
    assert_eq!(
        next(&connection).await,
        Incoming::Notification {
            method: "after".into(),
            params: Value::Null,
        }
    );
}

#[tokio::test]
async fn closing_the_stream_fails_pending_requests_and_ends_the_incoming_stream() {
    let (connection, peer) = pair(Options::default());
    let client = connection.client();
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.request("ping", Value::Null).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(peer.writer);

    assert_eq!(call.await.unwrap(), Err(CallError::Closed));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), connection.next())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        client.request("late", Value::Null).await,
        Err(CallError::Closed)
    );
}

#[tokio::test]
async fn close_flushes_what_was_queued_and_ends_the_peer_stream() {
    let (connection, mut peer) = pair(Options::default());
    connection.client().notify("last", Value::Null).unwrap();
    connection.close().await;
    assert_eq!(peer.read().await["method"], "last");
    assert!(peer.lines.next_line().await.unwrap().is_none());
}
