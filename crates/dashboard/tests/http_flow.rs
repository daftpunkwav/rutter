//! Functional tests for the dashboard's HTTP routes: token gate, static
//! assets, the cookie exchange, decision posts, and the pending count.
//! The server under test is the real `DashboardServer::run` on loopback.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;
use std::time::Duration;

use rutter_dashboard::DashboardServer;
use rutter_engine::config::LaunchMode;
use rutter_policy::{ApprovalBroker, Decision, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;

use common::FlowLauncher;

/// A running dashboard: bound to a picked-free loopback port, with its
/// access file in a temp dir (tests run with piped stderr, so the
/// hand-off path is mandatory). The manager stays reachable so tests
/// park approvals on the very broker the server decides on.
struct Serving {
    base: String,
    token: String,
    client: reqwest::Client,
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

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    wait_up(&client, &base, &token).await;

    Serving {
        base,
        token,
        client,
        manager,
        _access_dir: access_dir,
    }
}

/// Polls `/` with the token until the server answers, so the tests never
/// depend on startup timing.
async fn wait_up(client: &reqwest::Client, base: &str, token: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if client
            .get(format!("{base}/"))
            .query(&[("token", token)])
            .send()
            .await
            .is_ok()
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dashboard did not start within 10 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_first_visit_exchanges_the_query_token_for_a_cookie() {
    let serving = serve().await;
    let response = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("index response");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("the token is exchanged for an HttpOnly cookie")
        .to_str()
        .expect("ascii cookie");
    assert!(
        set_cookie.contains("HttpOnly"),
        "the cookie must not be readable from script: {set_cookie}"
    );

    // Later requests authenticate through the cookie alone.
    let cookie = set_cookie.split(';').next().unwrap();
    for path in ["/", "/app.js", "/i18n/en.json"] {
        let response = serving
            .client
            .get(format!("{}{path}", serving.base))
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .expect("asset response");
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
    }
    let app_js = serving
        .client
        .get(format!("{}/app.js", serving.base))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .expect("app.js")
        .text()
        .await
        .expect("app.js body");
    assert!(
        app_js.contains("WebSocket"),
        "the embedded application script is served verbatim"
    );
}

#[tokio::test]
async fn requests_without_the_token_are_forbidden() {
    let serving = serve().await;
    for path in ["/", "/app.js", "/i18n/en.json", "/api/pending"] {
        let response = serving
            .client
            .get(format!("{}{path}", serving.base))
            .send()
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            reqwest::StatusCode::FORBIDDEN,
            "{path} must not answer to strangers"
        );
    }
}

#[tokio::test]
async fn a_wrong_token_is_forbidden_too() {
    let serving = serve().await;
    let response = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", "not-the-token")])
        .send()
        .await
        .expect("response");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn decision_posts_validate_then_decide_parked_approvals() {
    let serving = serve().await;
    let url = format!("{}/api/decisions", serving.base);
    let authed =
        |request: reqwest::RequestBuilder| request.query(&[("token", serving.token.as_str())]);

    let bad_json = authed(serving.client.post(&url).body("not json"))
        .send()
        .await
        .expect("bad json response");
    assert_eq!(bad_json.status(), reqwest::StatusCode::BAD_REQUEST);

    let missing_grant = authed(
        serving
            .client
            .post(&url)
            .json(&serde_json::json!({"request_id": "apr-1"})),
    )
    .send()
    .await
    .expect("missing grant response");
    assert_eq!(missing_grant.status(), reqwest::StatusCode::BAD_REQUEST);

    let unknown = authed(
        serving
            .client
            .post(&url)
            .json(&serde_json::json!({"request_id": "apr-nope", "grant": true})),
    )
    .send()
    .await
    .expect("unknown id response");
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);

    // A parked approval decides through the endpoint.
    let broker = serving.manager.broker();
    let (id, receiver) = broker.open();
    let grant = authed(
        serving
            .client
            .post(&url)
            .json(&serde_json::json!({"request_id": id.as_str(), "grant": true})),
    )
    .send()
    .await
    .expect("grant response");
    assert_eq!(grant.status(), reqwest::StatusCode::OK);
    assert_eq!(
        receiver.await.expect("the decision is delivered"),
        Decision::Grant
    );
}

#[tokio::test]
async fn pending_counts_parked_approvals_and_unknown_paths_404() {
    let serving = serve().await;
    let pending = serving
        .client
        .get(format!("{}/api/pending", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("pending response")
        .text()
        .await
        .expect("pending body");
    assert_eq!(pending, "{\"pending\":0}\n");

    // One parked approval is visible to operators.
    let broker = serving.manager.broker();
    let (_id, _receiver) = broker.open();
    let pending = serving
        .client
        .get(format!("{}/api/pending", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("pending response")
        .text()
        .await
        .expect("pending body");
    assert_eq!(pending, "{\"pending\":1}\n");

    let missing = serving
        .client
        .get(format!("{}/no-such-path", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("fallback response");
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}
