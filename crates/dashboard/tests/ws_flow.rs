//! Functional tests for the dashboard's WebSocket loop: the engine-off
//! note, replay-then-live event order, client decisions, and screencast
//! control messages. The socket under test is the real `ws_loop`.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rutter_core::ids::SessionId;
use rutter_dashboard::DashboardServer;
use rutter_engine::config::LaunchMode;
use rutter_events::Event;
use rutter_policy::{ApprovalBroker, Decision, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;
use tokio_tungstenite::tungstenite::Message;

use common::FlowLauncher;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A running dashboard plus the manager behind it (tests start sessions
/// and park approvals on the same objects the server serves).
struct Serving {
    ws_url: String,
    token: String,
    manager: Arc<SessionManager>,
    _access_dir: tempfile::TempDir,
}

async fn serve() -> Serving {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let access_dir = tempfile::tempdir().expect("temp dir for the access files");
    let manager = Arc::new(SessionManager::new(
        FlowLauncher::new(),
        LaunchMode::Headless,
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    ));
    let server = DashboardServer::new(
        Arc::clone(&manager),
        port,
        Some(access_dir.path().to_path_buf()),
    );
    let token = server.token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });

    Serving {
        ws_url: format!("ws://127.0.0.1:{port}/ws"),
        token,
        manager,
        _access_dir: access_dir,
    }
}

async fn connect(serving: &Serving) -> WsStream {
    let url = format!("{}?token={}", serving.ws_url, serving.token);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((stream, _)) => return stream,
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "ws did not accept upgrades within 10 s: {error}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

/// Reads one text frame and parses it as JSON.
async fn next_json(stream: &mut WsStream) -> serde_json::Value {
    let message = stream
        .next()
        .await
        .expect("a frame")
        .expect("the stream stays open");
    let text = message
        .into_text()
        .expect("the dashboard sends text frames here");
    serde_json::from_str(&text).expect("dashboard frames are JSON")
}

/// Reads frames until one is a control message of `expected_type`.
/// Envelopes (replay or live) have no top-level `type`, so they are
/// skipped; a caller that expects no envelopes asserts on `next_json`
/// directly instead.
async fn next_of_type(stream: &mut WsStream, expected_type: &str) -> serde_json::Value {
    for _ in 0..16 {
        let frame = next_json(stream).await;
        if frame["type"] == expected_type {
            return frame;
        }
    }
    panic!("no {expected_type} frame arrived within 16 frames");
}

#[tokio::test]
async fn ws_without_an_engine_replies_a_note_and_closes() {
    let serving = serve().await;
    let mut stream = connect(&serving).await;

    let note = next_json(&mut stream).await;
    assert_eq!(note["type"], "note");
    assert!(
        note["text"].as_str().expect("note text").contains("engine"),
        "the note says the engine has not started: {note}"
    );
    // The loop returns after the note; the server drops the socket.
    // A dropped upgrade closes the connection without a handshake
    // close frame (Reset on Windows, EOF elsewhere) — both mean "gone".
    match stream.next().await {
        None => {}
        Some(Ok(Message::Close(_))) => {}
        Some(Err(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ))) => {}
        other => panic!("the socket closes after the note, got {other:?}"),
    }
}

#[tokio::test]
async fn ws_replays_history_then_forwards_live_events() {
    let serving = serve().await;
    let id = SessionId::new("s1");
    // Starts the engine and publishes EngineStarted + SessionStarted
    // before the socket connects, so both must arrive through replay,
    // in sequence order.
    serving
        .manager
        .session(id.clone())
        .await
        .expect("session starts");
    let backbone = serving
        .manager
        .backbone()
        .await
        .expect("the engine is running");

    let mut stream = connect(&serving).await;
    let first = next_json(&mut stream).await;
    assert_eq!(
        first["event"]["type"], "engine_started",
        "the launch the session witnessed replays first: {first}"
    );
    let replayed = next_json(&mut stream).await;
    assert_eq!(
        replayed["event"]["type"], "session_started",
        "the recorded session start replays: {replayed}"
    );
    assert!(
        replayed["seq"].as_u64().expect("seq") > first["seq"].as_u64().expect("seq"),
        "replay is ordered by sequence number"
    );

    // Live: an event published after the connect arrives too, with a
    // higher sequence than anything replayed.
    backbone.publish(id.clone(), Event::SessionClosed);
    let live = next_json(&mut stream).await;
    assert!(
        live["seq"].as_u64().expect("seq") > replayed["seq"].as_u64().expect("seq"),
        "live events continue where replay stopped: {live}"
    );
    assert_eq!(live["event"]["type"], "session_closed");
}

