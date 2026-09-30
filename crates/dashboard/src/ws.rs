//! The dashboard WebSocket: replay first, then live events. Inbound
//! text frames are one of three kinds, dispatched on their `type`
//! field — an approval decision, screencast control, or the accepted
//! no-op `subscribe`; screencast frames leave as binary frames on
//! demand.
//!
//! A connection lives only as long as its peer answers: a silent peer
//! is asked to prove it is there on the [`Dashboard::keepalive`]
//! cadence, and every write is bounded by [`WRITE_TIMEOUT`].

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use rutter_core::ids::SessionId;
use rutter_events::{Backbone, Envelope};
use rutter_session::ScreencastStream;
use serde_json::Value;
use std::time::Duration;

use crate::Dashboard;

/// How long one write to a client may block before the connection is
/// declared dead. Sends are the only thing backpressure stops at, and a
/// peer that stopped reading — a suspended tab, a laptop that went to
/// sleep mid-stream — would otherwise park this loop inside a send for
/// good, holding the socket, a full broadcast receiver, and a running
/// screencast capture that keeps acking frames nobody will ever see.
///
/// The budget cannot cut a healthy client short: the listener is bound
/// to loopback, so a client is always on this machine, and the heaviest
/// thing this loop writes is a screencast frame at the channel's rate
/// (2 per second, ~150 KB). A local reader drains that in
/// milliseconds, which leaves this budget three orders of magnitude of
/// headroom rather than a guess at link speed.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Replays history per session, then forwards live events until the
/// peer stops answering. Inbound text frames are dispatched on their
/// `type` field: `screencast` and `subscribe` are handled in this
/// loop, everything else goes to [`handle_decision`].
pub(crate) async fn ws_loop(state: Dashboard, mut socket: WebSocket) {
    let Some(backbone) = state.manager.backbone().await else {
        let _ = socket
            .send(Message::text(
                r#"{"type":"note","text":"engine not started yet"}"#,
            ))
            .await;
        return;
    };

    // Subscribe before taking the replay snapshot: events published in
    // between would otherwise be in neither the snapshot nor the live
    // stream. Live envelopes at or below the replay watermark are
    // duplicates and are skipped.
    let mut live = backbone.subscribe();

    // Highest sequence number sent to this client; live envelopes at or
    // below it are duplicates and are skipped.
    let mut sent_up_to = match drain_history(&state, &backbone, None, &mut socket).await {
        Ok(watermark) => watermark,
        Err(()) => return,
    };

    // Live: forward the broadcast stream; client decisions come back on
    // the same socket, so every write happens in this loop and replies
    // go straight to the socket it already owns. A reply must never be
    // handed to a queue this loop would also have to drain: a client
    // that out-ran the drain would park the loop inside its own send,
    // with no acks, no events, and no way back. Screencast frames are
    // on-demand: they flow only while a viewer asked for them, as
    // binary WebSocket frames.
    let mut current: Option<ScreencastStream> = None;
    // A connection that never speaks again is not one this loop can
    // tell from a healthy one: a peer that vanishes without a close
    // (a dropped network, a killed browser) leaves the socket parked in
    // `recv` forever. A ping asks the peer to prove it is there, and
    // one unanswered ping — a full interval, not a round-trip budget —
    // ends the connection. It cannot misfire on a slow client: the pong
    // comes from the peer's protocol stack, not from the page, and the
    // deadline is measured from the previous ping.
    let mut keepalive = tokio::time::interval_at(
        tokio::time::Instant::now() + state.keepalive,
        state.keepalive,
    );
    let mut awaiting_pong = false;

    loop {
        tokio::select! {
            envelope = live.recv() => {
                match envelope {
                    Ok(envelope) => {
                        // Already covered by the replay snapshot or a gap refill.
                        if sent_up_to.is_some_and(|seen| envelope.seq <= seen) {
                            continue;
                        }
                        if send_within(&mut socket, Message::text(encode(&envelope)))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        sent_up_to = Some(envelope.seq);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // The bus dropped envelopes while this client was
                        // slow, but the rings still hold the semantic
                        // events: resync from the last
                        // sequence sent instead of losing the gap until a
                        // reconnect.
                        match drain_history(&state, &backbone, sent_up_to, &mut socket).await {
                            Ok(watermark) => sent_up_to = watermark,
                            Err(()) => break,
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            frame = async {
                match current.as_mut() {
                    Some(stream) => stream.next_frame().await,
                    None => std::future::pending().await,
                }
            } => {
                match frame {
                    Some(jpeg_frame) => {
                        if send_within(&mut socket, Message::binary(jpeg_frame.jpeg))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    None => current = None,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let value: Option<Value> = serde_json::from_str(&text).ok();
                        match value.as_ref().and_then(|v| v.get("type")).and_then(Value::as_str) {
                            Some("screencast") => {
                                // Screencast control lives here because
                                // the stream itself must live in this
                                // loop. The previous stream is dropped
                                // before a second one is registered:
                                // its task would otherwise tear the new
                                // capture down on its way out.
                                drop(current.take());
                                let (stream, ack) =
                                    screencast_control(&state, value.as_ref()).await;
                                current = stream;
                                if send_within(&mut socket, Message::text(ack))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Some("subscribe") => {
                                // `subscribe` is accepted as a client
                                // message; replay and the live
                                // stream start automatically on connect,
                                // so there is nothing further to do.
                            }
                            _ => {
                                // The decision path, and the only one
                                // that answers. A message that names
                                // no known type — or one whose body
                                // does not parse — is dropped without
                                // a reply rather than closing the
                                // connection.
                                if let Some(reply) = handle_decision(&state, &text)
                                    && send_within(&mut socket, Message::text(reply))
                                        .await
                                        .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    // The peer's answer to the liveness ping; the next
                    // tick sends another one instead of giving up on it.
                    Some(Ok(Message::Pong(_))) => awaiting_pong = false,
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            _ = keepalive.tick() => {
                if awaiting_pong {
                    break;
                }
                if send_within(&mut socket, Message::Ping(Bytes::new()))
                    .await
                    .is_err()
                {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
    // Dropping `current` stops the capture task.
}

/// Writes every session's history at or above `after` in sequence
/// order and answers the new watermark. `Err` means a write failed and
/// the connection is over.
///
/// The first call replays from the start (`after` is `None`); the
/// `Lagged` refill passes the highest sequence already sent, and the
/// filter runs inside the ring so entries at or below it are never
/// cloned. An empty batch keeps the watermark the caller already had.
async fn drain_history(
    state: &Dashboard,
    backbone: &Backbone,
    after: Option<u64>,
    socket: &mut WebSocket,
) -> Result<Option<u64>, ()> {
    let mut batch: Vec<Envelope> = Vec::new();
    for session in state.manager.session_ids().await {
        batch.extend(backbone.replay_after(&session, after));
    }
    batch.sort_by_key(|envelope| envelope.seq);
    for envelope in &batch {
        send_within(socket, Message::text(encode(envelope))).await?;
    }
    Ok(batch.last().map(|envelope| envelope.seq).or(after))
}

/// Answers one `screencast` control message: the stream the loop should
/// keep running and the ack to write. A refusal names its cause instead
/// of acking a stream that never comes, and a request that names no
/// session refuses the same way a body that never parsed does.
async fn screencast_control(
    state: &Dashboard,
    request: Option<&Value>,
) -> (Option<ScreencastStream>, String) {
    let mut ack = serde_json::json!({ "type": "screencast-ack", "started": false });
    if !request
        .and_then(|value| value.get("on"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return (None, ack.to_string());
    }
    let Some(session_id) = request
        .and_then(|value| value.get("session"))
        .and_then(Value::as_str)
        .map(|session| SessionId::new(session.to_owned()))
    else {
        ack["reason"] = serde_json::json!("the request names no session");
        return (None, ack.to_string());
    };
    let Some(session) = state.manager.get_session(&session_id).await else {
        ack["reason"] = serde_json::json!("no such session");
        return (None, ack.to_string());
    };
    match session.screencast().await {
        Ok(stream) => {
            ack["started"] = serde_json::json!(true);
            (Some(stream), ack.to_string())
        }
        Err(other) => {
            // The refusal names itself: `SessionError::NoOpenPage`'s
            // Display is the exact text docs/dashboard.md pins, so the
            // reason travels the error's own wording with no copy of it
            // here.
            ack["reason"] = serde_json::json!(other.to_string());
            (None, ack.to_string())
        }
    }
}

/// Applies one decision message; approvals answer the broker. The
/// decision body is the crate's shared contract (see
/// [`crate::parse_decision`]); only the reply shape is this loop's.
fn handle_decision(state: &Dashboard, text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    match value.get("type")?.as_str()? {
        "decision" => {
            let (request_id, decision) = crate::parse_decision(&value)?;
            let accepted = state.broker.decide(&request_id, decision);
            // Built through serde_json so a hostile request_id cannot
            // produce a malformed reply.
            serde_json::to_string(&serde_json::json!({
                "type": "decision-ack",
                "request_id": request_id.as_str(),
                "accepted": accepted,
            }))
            .ok()
        }
        _ => None,
    }
}

/// One bounded write to the client. `Err` means the connection is over:
/// either the peer is gone or it stopped reading for longer than
/// [`WRITE_TIMEOUT`], and in both cases this loop has nothing left to do
/// but stop.
async fn send_within(socket: &mut WebSocket, message: Message) -> Result<(), ()> {
    tokio::time::timeout(WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| ())
        .and_then(|sent| sent.map_err(|_| ()))
}

/// The wire form of one envelope. Serialization of a known-shaped value
/// does not fail, so a `{}` placeholder keeps an unencodable envelope
/// from taking the whole connection down with it.
fn encode(envelope: &Envelope) -> String {
    serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn decision_ack_is_well_formed_for_hostile_request_ids() {
        // The ack is serialized JSON, so a request_id carrying quotes or
        // escapes cannot forge extra fields in the reply.
        let state = Dashboard {
            manager: crate::test_support::test_manager(),
            broker: Arc::new(rutter_policy::ApprovalBroker::new()),
            token: "t".to_owned(),
            port: 7700,
            keepalive: std::time::Duration::from_secs(20),
        };
        // A hostile id: if the reply were built by string concatenation,
        // these quotes would terminate the id and forge extra fields.
        let hostile = "apr-1\"},\"accepted\":true,\"injected\":{";
        let message = serde_json::json!({
            "type": "decision",
            "request_id": hostile,
            "grant": true,
        })
        .to_string();
        let reply = handle_decision(&state, &message).expect("a decision-ack reply");
        let ack: Value = serde_json::from_str(&reply).expect("the reply must be valid JSON");
        assert_eq!(ack["type"], "decision-ack");
        assert_eq!(ack["request_id"], hostile, "the id round-trips verbatim");
        assert_eq!(
            ack["accepted"], false,
            "an unknown approval is rejected, not granted"
        );
        assert!(ack.get("injected").is_none(), "no forged field survives");
    }
}
