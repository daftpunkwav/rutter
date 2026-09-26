//! The supervision dashboard: local web server over the event backbone
//! and the approval broker.
//!
//! Responsibilities:
//! - Serve the static frontend (embedded, no build step) on
//!   127.0.0.1 only, gated by a per-launch random token; the first
//!   visit exchanges the query token for an HttpOnly session cookie,
//!   and `Host` headers are validated against DNS rebinding
//!   (docs/dashboard.md).
//! - Hand that token's URL to a human through a channel chosen by who
//!   owns stderr: a terminal gets the URL, a piped stderr (the MCP
//!   client's case) gets an owner-only file and only its path is
//!   printed. See `DashboardServer::hand_off`.
//! - Stream events over one WebSocket: replay first, then live; the
//!   client submits approval decisions through the same socket.
//!
//! Boundary: observation plus verdict submission — the dashboard never
//! executes actions (docs/architecture.md). The screencast live view (docs/dashboard.md)
//! streams binary frames on demand through the same WebSocket.
//!
//! Module layout: `auth` owns the endpoint gate (host + token),
//! `ws` owns the WebSocket loop; this file owns the server, routes,
//! and static handlers.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod auth;
mod ws;

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
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
/// docs/dashboard.md).
mod assets {
    /// The dashboard page shell.
    pub const INDEX_HTML: &str = include_str!("../../../frontend/src/index.html");
    /// The dashboard application script.
    pub const APP_JS: &str = include_str!("../../../frontend/src/app.js");
    /// The English string catalog (default locale, docs/dashboard.md).
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
    /// Minted once, in [`DashboardServer::new`]: every reader must be
    /// handed the value the server actually accepts.
    token: String,
    /// Where the access URL's hand-off file lands when stderr is not a
    /// terminal. The file is named inside `run`, after the bind, so the
    /// name carries the port the server actually owns.
    access_dir: Option<PathBuf>,
}

impl DashboardServer {
    /// Binds the dashboard to a port. Approval decisions always go
    /// through the manager's broker — the one the sessions park on — so
    /// it is derived here, never passed in. `access_dir` is the private
    /// hand-off directory for the access URL; it is required whenever
    /// stderr has no terminal (see `hand_off`).
    pub fn new(manager: Arc<SessionManager>, port: u16, access_dir: Option<PathBuf>) -> Self {
        Self {
            broker: manager.broker(),
            token: generate_token(),
            manager,
            port,
            access_dir,
        }
    }

    /// The per-launch access token (docs/dashboard.md).
    pub fn token(&self) -> String {
        self.token.clone()
    }

    /// Serves until the process exits. The listener binds before the
    /// hand-off so the URL and its file name carry the port the server
    /// actually owns: `--dashboard 0` names its real port, and an
    /// instance that lost the bind race never touches the hand-off file
    /// of the instance that won it.
    pub async fn run(self) -> Result<(), String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", self.port))
            .await
            .map_err(|error| format!("cannot bind 127.0.0.1:{}: {error}", self.port))?;
        let bound_port = listener
            .local_addr()
            .map_err(|error| format!("cannot read the dashboard bind: {error}"))?
            .port();
        let hand_off = self.hand_off(bound_port)?;
        eprintln!("rutter: dashboard on http://127.0.0.1:{bound_port} — {hand_off}");
        let state = Dashboard {
            manager: self.manager,
            broker: self.broker,
            token: self.token,
        };

        let app = Router::new()
            .route("/", get(index))
            .route("/app.js", get(app_js))
            .route("/i18n/en.json", get(i18n))
            .route("/ws", get(ws_upgrade))
            .route("/api/decisions", post(decide))
            .route("/api/pending", get(pending))
            .fallback(not_found)
            .with_state(state);

