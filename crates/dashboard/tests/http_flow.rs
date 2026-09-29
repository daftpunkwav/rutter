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
    port: u16,
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
        port,
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
    // The decision endpoint is the one that grants a parked approval,
    // so an unauthenticated post must be refused before the body is
    // even looked at — the loop above only ever asked with GET.
    let broker = serving.manager.broker();
    let (id, receiver) = broker.open();
    let refused = serving
        .client
        .post(format!("{}/api/decisions", serving.base))
        .json(&serde_json::json!({"request_id": id.as_str(), "grant": true}))
        .send()
        .await
        .expect("decision response");
    assert_eq!(
        refused.status(),
        reqwest::StatusCode::FORBIDDEN,
        "an unauthenticated grant must never reach the broker"
    );
    assert!(
        broker.decide(&id, Decision::Deny),
        "the approval is still parked: the refused post decided nothing"
    );
    let _ = receiver.await;
}

/// A page served from another loopback port: the origin an attacker who
/// controls any local web server can put content on, and the one the
/// dashboard's gate has to refuse. It is *same-site* with the
/// dashboard — a site spans every port on a host — so the browser
/// attaches the `SameSite=Strict` token cookie to anything it sends
/// here, and the request that carries an `Origin` is a simple
/// cross-site POST the preflight never covers.
fn foreign_loopback_origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// The session cookie a first visit with the token hands out, so the
/// forged requests below carry the credential a browser would attach.
async fn session_cookie(serving: &Serving) -> String {
    let response = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("index response");
    response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("the first visit sets the cookie")
        .to_str()
        .expect("ascii cookie")
        .split(';')
        .next()
        .expect("the cookie pair")
        .to_owned()
}

#[tokio::test]
async fn a_page_on_another_loopback_port_cannot_grant_an_approval() {
    // The attack path this closes. Everything a browser would send is
    // here: a valid credential, a real parked approval, and a simple
    // cross-site POST that skips the CORS preflight. Only the port
    // separates the origin from the dashboard's own, and `SameSite`
    // does not — a site is a host, not a host:port.
    let serving = serve().await;
    let cookie = session_cookie(&serving).await;
    let origin = foreign_loopback_origin();

    let broker = serving.manager.broker();
    let (id, receiver) = broker.open();
    let refused = serving
        .client
        .post(format!("{}/api/decisions", serving.base))
        .header(reqwest::header::COOKIE, &cookie)
        .header(reqwest::header::ORIGIN, &origin)
        .header(reqwest::header::CONTENT_TYPE, "text/plain;charset=UTF-8")
        .body(serde_json::json!({"request_id": id.as_str(), "grant": true}).to_string())
        .send()
        .await
        .expect("decision response");
    assert_eq!(
        refused.status(),
        reqwest::StatusCode::FORBIDDEN,
        "a grant posted from another loopback port must never reach the broker"
    );
    assert!(
        broker.decide(&id, Decision::Deny),
        "the approval is still parked: the forged post decided nothing"
    );
    let _ = receiver.await;
}

#[tokio::test]
async fn a_page_on_another_loopback_port_cannot_open_the_decision_socket() {
    // The same forgery over the WebSocket, which carries decisions too
    // and is a cross-site request a browser will happily make: a
    // handshake is not covered by the same-origin policy, so only the
    // endpoint gate stands between a foreign origin and a socket it
    // can drive. A raw handshake is written because the upgrade has to
    // be well formed for the handler's gate to be the thing that
    // answers — a malformed one would be refused by the extractor and
    // prove nothing.
    let serving = serve().await;
    let cookie = session_cookie(&serving).await;
    let request = format!(
        "GET /ws HTTP/1.1\r\n\
         host: 127.0.0.1:{port}\r\n\
         origin: {origin}\r\n\
         cookie: {cookie}\r\n\
         connection: Upgrade\r\n\
         upgrade: websocket\r\n\
         sec-websocket-version: 13\r\n\
         sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         \r\n",
        port = serving.port,
        origin = foreign_loopback_origin(),
    );

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", serving.port))
        .await
        .expect("server reachable");
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(request.as_bytes())
        .await
        .expect("write handshake");
    // Only the status line: a refused upgrade leaves the connection
    // open for a keep-alive request, so reading to end of stream would
    // wait for a close that never comes.
    let mut status = String::new();
    stream
        .read_line(&mut status)
        .await
        .expect("read handshake answer");
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "the decision socket must not open for another loopback origin: {status}"
    );
}

