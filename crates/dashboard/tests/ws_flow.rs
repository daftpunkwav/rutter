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
    serve_with_keepalive(Duration::from_secs(20)).await
}

/// The same server with a liveness cadence the tests can wait out.
async fn serve_with_keepalive(keepalive: Duration) -> Serving {
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
    )
    .with_keepalive(keepalive);
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

    // Screencast on for an unknown session: still acked, no stream, and
    // the reason names the miss instead of leaving a bare ack.
    stream
        .send(Message::text(
            serde_json::json!({"type": "screencast", "on": true, "session": "ghost"}).to_string(),
        ))
        .await
        .expect("send screencast on");
    let ack = next_of_type(&mut stream, "screencast-ack").await;
    assert_eq!(
        ack["started"], false,
        "an unknown session streams nothing: {ack}"
    );
    assert_eq!(ack["reason"], "no such session", "the miss is named: {ack}");

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
async fn an_unknown_message_type_is_dropped_without_ending_the_stream() {
    // The dispatch loop's catch-all drops a message that names no
    // known type: no reply, but also no close. If that branch took
    // the connection down, a client on a newer protocol -- or one that
    // sent a frame the server does not model -- would lose its replay
    // and its live events rather than being told nothing.
    let serving = serve().await;
    let id = SessionId::new("s1");
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

    // Drain the replay so the assertions below are about the live
    // branch alone.
    loop {
        let frame = next_json(&mut stream).await;
        if frame["event"]["type"] == "session_started" {
            break;
        }
    }

    // A body that names no known type, and one that does not parse at
    // all, take the same branch. Neither may be answered.
    stream
        .send(Message::text(
            serde_json::json!({"type": "nonsense", "request_id": "apr-1"}).to_string(),
        ))
        .await
        .expect("send unknown type");
    stream
        .send(Message::text("{not json".to_string()))
        .await
        .expect("send unparsable body");

    // The decision that follows is the barrier: one socket carries one
    // ordered stream and this loop handles its frames in order, so an
    // ack can only exist if the two frames before it were received
    // and neither ended the connection. Publishing first would race
    // the `select!` and prove nothing.
    let (approval, receiver) = serving.manager.broker().open();
    stream
        .send(Message::text(
            serde_json::json!({
                "type": "decision",
                "request_id": approval.as_str(),
                "grant": false,
            })
            .to_string(),
        ))
        .await
        .expect("send decision after the noise");
    let ack = tokio::time::timeout(
        Duration::from_secs(5),
        next_of_type(&mut stream, "decision-ack"),
    )
    .await
    .expect("the loop still answers after an unknown type");
    assert_eq!(ack["accepted"], true, "the decision is still delivered");
    assert_eq!(receiver.await.expect("delivered"), Decision::Deny);

    // And the live branch is untouched too.
    backbone.publish(id, Event::SessionClosed);
    let live = tokio::time::timeout(Duration::from_secs(5), next_json(&mut stream))
        .await
        .expect("events still flow after an unknown type");
    assert_eq!(live["event"]["type"], "session_closed");
}

#[tokio::test]
async fn screencast_of_a_session_without_a_page_names_the_reason() {
    // A session with no open page answers
    // `no open page to observe` rather than opening one. The ack used
    // to swallow the failure, leaving the operator a bare ack, no
    // frames, and no explanation.
    let serving = serve().await;
    serving
        .manager
        .session(SessionId::new("s-live"))
        .await
        .expect("the stub engine opens a session");
    let mut stream = connect(&serving).await;
    stream
        .send(Message::text(
            serde_json::json!({"type": "screencast", "on": true, "session": "s-live"}).to_string(),
        ))
        .await
        .expect("send screencast on");
    let ack = next_of_type(&mut stream, "screencast-ack").await;
    assert_eq!(ack["started"], false, "no page, no stream: {ack}");
    assert_eq!(
        ack["reason"], "no open page to observe",
        "the refusal names its cause: {ack}"
    );
}