        axum::serve(listener, app)
            .await
            .map_err(|error| format!("dashboard server failed: {error}"))
    }

    /// Puts the access URL where a human will find it, choosing the
    /// channel by who sits on the other end of stderr.
    ///
    /// A terminal means a person typed the command, so the URL goes out
    /// as it is. A pipe means a process did — and over the MCP transport
    /// that process is the very client rutter exists to supervise, so
    /// the token never travels that way: it lands in an owner-only file
    /// and the printed line names the path instead. The file is named
    /// per bound port, so two rutter processes on one machine — the
    /// multi-instance deployment — write different files and neither
    /// hand-off can clobber the other (docs/dashboard.md §2).
    ///
    /// This is defence in depth, not isolation. A supervised process
    /// running as the same user can still read that file; a deployment
    /// needing a channel the agent cannot observe has to run the
    /// dashboard outside the agent's account (docs/dashboard.md §2).
    fn hand_off(&self, bound_port: u16) -> Result<String, String> {
        let url = format!("http://127.0.0.1:{bound_port}/?token={}", self.token);
        if std::io::stderr().is_terminal() {
            return Ok(format!("open {url}"));
        }
        let Some(dir) = self.access_dir.as_ref() else {
            return Err(
                "stderr is no terminal and no access hand-off path was configured".to_owned(),
            );
        };
        let path = dir.join(format!("dashboard-access-{bound_port}.url"));
        write_private(&path, &url)?;
        Ok(format!(
            "access URL in {}; open it from a terminal you control",
            path.display()
        ))
    }
}

/// The per-launch token: launch time and process id hashed with a
/// randomly keyed hasher seeded from OS entropy, so the value is not
/// predictable from launch circumstances alone. `RUTTER_DASHBOARD_TOKEN`
/// overrides it for automation — but that override travels in the
/// environment of the process being supervised, so it belongs to a
/// trusted launcher rather than to a human-only channel.
fn generate_token() -> String {
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

/// Writes the access URL so that only its owner can read it on Unix,
/// replacing any earlier copy: the mode applies at creation, so a stale
/// file's permissions must never be inherited. Windows has no in-tree
/// permission bits, and the file keeps its directory's ACL there.
fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot replace {}: {error}", path.display())),
    }
    let mut options = std::fs::File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
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
    // it for a session cookie (docs/dashboard.md), and later requests
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

/// How many approvals are parked right now, for operators and monitoring.
/// The number is visible without a browser: a decision waiting on nobody is
/// the failure mode worth detecting.
async fn pending(
    State(state): State<Dashboard>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<String, StatusCode> {
    if !auth::access_allowed(&state, &headers, &query) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(format!(
        "{{\"pending\":{}}}\n",
        state.broker.pending_count()
    ))
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
            None,
        );
        let token = server.token();
        assert_eq!(token.len(), 16, "64-bit hex token: {token}");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(
            auth::cookie_safe(&token),
            "generated tokens travel in cookies"
        );
    }

    #[test]
    fn the_access_file_holds_one_url_and_replaces_an_earlier_one() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("access.url");
        write_private(&path, "first").expect("first write");
        write_private(&path, "second").expect("rewrite over the old copy");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "second\n",
            "one launch's URL per file, never two appended"
        );
    }

    #[test]
    fn the_hand_off_file_names_the_port_the_server_actually_owns() {
        let dir = tempfile::tempdir().expect("temp dir");
        let server = DashboardServer::new(
            Arc::new(SessionManager::new(
                Arc::new(UnsupportedLauncher),
                rutter_engine::config::LaunchMode::Headless,
                rutter_session::config::SessionConfig::default(),
                Arc::new(rutter_policy::RuleSet::default_set()),
                Arc::new(rutter_policy::ApprovalBroker::new()),
                None,
            )),
            7700,
            Some(dir.path().to_path_buf()),
        );

        // The bound port names the file and the URL: `--dashboard 0`
        // lands under its real port, and an instance that lost the bind
        // race (bound port taken) never writes at all.
        let message = server.hand_off(7701).expect("hand-off");
        let path = dir.path().join("dashboard-access-7701.url");
        assert!(
            message.contains(&path.display().to_string()),
            "the printed line names the file: {message}"
        );
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(
            written.contains("http://127.0.0.1:7701/?token="),
            "the URL carries the bound port: {written}"
        );
        assert!(
            written.contains(&server.token()),
            "the URL carries this launch's token: {written}"
        );
        assert!(
            !dir.path().join("dashboard-access-7700.url").exists(),
            "the configured port never names the file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_access_file_is_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("access.url");
        write_private(&path, "secret").expect("write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the access token stays owner-only");
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
