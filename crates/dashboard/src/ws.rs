//! The dashboard WebSocket: replay first, then live events; approval
//! decisions arrive as client messages, screencast frames leave as
//! binary frames on demand (docs/events.md, docs/dashboard.md).

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::extract::ws::{Message, WebSocket};
use rutter_core::ids::SessionId;
use rutter_events::Envelope;
use rutter_policy::{ApprovalId, Decision};
use rutter_session::ScreencastStream;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::Dashboard;

/// Replays history per session, then forwards live events; client
/// messages carry approval decisions.
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

    // Replay: every session's history, ordered by sequence number.
    let mut replay: Vec<Envelope> = Vec::new();
    for session in state.manager.session_ids().await {
        replay.extend(backbone.replay(&session));
    }
    replay.sort_by_key(|envelope| envelope.seq);
    let watermark = replay.last().map(|envelope| envelope.seq);
    for envelope in replay {
        if send_envelope(&mut socket, &envelope).await.is_err() {
            return;
        }
    }

    // Live: forward the broadcast stream; client decisions come back
    // on the same socket, so all writes happen in this loop. Screencast
    // frames are on-demand (docs/dashboard.md): they flow only while a
    // viewer asked for them, as binary WebSocket frames.
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Message>(64);
    let mut current: Option<ScreencastStream> = None;
    // Highest sequence number sent to this client; live envelopes at or
    // below it are duplicates and are skipped.
    let mut sent_up_to = watermark;

    loop {
        tokio::select! {
            envelope = live.recv() => {
                match envelope {
                    Ok(envelope) => {
                        // Already covered by the replay snapshot or a gap refill.
                        if sent_up_to.is_some_and(|seen| envelope.seq <= seen) {
                            continue;
                        }
                        let Ok(json) = serde_json::to_string(&envelope) else {
                            continue;
                        };
                        if socket.send(Message::text(json)).await.is_err() {
                            break;
                        }
                        sent_up_to = Some(envelope.seq);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // The bus dropped envelopes while this client was
                        // slow, but the rings still hold the semantic
                        // events (docs/events.md): resync from the last
                        // sequence sent instead of losing the gap until a
                        // reconnect.
                        let mut gap: Vec<Envelope> = Vec::new();
                        for session in state.manager.session_ids().await {
                            gap.extend(backbone.replay(&session));
                        }
                        gap.retain(|envelope| {
                            sent_up_to.is_none_or(|seen| envelope.seq > seen)
                        });
                        gap.sort_by_key(|envelope| envelope.seq);
                        for envelope in gap {
                            if send_envelope(&mut socket, &envelope).await.is_err() {
                                return;
                            }
                            sent_up_to = Some(envelope.seq);
                        }
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            reply = outgoing_rx.recv() => {
                match reply {
                    Some(message) => {
                        if socket.send(message).await.is_err() {
                            break;
                        }
                    }
                    None => break,
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
                        if socket
                            .send(Message::binary(jpeg_frame.jpeg))
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
                        // Screencast control lives here because the
                        // stream itself must live in this loop.
                        let value: Option<Value> = serde_json::from_str(&text).ok();
                        match value.as_ref().and_then(|v| v.get("type")).and_then(Value::as_str) {
                            Some("screencast") => {
                                // Dropping the previous stream stops
                                // its capture task.
                                current = None;
                                let on = value
                                    .as_ref()
                                    .and_then(|v| v.get("on"))
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                if on {
                                    let session_id = value
                                        .as_ref()
                                        .and_then(|v| v.get("session"))
                                        .and_then(Value::as_str)
                                        .map(|s| SessionId::new(s.to_owned()));
                                    if let Some(session_id) = session_id
                                        && let Some(session) =
                                            state.manager.get_session(&session_id).await
                                        {
                                            current =
                                                session.screencast().await.ok();
                                        }
                                }
                                let _ = socket
                                    .send(Message::text(
                                        "{\"type\":\"screencast-ack\"}",
                                    ))
                                    .await;
                            }
                            Some("subscribe") => {
                                // docs/dashboard.md lists subscribe as a
                                // client message; replay and the live
                                // stream start automatically on connect,
                                // so there is nothing further to do.
                            }
                            _ => {
                                if let Some(reply) = handle_client_message(&state, &text) {
                                    let _ =
                                        outgoing_tx.send(Message::text(reply)).await;
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }
    // Dropping `current` stops the capture task.
}

/// Applies one client message; approvals answer the broker.
fn handle_client_message(state: &Dashboard, text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    match value.get("type")?.as_str()? {
        "decision" => {
            let request_id = value.get("request_id")?.as_str()?;
            let granted = value.get("grant")?.as_bool()?;
            let decision = if granted {
                Decision::Grant
            } else {
                Decision::Deny
            };
            let accepted = state
                .broker
                .decide(&ApprovalId::new(request_id.to_owned()), decision);
            // Built through serde_json so a hostile request_id cannot
            // produce a malformed reply.
            serde_json::to_string(&serde_json::json!({
                "type": "decision-ack",
                "request_id": request_id,
                "accepted": accepted,
            }))
            .ok()
        }
        _ => None,
    }
}

async fn send_envelope(socket: &mut WebSocket, envelope: &Envelope) -> Result<(), axum::Error> {
    let json = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_owned());
    socket.send(Message::text(json)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rutter_session::manager::SessionManager;
    use std::sync::Arc;

    #[test]
    fn decision_ack_is_well_formed_for_hostile_request_ids() {
        // The ack is serialized JSON, so a request_id carrying quotes or
        // escapes cannot forge extra fields in the reply.
        let state = Dashboard {
            manager: Arc::new(SessionManager::new(
                Arc::new(UnsupportedLauncher),
                rutter_engine::config::LaunchMode::Headless,
                rutter_session::config::SessionConfig::default(),
                Arc::new(rutter_policy::RuleSet::default_set()),
                Arc::new(rutter_policy::ApprovalBroker::new()),
                None,
            )),
            broker: Arc::new(rutter_policy::ApprovalBroker::new()),
            token: "t".to_owned(),
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
        let reply = handle_client_message(&state, &message).expect("a decision-ack reply");
        let ack: Value = serde_json::from_str(&reply).expect("the reply must be valid JSON");
        assert_eq!(ack["type"], "decision-ack");
        assert_eq!(ack["request_id"], hostile, "the id round-trips verbatim");
        assert_eq!(
            ack["accepted"], false,
            "an unknown approval is rejected, not granted"
        );
        assert!(ack.get("injected").is_none(), "no forged field survives");
    }

    /// Launcher stub satisfying the manager constructor; the dashboard
    /// tests never launch an engine through it. Duplicated from the
    /// lib tests because test fixtures stay module-local.
    struct UnsupportedLauncher;

    #[async_trait::async_trait]
    impl rutter_engine::supervisor::EngineLauncher for UnsupportedLauncher {
        fn describe(&self) -> String {
            "unsupported".to_owned()
        }

        async fn launch(
            &self,
            _mode: rutter_engine::config::LaunchMode,
        ) -> Result<Arc<dyn rutter_engine::engine::Engine>, rutter_engine::error::EngineError>
        {
            Err(rutter_engine::error::EngineError::Unsupported {
                operation: "launch".to_owned(),
                reason: "dashboard tests never launch engines".to_owned(),
            })
        }
    }
}
