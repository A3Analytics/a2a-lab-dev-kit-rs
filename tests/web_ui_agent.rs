//! Shared lab refusals after the agent page was removed.

#![cfg(feature = "web-ui")]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
const SCHEME: &str = "This agent requires a security scheme this page cannot use.";
const REFUSE: &str = "This page will not send credentials or call an unsupported agent.";
const TOKEN: &str = "agent-page-token";
const ROUTES: [&str; 5] = ["#logs", "#metrics", "#tasks", "#images", "#chat"];

const VISIBLE: &str = r"(needle) => {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (!node.textContent || !node.textContent.includes(needle)) continue;
    const el = node.parentElement;
    if (!el || el.closest('[hidden]') || el.closest('details:not([open])')) continue;
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    for (const rect of range.getClientRects()) {
      if (rect.width > 2 && rect.height > 2 && rect.bottom > 0 && rect.right > 0 && rect.top < window.innerHeight && rect.left < window.innerWidth) return true;
    }
  }
  return false;
}";

#[tokio::test]
async fn browser_agent_page_is_absent() {
    let base = serve(populated_card().to_string()).await;
    tokio::task::spawn_blocking(move || absent_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_agent_refusals_stay_visible_on_every_route() {
    let locked = serve(locked_card().to_string()).await;
    let bearer = serve(bearer_card().to_string()).await;
    tokio::task::spawn_blocking(move || {
        refusal_flow(&locked);
        bearer_flow(&bearer);
    })
    .await
    .unwrap();
}

fn absent_flow(base: &str) {
    let mut page = open_agent(base, "connected");
    assert_title(&mut page, "Logs");
    assert_eq!(
        page.eval("document.getElementById('nav-agent')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('view-agent')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('bearer-token')"),
        Value::Null
    );
    for hash in ["#agent", "#overview", "#nope"] {
        page.show(hash);
        assert_title(&mut page, "Logs");
    }
}

fn refusal_flow(base: &str) {
    let mut page = open_agent(base, "unsupported");
    page.viewport(1280, 900);
    assert_scheme_visible(&mut page);
    page.show("#tasks");
    page.wait_until(&format!(
        "document.getElementById('alert')?.textContent.includes({})",
        serde_json::to_string(REFUSE).unwrap_or_default()
    ));
    assert_absent(&mut page);
    assert!(
        !page.urls().iter().any(|url| url.contains("/message:send")),
        "unsupported agent was called"
    );
    assert_refusals(&mut page);
}

fn assert_scheme_visible(page: &mut Browser) {
    assert_absent(page);
    assert_alert_outside(page);
    for route in ROUTES {
        page.show(route);
        assert_title(page, route_title(route));
        assert_absent(page);
        assert_visible(page, SCHEME);
    }
}

fn assert_refusals(page: &mut Browser) {
    for route in ROUTES {
        page.show(route);
        assert_title(page, route_title(route));
        assert_absent(page);
        assert_alert_outside(page);
        assert_visible(page, SCHEME);
        assert_visible(page, REFUSE);
    }
}

fn bearer_flow(base: &str) {
    let mut page = open_agent(base, "connected");
    assert_eq!(
        page.eval("document.getElementById('bearer-token')?.type"),
        json!("password")
    );
    assert_eq!(
        page.eval("document.getElementById('bearer-token')?.autocomplete"),
        json!("off")
    );
    assert_eq!(
        page.eval("!!document.getElementById('bearer-token')?.closest('header.banner')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("!!document.querySelector('label[for=\"bearer-token\"]')"),
        Value::Bool(true)
    );
    page.set_value("bearer-token", TOKEN);
    let stored = page
        .eval("JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie");
    assert!(!stored.as_str().unwrap_or("").contains(TOKEN), "{stored}");
    page.reload();
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    assert_eq!(
        page.eval("document.getElementById('bearer-token')?.value"),
        json!("")
    );
    assert_absent(&mut page);
}

fn open_agent(base: &str, state: &str) -> Browser {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(&format!(
        "document.getElementById('connection')?.dataset.state === '{state}'"
    ));
    page
}

fn assert_title(page: &mut Browser, title: &str) {
    let title = serde_json::to_string(title).unwrap_or_default();
    let shown = page.eval(&format!(
        "(() => {{ const headings = [...document.querySelectorAll('h1')]; return headings.length === 1 && headings[0].id === 'view-title' && headings[0].textContent.trim() === {title}; }})()"
    ));
    assert_eq!(shown, Value::Bool(true), "{title}");
}

fn assert_visible(page: &mut Browser, needle: &str) {
    let needle_json = serde_json::to_string(needle).unwrap_or_default();
    let shown = page.eval(&format!("({VISIBLE})({needle_json})"));
    assert_eq!(
        shown,
        Value::Bool(true),
        "{needle}: {}",
        page.eval("document.body.innerText")
    );
}

fn assert_absent(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('nav-agent')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('view-agent')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('protocol-advanced')"),
        Value::Null
    );
}

fn assert_alert_outside(page: &mut Browser) {
    assert_eq!(
        page.eval(
            "(() => { const el = document.getElementById('alert'); return !!el && !el.hasAttribute('hidden') && !el.closest('#protocol-advanced') && !el.closest('[hidden]'); })()",
        ),
        Value::Bool(true)
    );
}

fn route_title(hash: &str) -> &'static str {
    match hash {
        "#metrics" => "Metrics",
        "#tasks" => "Tasks",
        "#images" => "Images",
        "#chat" => "Chat",
        _ => "Logs",
    }
}

fn populated_card() -> Value {
    json!({
        "name": "a2a-lab",
        "description": "Lab logs, metrics, tasks, and images",
        "version": "0.1.0",
        "supportedInterfaces": [http_interface()],
        "capabilities": {"streaming": true},
        "skills": [{"id": "list-tasks", "name": "List tasks", "description": "List"}]
    })
}

fn locked_card() -> Value {
    json!({
        "name": "locked",
        "description": "mtls only",
        "version": "2",
        "supportedInterfaces": [http_interface()],
        "capabilities": {"streaming": false},
        "skills": [{"id": "list-tasks", "name": "List", "description": "List"}],
        "securitySchemes": {"mtls": {"mtlsSecurityScheme": {}}},
        "securityRequirements": [{"mtls": []}]
    })
}

fn bearer_card() -> Value {
    json!({
        "name": "bearer-agent",
        "description": "token",
        "version": "1",
        "supportedInterfaces": [http_interface()],
        "capabilities": {"streaming": false},
        "skills": [{"id": "list-tasks", "name": "List", "description": "List"}],
        "securitySchemes": {
            "bearerAuth": {"httpAuthSecurityScheme": {"scheme": "bearer"}}
        },
        "securityRequirements": [{"bearerAuth": []}]
    })
}

fn http_interface() -> Value {
    json!({
        "url": "http://127.0.0.1/agent",
        "protocolBinding": "HTTP+JSON",
        "protocolVersion": "1.0"
    })
}

#[derive(Clone)]
struct MockState {
    body: String,
    pkg: PathBuf,
}

async fn serve(body: String) -> String {
    let state = MockState {
        body,
        pkg: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/web-ui-wasm/pkg"),
    };
    let app = Router::new()
        .route(
            "/.well-known/agent-card.json",
            axum::routing::get(card_route),
        )
        .route("/ui", axum::routing::get(index_route))
        .route("/ui/", axum::routing::get(index_route))
        .route("/ui/index.html", axum::routing::get(index_route))
        .route("/ui/assets/{*path}", axum::routing::get(asset_route))
        .route("/ui/{*path}", axum::routing::get(index_route))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|_| panic!("bind mock ui"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|_| panic!("mock ui address"));
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .unwrap_or_else(|_| panic!("serve mock ui"));
    });
    format!("http://{address}")
}

async fn card_route(State(state): State<MockState>) -> Response {
    reply(StatusCode::OK, "application/json", state.body.into_bytes())
}

async fn index_route(State(state): State<MockState>) -> Response {
    let body = std::fs::read(state.pkg.join("index.html")).unwrap_or_default();
    reply(StatusCode::OK, "text/html; charset=utf-8", body)
}

async fn asset_route(State(state): State<MockState>, Path(name): Path<String>) -> Response {
    if !safe_asset(&name) {
        return reply(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        );
    }
    let body = std::fs::read(state.pkg.join("assets").join(&name)).unwrap_or_default();
    if body.is_empty() {
        return reply(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        );
    }
    reply(StatusCode::OK, content_type(&name), body)
}

fn safe_asset(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
        })
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn reply(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(POLICY));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}
