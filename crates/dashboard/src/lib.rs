//! The supervision dashboard: local web server over the event backbone
//! and the approval broker.
//!
//! Responsibilities:
//! - Serve the static frontend (embedded, no build step) on
//!   127.0.0.1 only, gated by a per-launch random token printed to the
//!   terminal; `Host` headers are validated against DNS rebinding
//!   (blueprint §7.7).
//! - Stream events over one WebSocket: replay first, then live; the
//!   client submits approval decisions through the same socket.
//!
//! Boundary: observation plus verdict submission — the dashboard never
//! executes actions (blueprint §5). The screencast live view (§7.7) is
//! not wired yet and lands with the remaining M2 work.

use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use axum::routing::{get, post};
use rutter_events::Envelope;
use rutter_policy::{ApprovalBroker, ApprovalId, Decision};
use rutter_session::manager::SessionManager;
use serde_json::Value;
use tokio::sync::mpsc;

/// Static frontend sources, embedded at compile time.
mod assets {
    /// The dashboard page shell.
    pub const INDEX_HTML: &str = include_str!("../../../frontend/src/index.html");
    /// The dashboard application script.
    pub const APP_JS: &str = include_str!("../../../frontend/src/app.js");
    /// The English string catalog (default locale, blueprint §7.7).
    pub const I18N_EN: &str = include_str!("../../../frontend/i18n/en.json");
}

/// Shared dashboard state.
#[derive(Clone)]
struct Dashboard {
    manager: Arc<SessionManager>,
    broker: Arc<ApprovalBroker>,
    token: String,
}

/// The dashboard server; bind and serve until the process exits.
pub struct DashboardServer {
    manager: Arc<SessionManager>,
    broker: Arc<ApprovalBroker>,
    port: u16,
}

impl DashboardServer {
    /// Binds the dashboard to a port; the access token is generated
    /// here and printed once by [`DashboardServer::token`].
    pub fn new(manager: Arc<SessionManager>, broker: Arc<ApprovalBroker>, port: u16) -> Self {
        Self {
            manager,
            broker,
            port,
        }
    }

    /// The per-launch access token (blueprint §7.7). The
    /// `RUTTER_DASHBOARD_TOKEN` override exists for automation; without
    /// it the token mixes time and process id, enough to resist
    /// accidental observers on a single-user machine.
    pub fn token(&self) -> String {
        if let Ok(token) = std::env::var("RUTTER_DASHBOARD_TOKEN") {
            if !token.is_empty() {
                return token;
            }
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Serves until the process exits.
    pub async fn run(self) -> Result<(), String> {
        let token = self.token();
        eprintln!(
            "rutter: dashboard on http://127.0.0.1:{}/?token={token}",
            self.port
        );
        let state = Dashboard {
            manager: self.manager,
            broker: self.broker,
            token: token.clone(),
        };

        let app = Router::new()
            .route("/", get(index))
            .route("/app.js", get(app_js))
            .route("/i18n/en.json", get(i18n))
            .route("/ws", get(ws_upgrade))
            .route("/api/decisions", post(decide))
            .fallback(not_found)
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", self.port))
            .await
            .map_err(|error| format!("cannot bind 127.0.0.1:{}: {error}", self.port))?;
        axum::serve(listener, app)
            .await
            .map_err(|error| format!("dashboard server failed: {error}"))
    }
}

/// Host header validation: only loopback names pass (blueprint §7.7,
/// DNS rebinding).
fn host_allowed(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| {
            host.starts_with("127.0.0.1")
                || host.starts_with("localhost")
                || host.starts_with("[::1]")
        })
}

fn token_ok(state: &Dashboard, provided: Option<&String>) -> bool {
    provided.is_some_and(|token| constant_time_eq(token, &state.token))
}

/// Equality without early exit; tokens are short so this is cheap.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn index(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Html<&'static str>, StatusCode> {
    if !host_allowed(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    // First visit carries the token in the query; the page then stores
    // it in localStorage and reconnects without it.
    if !token_ok(&state, query.get("token")) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Html(assets::INDEX_HTML))
}

async fn app_js(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<([(String, String); 1], &'static str), StatusCode> {
    if !host_allowed(&headers) || !token_ok(&state, query.get("token")) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok((
        [(
            "content-type".to_owned(),
            "application/javascript".to_owned(),
        )],
        assets::APP_JS,
    ))
}

async fn i18n(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<([(String, String); 1], &'static str), StatusCode> {
    if !host_allowed(&headers) || !token_ok(&state, query.get("token")) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok((
        [("content-type".to_owned(), "application/json".to_owned())],
        assets::I18N_EN,
    ))
}

/// WebSocket upgrade: validates host and token like every endpoint.
async fn ws_upgrade(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, StatusCode> {
    if !host_allowed(&headers) || !token_ok(&state, query.get("token")) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(upgrade.on_upgrade(move |socket| ws_loop(state, socket)))
}

/// Replays history per session, then forwards live events; client
/// messages carry approval decisions.
async fn ws_loop(state: Dashboard, mut socket: WebSocket) {
    let Some(backbone) = state.manager.backbone().await else {
        let _ = socket
            .send(Message::text(
                r#"{"type":"note","text":"engine not started yet"}"#,
            ))
            .await;
        return;
    };

    // Replay: every session's history, ordered by sequence number.
    let mut replay: Vec<Envelope> = Vec::new();
    for session in state.manager.session_ids().await {
        replay.extend(backbone.replay(&session));
    }
    replay.sort_by_key(|envelope| envelope.seq);
    for envelope in replay {
        if send_envelope(&mut socket, &envelope).await.is_err() {
            return;
        }
    }

    // Live: forward the broadcast stream; client decisions come back
    // on the same socket, so all writes happen in this loop.
    let mut live = backbone.subscribe();
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Message>(64);

    loop {
        tokio::select! {
            envelope = live.recv() => {
                match envelope {
                    Ok(envelope) => {
                        let Ok(json) = serde_json::to_string(&envelope) else {
                            continue;
                        };
                        if socket.send(Message::text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Semantic events survive via replay; a lagging
                        // dashboard re-syncs on reconnect (blueprint §7.5).
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
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(reply) = handle_client_message(&state, &text) {
                            let _ = outgoing_tx.send(Message::text(reply)).await;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }
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
            Some(format!(
                "{{\"type\":\"decision-ack\",\"request_id\":\"{request_id}\",\"accepted\":{accepted}}}"
            ))
        }
        _ => None,
    }
}

async fn send_envelope(socket: &mut WebSocket, envelope: &Envelope) -> Result<(), axum::Error> {
    let json = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_owned());
    socket.send(Message::text(json)).await
}

/// Decisions may also arrive as plain HTTP posts from scripts.
async fn decide(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    body: String,
) -> Result<StatusCode, StatusCode> {
    if !host_allowed(&headers) || !token_ok(&state, query.get("token")) {
        return Err(StatusCode::FORBIDDEN);
    }
    let value: Value = serde_json::from_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let granted = value
        .get("grant")
        .and_then(Value::as_bool)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let decision = if granted {
        Decision::Grant
    } else {
        Decision::Deny
    };
    if state
        .broker
        .decide(&ApprovalId::new(request_id.to_owned()), decision)
    {
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