#[tokio::test]
async fn the_dashboards_own_origin_still_decides() {
    // The other half: the gate must not cost the operator their own
    // approvals. The frontend builds every request from
    // `window.location`, so the browser sends exactly this origin.
    let serving = serve().await;
    let cookie = session_cookie(&serving).await;
    let broker = serving.manager.broker();
    let (id, receiver) = broker.open();

    let accepted = serving
        .client
        .post(format!("{}/api/decisions", serving.base))
        .header(reqwest::header::COOKIE, &cookie)
        .header(
            reqwest::header::ORIGIN,
            format!("http://127.0.0.1:{}", serving.port),
        )
        .json(&serde_json::json!({"request_id": id.as_str(), "grant": true}))
        .send()
        .await
        .expect("decision response");
    assert_eq!(
        accepted.status(),
        reqwest::StatusCode::OK,
        "the operator's own grant must land"
    );
    assert_eq!(
        receiver.await.expect("the parked approval was decided"),
        Decision::Grant,
        "the broker saw the grant"
    );
}

/// Reads a header as text, or `""` when the response carries none.
fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn every_answer_carries_the_response_hardening_headers() {
    // One middleware, so a route added later inherits the headers
    // instead of having to remember them. The refusals and the
    // fallback are covered too: they are answers like any other, and a
    // hardening header that only rode on the happy path would be a
    // route-by-route promise nobody keeps.
    let serving = serve().await;

    let authed = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("index response");
    let refused = serving
        .client
        .get(format!("{}/", serving.base))
        .send()
        .await
        .expect("refused response");
    let missing = serving
        .client
        .get(format!("{}/no-such-path", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("fallback response");

    for (label, response) in [
        ("authed", &authed),
        ("refused", &refused),
        ("fallback", &missing),
    ] {
        assert_eq!(header(response, "cache-control"), "no-store", "{label}");
        assert_eq!(
            header(response, "referrer-policy"),
            "no-referrer",
            "{label}"
        );
        assert_eq!(
            header(response, "x-content-type-options"),
            "nosniff",
            "{label}"
        );
        assert_eq!(header(response, "x-frame-options"), "DENY", "{label}");
        assert_eq!(
            header(response, "content-security-policy"),
            "frame-ancestors 'none'",
            "{label}"
        );
    }
}

#[tokio::test]
async fn the_approval_ui_refuses_to_be_framed() {
    // The clickjacking path: `SameSite=Strict` keeps the cookie off a
    // cross-site request, but the first visit authenticates from the
    // query string, so a holder of the token could otherwise embed the
    // approval card in its own page and overlay the Grant button. The
    // framing headers are what refuse that embedding.
    let serving = serve().await;
    let response = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("index response");

    assert_eq!(header(&response, "x-frame-options"), "DENY");
    let policy = header(&response, "content-security-policy");
    assert!(
        policy.contains("frame-ancestors 'none'"),
        "every ancestor is refused, not just the parent: {policy}"
    );
}

#[tokio::test]
async fn the_token_bearing_url_is_never_stored_or_refered() {
    // The dashboard hands its token out in a query string, so the
    // response that carries it must tell the browser to keep neither
    // the page nor its URL: an operator's history and disk cache are
    // exactly the places a credential must not settle.
    let serving = serve().await;
    let response = serving
        .client
        .get(format!("{}/", serving.base))
        .query(&[("token", serving.token.as_str())])
        .send()
        .await
        .expect("index response");

    assert_eq!(header(&response, "cache-control"), "no-store");
    assert_eq!(header(&response, "referrer-policy"), "no-referrer");
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