#[tokio::test]
async fn ws_decisions_answer_parked_approvals_and_controls_are_acked() {
    let serving = serve().await;
    let mut stream = connect(&serving).await;
    // No engine yet: the first frame is the note and the loop has ended.
    let _note = next_json(&mut stream).await;
    drop(stream);

    let id = SessionId::new("s1");
    serving.manager.session(id).await.expect("session starts");
    let mut stream = connect(&serving).await;

    // A decision for a parked approval is accepted and delivered.
    let broker = serving.manager.broker();
    let (approval, receiver) = broker.open();
    let decision = serde_json::json!({
        "type": "decision",
        "request_id": approval.as_str(),
        "grant": false,
    })
    .to_string();
    stream
        .send(Message::text(decision))
        .await
        .expect("send decision");
    let ack = next_of_type(&mut stream, "decision-ack").await;
    assert_eq!(ack["request_id"], approval.as_str());
    assert_eq!(
        ack["accepted"], true,
        "the parked approval accepts the deny"
    );
    assert_eq!(receiver.await.expect("delivered"), Decision::Deny);

    // A decision for an unknown approval reports not accepted.
    let unknown = serde_json::json!({
        "type": "decision",
        "request_id": "apr-gone",
        "grant": true,
    })
    .to_string();
    stream
        .send(Message::text(unknown))
        .await
        .expect("send unknown decision");
    let ack = next_of_type(&mut stream, "decision-ack").await;
    assert_eq!(ack["accepted"], false, "unknown approvals stay unknown");

    // Screencast control (off: no session needed) is acknowledged.
    stream
        .send(Message::text(
            serde_json::json!({"type": "screencast", "on": false}).to_string(),
        ))
        .await
        .expect("send screencast off");
    next_of_type(&mut stream, "screencast-ack").await;

    // Screencast on for an unknown session: still acked, no stream.
    stream
        .send(Message::text(
            serde_json::json!({"type": "screencast", "on": true, "session": "ghost"}).to_string(),
        ))
        .await
        .expect("send screencast on");
    next_of_type(&mut stream, "screencast-ack").await;

    // The documented subscribe message needs no reply (replay and the
    // live stream start automatically on connect), and an unknown
    // message type is ignored: both send nothing, so a well-formed
    // decision afterwards still gets its ack on a clean stream.
    stream
        .send(Message::text(
            serde_json::json!({"type": "subscribe"}).to_string(),
        ))
        .await
        .expect("send subscribe");
    stream
        .send(Message::text(
            serde_json::json!({"type": "mystery"}).to_string(),
        ))
        .await
        .expect("send unknown type");

    let (approval, receiver) = broker.open();
    stream
        .send(Message::text(
            serde_json::json!({
                "type": "decision",
                "request_id": approval.as_str(),
                "grant": true,
            })
            .to_string(),
        ))
        .await
        .expect("send decision after noise");
    let ack = next_of_type(&mut stream, "decision-ack").await;
    assert_eq!(ack["accepted"], true);
    assert_eq!(receiver.await.expect("delivered"), Decision::Grant);
}

#[tokio::test]
async fn ws_rejects_an_upgrade_without_the_token() {
    let serving = serve().await;
    let url = format!("{}?token=wrong", serving.ws_url);
    // `serve` releases the port before the spawned server rebinds it,
    // so the first attempts can hit a connection refusal that carries
    // no verdict — retry like `connect` does and judge only the HTTP
    // answer the running server gives.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok(_) => panic!("the upgrade must be refused, not accepted"),
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), 403, "strangers get forbidden");
                break;
            }
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "dashboard never answered the upgrade: {error}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}