#[tokio::test]
async fn screencast_of_an_open_page_acks_started() {
    // The success half of the ack contract: a session with an open
    // page answers `started: true` with no
    // reason, and frames follow as binary messages.
    let serving = serve().await;
    let session = serving
        .manager
        .session(SessionId::new("s-live"))
        .await
        .expect("the stub engine opens a session");
    session.open_page(None).await.expect("the stub page opens");
    let mut stream = connect(&serving).await;
    stream
        .send(Message::text(
            serde_json::json!({"type": "screencast", "on": true, "session": "s-live"}).to_string(),
        ))
        .await
        .expect("send screencast on");
    let ack = next_of_type(&mut stream, "screencast-ack").await;
    assert_eq!(
        ack["started"], true,
        "an open page starts the stream: {ack}"
    );
    assert!(
        ack.get("reason").is_none(),
        "a started stream carries no refusal: {ack}"
    );
}

#[tokio::test]
async fn a_decision_flood_is_acked_without_wedging_the_loop() {
    // The decision reply used to be handed to a bounded channel whose
    // only reader was this same select loop, so a client that out-ran
    // the drain parked the loop inside its own send: no further acks,
    // no live events, no close, and no way back — the connection was
    // dead for good. Replies now go straight to the socket the loop
    // already owns exclusively, so backpressure stops at the client.
    const DECISIONS: usize = 8192;

    let serving = serve().await;
    serving
        .manager
        .session(SessionId::new("s1"))
        .await
        .expect("session starts");
    let mut stream = connect(&serving).await;
    let decision = serde_json::json!({
        "type": "decision",
        "request_id": "apr-flood",
        "grant": true,
    })
    .to_string();

    let answered = tokio::time::timeout(Duration::from_secs(15), async {
        for _ in 0..DECISIONS {
            stream.send(Message::text(decision.clone())).await.ok()?;
        }
        let mut acked = 0usize;
        while acked < DECISIONS {
            let frame = stream.next().await?.ok()?;
            if frame.into_text().ok()?.contains("\"decision-ack\"") {
                acked += 1;
            }
        }
        Some(())
    })
    .await;

    assert!(
        answered.is_ok(),
        "every one of the {DECISIONS} decisions must be acked; the loop wedged otherwise"
    );
}

#[tokio::test]
async fn a_peer_that_stops_answering_is_disconnected() {
    // A client that vanishes without a close — a dropped network, a
    // killed browser — leaves the loop parked in `recv` forever, and
    // with it the socket, a full broadcast receiver, and whatever
    // screencast capture it was holding. One unanswered ping ends it.
    const KEEPALIVE: Duration = Duration::from_millis(50);

    let serving = serve_with_keepalive(KEEPALIVE).await;
    serving
        .manager
        .session(SessionId::new("s1"))
        .await
        .expect("session starts");
    let mut stream = connect(&serving).await;
    // Read the replay, then go silent. A peer that never reads cannot
    // answer a ping — reading one would flush the pong the stack queued
    // for it — so this is the shape of a client that vanished: the
    // socket stays open on both ends and nothing more crosses it.
    loop {
        let frame = next_json(&mut stream).await;
        if frame["event"]["type"] == "session_started" {
            break;
        }
    }
    tokio::time::sleep(KEEPALIVE * 8).await;

    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match stream.next().await {
                None | Some(Err(_)) => return true,
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "an unanswered ping must end the connection, or the loop holds it forever"
    );
}

#[tokio::test]
async fn a_peer_that_answers_keeps_its_connection() {
    // The other half of the liveness contract: a client that answers
    // its pings is never dropped, however long it stays connected. The
    // pong comes from the peer's protocol stack, so this is exactly
    // what a slow-but-alive client does.
    const KEEPALIVE: Duration = Duration::from_millis(50);
    const PING_ROUNDS: usize = 6;

    let serving = serve_with_keepalive(KEEPALIVE).await;
    let id = SessionId::new("s1");
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

    let mut answered = 0;
    tokio::time::timeout(Duration::from_secs(5), async {
        while answered < PING_ROUNDS {
            match stream.next().await {
                Some(Ok(Message::Ping(payload))) => {
                    stream.send(Message::Pong(payload)).await.expect("pong");
                    answered += 1;
                }
                Some(Ok(_)) => continue,
                // The server closed: a responsive peer must not lose it.
                other => panic!("the connection ended at ping {answered}: {other:?}"),
            }
        }
    })
    .await
    .expect("every ping is answered in time");

    // Still delivering after the ping rounds: the pings cost a healthy
    // connection nothing.
    backbone.publish(id, Event::SessionClosed);
    let live = tokio::time::timeout(Duration::from_secs(5), next_json(&mut stream))
        .await
        .expect("events still flow");
    assert_eq!(live["event"]["type"], "session_closed");
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
