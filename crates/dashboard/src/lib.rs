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
//!
//! Module layout: [`auth`] owns the endpoint gate (host + token),
//! [`ws`] owns the WebSocket loop; this file owns the server, routes,
//! and static handlers.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod auth;
mod ws;

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use rutter_policy::{ApprovalBroker, ApprovalId, Decision};
use rutter_session::manager::SessionManager;
use serde_json::Value;

/// Static frontend sources, embedded at compile time (no build step,
/// blueprint §7.7).
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
pub(crate) struct Dashboard {
    pub(crate) manager: Arc<SessionManager>,
    pub(crate) broker: Arc<ApprovalBroker>,
    pub(crate) token: String,
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

async fn index(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, StatusCode> {
    if !auth::access_allowed(&state, &headers, &query) {
        return Err(StatusCode::FORBIDDEN);
    }
    // First visit carries the token in the query; the answer exchanges
    // it for a session cookie (blueprint §7.7), and later requests
    // authenticate through the cookie alone.
    let mut response = Html(assets::INDEX_HTML).into_response();
    if let Some(cookie) = auth::token_cookie_header(&state.token) {
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
    if !auth::access_allowed(&state, &headers, &query) {
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
    if !auth::access_allowed(&state, &headers, &query) {
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
    if !auth::access_allowed(&state, &headers, &query) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(upgrade.on_upgrade(move |socket| ws::ws_loop(state, socket)))
}

/// Decisions may also arrive as plain HTTP posts from scripts.
async fn decide(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: String,
) -> Result<StatusCode, StatusCode> {
    if !auth::access_allowed(&state, &headers, &query) {
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
        assert!(
            auth::cookie_safe(&token),
            "generated tokens travel in cookies"
        );
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
