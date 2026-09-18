//! The supervision dashboard: local web server over the event backbone
//! and the approval broker.
//!
//! Responsibilities:
//! - Serve the static frontend (embedded, no build step) on
//!   127.0.0.1 only, gated by a per-launch random token printed to the
//!   terminal; the first visit exchanges the query token for an
//!   HttpOnly session cookie, and `Host` headers are validated against
//!   DNS rebinding (blueprint §7.7).
//! - Stream events over one WebSocket: replay first, then live; the
//!   client submits approval decisions through the same socket.
//!
//! Boundary: observation plus verdict submission — the dashboard never
//! executes actions (blueprint §5). The screencast live view (§7.7)
//! streams binary frames on demand through the same WebSocket.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use rutter_core::ids::SessionId;
use rutter_engine::page::ScreencastStream;
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
    /// here and printed once by [`DashboardServer::token`]. Approval
    /// decisions always go through the manager's broker — the one the
    /// sessions park on — so it is derived here, never passed in.
    pub fn new(manager: Arc<SessionManager>, port: u16) -> Self {
        Self {
            broker: manager.broker(),
            manager,
            port,
        }
    }

    /// The per-launch access token (blueprint §7.7). The
    /// `RUTTER_DASHBOARD_TOKEN` override exists for automation; without
    /// it the token hashes the launch time and process id with a
    /// randomly keyed hasher seeded from OS entropy, so its value is
    /// unpredictable from launch circumstances alone.
    pub fn token(&self) -> String {
        if let Ok(token) = std::env::var("RUTTER_DASHBOARD_TOKEN")
            && !token.is_empty()
        {
            return token;
        }
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut hasher = RandomState::new().build_hasher();
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
/// DNS rebinding). The host name is compared after stripping any port;
/// a prefix match would let `127.0.0.1.evil.com` through.
fn host_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    let name = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6 literal: "[::1]:port" -> "::1".
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost") || name == "::1"
}

fn token_ok(state: &Dashboard, headers: &HeaderMap, provided: Option<&String>) -> bool {
    let provided = provided
        .map(String::to_owned)
        .or_else(|| cookie_token(headers));
    provided.is_some_and(|token| constant_time_eq(&token, &state.token))
}

/// Name of the HttpOnly cookie carrying the dashboard token after the
/// first visit (blueprint §7.7: the query token is exchanged for a
/// cookie on first connect).
const TOKEN_COOKIE: &str = "rutter_token";

/// Extracts the token cookie from `Cookie` headers, if present.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix(&format!("{TOKEN_COOKIE}="))
                .map(str::to_owned)
        })
}

/// Whether the token can travel in a cookie value: generated tokens
/// are hex, so an automation override with characters outside the
/// cookie-safe set keeps using the query parameter only.
fn cookie_safe(token: &str) -> bool {
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
}

/// The `Set-Cookie` header exchanging the query token for a session
/// cookie; `HttpOnly` keeps page scripts from reading it back.
fn token_cookie_header(token: &str) -> Option<axum::http::HeaderValue> {
    if !cookie_safe(token) {
        return None;
    }
    axum::http::HeaderValue::from_str(&format!(
        "{TOKEN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict"
    ))
    .ok()
}

/// Equality without early exit; tokens are short so this is cheap.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Gate every endpoint shares: the `Host` header must name loopback and
/// the request must carry the token (query parameter or cookie). Any
/// new route must pass through this check.
fn access_allowed(state: &Dashboard, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    host_allowed(headers) && token_ok(state, headers, query.get("token"))
}

async fn index(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, StatusCode> {
    if !access_allowed(&state, &headers, &query) {
        return Err(StatusCode::FORBIDDEN);
    }
    // First visit carries the token in the query; the answer exchanges
    // it for a session cookie (blueprint §7.7), and later requests
    // authenticate through the cookie alone.
    let mut response = Html(assets::INDEX_HTML).into_response();
    if let Some(cookie) = token_cookie_header(&state.token) {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }
    Ok(response)
}

async fn app_js(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<([(String, String); 1], &'static str), StatusCode> {
    if !access_allowed(&state, &headers, &query) {
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
    Query(query): Query<HashMap<String, String>>,
) -> Result<([(String, String); 1], &'static str), StatusCode> {
    if !access_allowed(&state, &headers, &query) {
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
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, StatusCode> {
    if !access_allowed(&state, &headers, &query) {
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
    // frames are on-demand (blueprint §7.7): they flow only while a
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
                        // events (blueprint §7.5): resync from the last
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
                                // Blueprint §7.7 lists subscribe as a
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

/// Decisions may also arrive as plain HTTP posts from scripts.
async fn decide(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: String,
) -> Result<StatusCode, StatusCode> {
    if !access_allowed(&state, &headers, &query) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie_headers(parts: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for part in parts {
            headers.append(
                axum::http::header::COOKIE,
                axum::http::HeaderValue::from_str(part).expect("cookie header"),
            );
        }
        headers
    }

    #[test]
    fn cookie_token_is_extracted_from_mixed_headers() {
        let headers = cookie_headers(&["a=1; rutter_token=abc123", "b=2"]);
        assert_eq!(cookie_token(&headers).as_deref(), Some("abc123"));
        assert_eq!(cookie_token(&HeaderMap::new()), None);
    }

    #[test]
    fn cookie_safety_rejects_separator_characters() {
        assert!(cookie_safe("0123abcdEF-_.~"));
        assert!(!cookie_safe("a b"));
        assert!(!cookie_safe("a;b"));
    }

    #[test]
    fn generated_tokens_are_cookie_safe_and_hex() {
        let server = DashboardServer::new(
            Arc::new(SessionManager::new(
                Arc::new(UnsupportedLauncher),
                rutter_engine::config::LaunchMode::Headless,
                rutter_session::config::SessionConfig::default(),
                Arc::new(rutter_policy::RuleSet::default_set()),
                Arc::new(rutter_policy::ApprovalBroker::new()),
                None,
            )),
            0,
        );
        let token = server.token();
        assert_eq!(token.len(), 16, "64-bit hex token: {token}");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(cookie_safe(&token), "generated tokens travel in cookies");
    }

    #[test]
    fn cookie_header_carries_httponly_and_samesite() {
        let header = token_cookie_header("0123abcd").expect("safe token");
        let text = header.to_str().expect("ascii header");
        assert!(text.contains("rutter_token=0123abcd"));
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Strict"));
        assert!(token_cookie_header("not safe").is_none());
    }

    /// Launcher stub satisfying the manager constructor; the dashboard
    /// tests never launch an engine through it.
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
