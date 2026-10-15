#![cfg(feature = "web-ui")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, MemoryLogs, MemoryMetrics, MemoryTasks,
    MetricDescriptor, MetricId, MetricPoint, SecurityScheme, TaskDefinition, TaskId, UtcTimestamp,
    bind_local,
};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
mod browser;
#[path = "harness/chat.rs"]
mod chat;

#[path = "harness/inventory.rs"]
mod inventory_browser;
#[path = "harness/workbench.rs"]
mod workbench_browser;

use browser::{Browser, header, request_method, request_url};

const POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
const MARKUP: &str = r#"<img id="xss-payload" src=x onerror="window.__xss=1">"#;
const TOKEN: &str = "page-lifetime-token";
const COMMAND: &str = r#"{"operation":"list_tasks","params":{"page":{"limit":10}}}"#;

#[tokio::test]
async fn browser_shell_reads_the_agent_card_and_exchanges_messages() {
    let base = lab(false).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || anonymous_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_bearer_token_stays_on_the_page() {
    let base = lab(true).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || bearer_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_untrusted_text_cannot_change_the_dom() {
    let base = mock(xss_card(), false, false).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || untrusted_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_navigation_keeps_one_view_and_its_state() {
    let base = lab(false).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || navigation_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_error_states_stay_distinct() {
    let errors = mock(anonymous_card("errors"), true, true).await;
    let grpc = mock(grpc_card(), false, false).await;
    tokio::task::spawn_blocking(move || error_flow(&errors, &grpc))
        .await
        .unwrap();
}

fn anonymous_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    assert_identity(&page.snapshot(), base);
    assert_layout(&mut page);
    exchange(&mut page);
    recover(&mut page);
    assert_same_origin(&page, base);
}

fn assert_identity(view: &Value, base: &str) {
    assert_eq!(view["name"], "a2a-lab");
    assert_eq!(view["access"], "anonymous");
    assert_eq!(view["bearer"], false);
    assert_card_text(view["text"].as_str().unwrap_or(""));
    assert_eq!(view["nav"], true);
    assert_eq!(view["skip"], true);
    assert_eq!(view["live"], "polite");
    assert!(
        view["unlabeled"]
            .as_array()
            .unwrap_or(&Vec::new())
            .is_empty()
    );
    assert_eq!(view["blankButtons"], 0);
    assert!(view["focus"].as_str().unwrap_or("").contains("3px"));
    assert!(
        view["wasm"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .any(|url| url.as_str().unwrap_or("").starts_with(base))
    );
}

fn assert_card_text(text: &str) {
    assert!(text.contains("HTTP+JSON"));
    assert!(text.contains("list-tasks"));
    assert!(text.contains("Streaming: yes"));
    assert!(text.contains("0.1.0"));
}

fn assert_layout(page: &mut Browser) {
    page.viewport(360, 800);
    let narrow = page.snapshot();
    let width = narrow["width"].as_f64().unwrap_or(0.0);
    assert!(
        narrow["scroll"].as_u64().unwrap_or(u64::MAX) <= narrow["width"].as_u64().unwrap_or(0) + 24
    );
    let button = narrow["buttonWidth"].as_f64().unwrap_or(0.0);
    assert!(button > 0.0);
    assert!(button <= width);
    page.viewport(1280, 800);
    assert!(page.snapshot()["buttonWidth"].as_f64().unwrap_or(0.0) > 0.0);
}

fn exchange(page: &mut Browser) {
    open_protocol(page);
    page.set_value("message-text", "hello");
    page.focus("send-text");
    page.press_enter();
    page.wait_until("document.getElementById('output')?.textContent.includes('lab-profile')");
    let sent = page.snapshot();
    assert!(!sent["context"].as_str().unwrap_or("").is_empty());
    assert_eq!(sent["state"], "connected");
    page.set_value("command-json", COMMAND);
    page.click("send-command");
    page.wait_until("document.getElementById('output')?.textContent.includes('xss-payload')");
    let payload = page.snapshot();
    assert_eq!(payload["xss"], false);
    assert_eq!(payload["xssPayload"], false);
    assert_eq!(payload["images"], 0);
    assert_eq!(payload["inlineScripts"], 0);
    let output = payload["output"].as_str().unwrap_or("");
    assert!(output.contains("xss-payload"));
    assert!(output.contains("onerror"));
    assert!(output.contains("list_tasks"));
    page.click("list-tasks");
    page.wait_until("document.getElementById('output')?.textContent.includes('Task ')");
    let listed = page.snapshot();
    assert_eq!(listed["state"], "connected");
    assert_eq!(listed["nav"], true);
}

fn recover(page: &mut Browser) {
    page.fail_network(true);
    page.click("retry");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'offline'");
    page.show("#overview");
    let offline = page.snapshot();
    assert_eq!(offline["name"], "a2a-lab");
    assert_eq!(offline["nav"], true);
    open_protocol(page);
    page.fail_network(false);
    page.click("retry");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
}

fn assert_same_origin(page: &Browser, base: &str) {
    assert!(
        page.urls()
            .iter()
            .all(|url| url.starts_with(base) || url.starts_with("data:"))
    );
    let protocol = page.requests(base);
    assert!(
        protocol
            .iter()
            .any(|request| request_url(request).contains("/message:send"))
    );
    assert!(
        protocol
            .iter()
            .any(|request| request_url(request).contains("/tasks?"))
    );
    assert!(
        protocol
            .iter()
            .any(|request| request_url(request).ends_with("/.well-known/agent-card.json"))
    );
    assert!(
        protocol
            .iter()
            .all(|request| header(request, "authorization").is_none())
    );
}

fn bearer_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    let ready = page.snapshot();
    assert_eq!(ready["access"], "bearer");
    assert_eq!(ready["bearer"], true);
    open_protocol(&mut page);
    page.set_value("message-text", "hello");
    page.click("send-text");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'unauthorized'");
    page.show("#overview");
    let rejected = page.snapshot();
    assert_eq!(rejected["nav"], true);
    assert!(!rejected["alert"].as_str().unwrap_or("").contains(TOKEN));
    page.set_value("bearer-token", TOKEN);
    open_protocol(&mut page);
    page.click("send-text");
    page.wait_until("document.getElementById('output')?.textContent.includes('lab-profile')");
    let accepted = page.snapshot();
    assert_eq!(accepted["state"], "connected");
    let console = page.console_text();
    assert_hidden(&accepted, &console);
    assert_authorized(&page.requests(base));
    page.reload();
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    open_protocol(&mut page);
    let reloaded = page.snapshot();
    assert_eq!(reloaded["bearerValue"], "");
    page.set_value("message-text", "hello again");
    page.click("send-text");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'unauthorized'");
    let again = page.requests(base);
    let last = again
        .iter()
        .rev()
        .find(|request| request_url(request).contains("/message:send"));
    assert!(last.is_some_and(|request| header(request, "authorization").is_none()));
}

fn untrusted_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'unsupported'");
    let view = page.snapshot();
    assert_eq!(
        view["name"],
        r#"<img id="xss-name" src=x onerror="window.__xss=1">"#
    );
    assert!(
        view["text"]
            .as_str()
            .unwrap_or("")
            .contains("<script>window.__xss=1</script>")
    );
    assert!(view["text"].as_str().unwrap_or("").contains("mutual TLS"));
    assert!(
        view["text"]
            .as_str()
            .unwrap_or("")
            .contains("cannot satisfy")
    );
    assert_eq!(view["access"], "unsupported");
    assert_eq!(view["bearer"], false);
    assert_eq!(view["xss"], false);
    assert_eq!(view["xssName"], false);
    assert_eq!(view["xssSkill"], false);
    assert_eq!(view["images"], 0);
    assert_eq!(view["inlineScripts"], 0);
    assert_eq!(view["nav"], true);
    open_protocol(&mut page);
    page.click("send-text");
    assert!(
        !page
            .urls()
            .iter()
            .any(|url| url.contains("evil.example") || url.contains("/message:send"))
    );
}

fn error_flow(errors: &str, grpc: &str) {
    let mut page = Browser::open(&format!("{errors}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'loading'");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    open_protocol(&mut page);
    page.set_value("message-text", "hello");
    page.click("send-text");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'forbidden'");
    let forbidden = page.snapshot();
    assert!(
        forbidden["alert"]
            .as_str()
            .unwrap_or("")
            .contains("not allowed")
    );
    assert_eq!(forbidden["nav"], true);
    page.click("send-text");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'malformed'");
    let malformed = page.snapshot();
    assert!(
        malformed["alert"]
            .as_str()
            .unwrap_or("")
            .contains("could not be read")
    );
    assert!(
        !malformed["text"]
            .as_str()
            .unwrap_or("")
            .contains("not-json")
    );
    assert_eq!(malformed["nav"], true);
    page.click("retry");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.goto(&format!("{grpc}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'unsupported'");
    let unsupported = page.snapshot();
    assert_eq!(unsupported["name"], "grpc-only");
    assert!(
        unsupported["text"]
            .as_str()
            .unwrap_or("")
            .contains("does not advertise HTTP+JSON")
            || unsupported["alert"]
                .as_str()
                .unwrap_or("")
                .contains("HTTP+JSON")
    );
    assert_eq!(unsupported["nav"], true);
    open_protocol(&mut page);
    page.click("send-text");
    assert!(
        !page
            .urls()
            .iter()
            .any(|url| url.contains("/message:send") && url.starts_with(grpc))
    );
}

fn navigation_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    assert_eq!(
        page.eval("document.querySelectorAll('#main > .view').length"),
        json!(1)
    );
    assert_eq!(
        page.eval("document.getElementById('view-agent')?.parentElement?.id"),
        json!("main")
    );
    assert_eq!(
        page.eval("document.getElementById('view-chat')"),
        Value::Null
    );
    page.show("#chat");
    page.set_value("chat-draft", "keep-draft");
    page.show("#workbench");
    page.set_value("workbench-params", "{\"keep\":true}");
    page.show("#inventory/logs");
    page.set_value("inventory-logs-limit", "7");
    page.show("#overview");
    page.show("#chat");
    assert_eq!(
        page.eval("document.getElementById('chat-draft')?.value"),
        json!("keep-draft")
    );
    assert_eq!(
        page.eval("document.activeElement?.id"),
        json!("chat-heading")
    );
    page.show("#workbench");
    assert_eq!(
        page.eval("document.getElementById('workbench-params')?.value"),
        json!("{\"keep\":true}")
    );
    assert_eq!(
        page.eval("document.getElementById('protocol-advanced')?.open === true"),
        Value::Bool(false)
    );
    page.show("#inventory/metrics");
    assert_eq!(
        page.eval("document.getElementById('nav-metrics')?.getAttribute('aria-current')"),
        json!("page")
    );
    assert_eq!(
        page.eval("document.getElementById('inventory-logs')"),
        Value::Null
    );
    page.show("#inventory/logs");
    assert_eq!(
        page.eval("document.getElementById('inventory-logs-limit')?.value"),
        json!("7")
    );
    page.eval("location.hash = '#nope'");
    page.wait_until("document.getElementById('view-agent')?.parentElement?.id === 'main'");
    page.show("#chat");
    page.reload();
    page.wait_until(
        "document.getElementById('connection')?.dataset.state === 'connected' && document.getElementById('view-chat')?.parentElement?.id === 'main'",
    );
    page.viewport(360, 800);
    assert_eq!(
        page.eval("document.getElementById('primary-nav')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    page.click("nav-toggle");
    page.wait_until(
        "document.getElementById('nav-toggle')?.getAttribute('aria-expanded') === 'true'",
    );
    page.eval(
        "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
    );
    page.wait_until("document.getElementById('primary-nav')?.hasAttribute('hidden')");
    page.click("nav-toggle");
    page.click("nav-logs");
    page.wait_until(
        "document.getElementById('view-logs')?.parentElement?.id === 'main' && document.getElementById('nav-toggle')?.getAttribute('aria-expanded') === 'false'",
    );
    page.viewport(1280, 800);
    assert_eq!(
        page.eval("document.getElementById('primary-nav')?.hasAttribute('hidden')"),
        Value::Bool(false)
    );
    assert_eq!(
        page.eval("document.getElementById('nav-toggle')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_shell_scripts(&mut page);
}

fn assert_shell_scripts(page: &mut Browser) {
    assert_eq!(page.eval("document.scripts.length"), json!(3));
    assert_eq!(
        page.eval("[...document.scripts].filter(script => !script.src).length"),
        json!(0)
    );
    assert_eq!(
        page.eval("[...document.scripts].every(script => script.src.includes('/ui/assets/'))"),
        Value::Bool(true)
    );
}

fn open_protocol(page: &mut Browser) {
    page.show("#agent");
    if page.eval("document.getElementById('protocol-advanced')?.open === true") != Value::Bool(true)
    {
        page.click("protocol-advanced-summary");
    }
    page.wait_until("document.getElementById('protocol-advanced')?.open === true");
}

fn assert_hidden(snapshot: &Value, console: &str) {
    for key in ["text", "html", "alert", "output", "name", "href", "cookie"] {
        let value = snapshot[key].as_str().unwrap_or("");
        assert!(!value.contains(TOKEN), "{key} contains the bearer token");
    }
    assert!(
        snapshot["local"]
            .as_array()
            .unwrap_or(&Vec::new())
            .is_empty()
    );
    assert!(
        snapshot["session"]
            .as_array()
            .unwrap_or(&Vec::new())
            .is_empty()
    );
    assert!(!console.contains(TOKEN));
}

fn assert_authorized(requests: &[Value]) {
    let card = requests
        .iter()
        .find(|request| request_url(request).ends_with("/.well-known/agent-card.json"));
    assert!(card.is_some_and(|request| header(request, "authorization").is_none()));
    let send = requests.iter().rev().find(|request| {
        request_url(request).contains("/message:send") && request_method(request) == "POST"
    });
    let auth = send.and_then(|request| header(request, "authorization"));
    let expected = format!("Bearer {TOKEN}");
    assert_eq!(auth, Some(expected.as_str()));
    assert!(
        requests
            .iter()
            .all(|request| !request_url(request).contains(TOKEN))
    );
}

#[tokio::test]
async fn browser_metrics_chart_draws_switches_and_stays_accessible() {
    let base = metric_lab().await;
    tokio::task::spawn_blocking(move || chart_flow(&base))
        .await
        .unwrap();
}

fn chart_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("metrics");
    page.wait_until(
        "window.Chart && window.Chart.getChart(document.getElementById('metric-chart'))",
    );
    assert_initial_series(&mut page);
    assert_point_tooltip(&mut page);
    assert_metric_switch(&mut page);
    assert_chart_resizes(&mut page);
    assert_value_filter(&mut page);
    let console = page.console_text();
    assert!(!console.contains("EvalError"), "{console}");
    assert!(!console.contains("Content Security Policy"), "{console}");
    assert!(!console.contains("Refused to"), "{console}");
}

fn assert_initial_series(page: &mut Browser) {
    let initial = page.eval(CHART_STATE);
    assert_eq!(initial["values"], json!([0, 1]));
    assert_eq!(initial["label"], "2 samples in open");
    assert_eq!(initial["hidden"], Value::Bool(false));
    assert_eq!(initial["svg"], Value::Bool(false));
    let readings = initial["readings"].as_str().unwrap_or_default();
    assert!(
        readings.contains("2024-01-01T00:01:00Z 1 open"),
        "{initial}"
    );
    let width = initial["width"].as_f64().unwrap_or(0.0);
    assert!(width > 200.0, "{initial}");
}

fn assert_point_tooltip(page: &mut Browser) {
    let point = page.eval(
        "(() => {
            const canvas = document.getElementById('metric-chart');
            const chart = window.Chart.getChart(canvas);
            const point = chart.getDatasetMeta(0).data[1];
            const rect = canvas.getBoundingClientRect();
            return { x: rect.left + point.x, y: rect.top + point.y };
        })()",
    );
    page.move_mouse(
        point["x"].as_f64().unwrap_or(0.0),
        point["y"].as_f64().unwrap_or(0.0),
    );
    let tip = page.eval(
        "(() => {
            const tooltip = window.Chart.getChart(document.getElementById('metric-chart')).tooltip;
            return {
                opacity: tooltip.opacity,
                title: (tooltip.title || []).join(' '),
                body: (tooltip.body || []).map(part => (part.lines || []).join(' ')).join(' ')
            };
        })()",
    );
    let title = tip["title"].as_str().unwrap_or_default();
    let body = tip["body"].as_str().unwrap_or_default();
    let opacity = tip["opacity"].as_f64().unwrap_or(0.0);
    assert!(opacity > 0.0, "{tip}");
    assert!(title.contains("2024-01-01T00:01:00Z"), "{tip}");
    assert!(body.contains('1'), "{tip}");
    assert!(body.contains("open"), "{tip}");
}

fn assert_metric_switch(page: &mut Browser) {
    page.click("metric-pick-1");
    page.wait_until(
        "window.Chart.getChart(document.getElementById('metric-chart')).data.datasets[0].data.includes(12)",
    );
    let switched = page.eval(CHART_STATE);
    assert_eq!(switched["values"], json!([5, 12]));
    assert_eq!(switched["label"], "2 samples in ms");
}

fn assert_chart_resizes(page: &mut Browser) {
    page.viewport(1280, 900);
    let wide = page.eval("document.getElementById('metric-chart').getBoundingClientRect().width");
    page.viewport(520, 900);
    let narrow = page.eval("document.getElementById('metric-chart').getBoundingClientRect().width");
    let wide = wide.as_f64().unwrap_or(0.0);
    let narrow = narrow.as_f64().unwrap_or(0.0);
    assert!(wide > narrow + 40.0, "{wide} vs {narrow}");
}

fn assert_value_filter(page: &mut Browser) {
    page.eval(
        "(() => {
            const input = document.getElementById('metric-filter-search');
            input.value = '12';
            input.dispatchEvent(new Event('change', { bubbles: true }));
            return true;
        })()",
    );
    page.wait_until(
        "window.Chart.getChart(document.getElementById('metric-chart')).data.datasets[0].data.length === 1",
    );
    let filtered = page.eval(CHART_STATE);
    assert_eq!(filtered["values"], json!([12]));
    assert_eq!(filtered["ariaHidden"], Value::Null);
    let readings = filtered["readings"].as_str().unwrap_or_default();
    assert!(readings.contains("12 ms"), "{filtered}");
}

const CHART_STATE: &str = "(() => {
    const canvas = document.getElementById('metric-chart');
    const chart = window.Chart.getChart(canvas);
    const list = document.getElementById('metric-view-entries');
    return {
        values: chart.data.datasets[0].data,
        label: canvas.getAttribute('aria-label'),
        readings: list.textContent,
        ariaHidden: list.getAttribute('aria-hidden'),
        hidden: document.getElementById('metric-plot').hasAttribute('hidden'),
        svg: !!document.querySelector('#metric-plot svg'),
        width: canvas.getBoundingClientRect().width
    };
})()";

async fn metric_lab() -> String {
    let metrics = MemoryMetrics::new();
    seed_metric(
        &metrics,
        "door",
        "Door",
        "open",
        &[("2024-01-01T00:00:00Z", 0.0), ("2024-01-01T00:01:00Z", 1.0)],
    )
    .await;
    seed_metric(
        &metrics,
        "latency",
        "Latency",
        "ms",
        &[
            ("2024-01-01T00:00:00Z", 5.0),
            ("2024-01-01T00:01:00Z", 12.0),
        ],
    )
    .await;
    let service = A2aLabService::new(MemoryLogs::new(), metrics, MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn seed_metric(
    metrics: &MemoryMetrics,
    id: &str,
    name: &str,
    unit: &str,
    points: &[(&str, f64)],
) {
    let metric_id = MetricId::new(id).unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: name.to_owned(),
            description: name.to_owned(),
            unit: unit.to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    for (timestamp, value) in points {
        metrics
            .insert_point(
                metric_id.clone(),
                MetricPoint::new(UtcTimestamp::parse(timestamp).unwrap(), *value).unwrap(),
            )
            .await
            .unwrap();
    }
}

async fn lab(secured: bool) -> String {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: MARKUP.to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service).with_web_ui();
    if secured {
        let schemes = [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into();
        let requirements = vec![[("bearerAuth".to_owned(), Vec::new())].into()];
        server = server.with_security(schemes, requirements);
    }
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn mock(card: Value, errors: bool, delay: bool) -> String {
    serve_mock(
        card,
        errors,
        delay,
        MockRoute::Default,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}

#[derive(Clone, Copy)]
enum MockRoute {
    Default,
    Inventory,
    Workbench,
}

#[derive(Clone)]
struct MockState {
    card: String,
    pkg: PathBuf,
    sends: Arc<AtomicU32>,
    errors: bool,
    delay: bool,
    route: MockRoute,
    tasks_status: StatusCode,
    tasks_body: Vec<u8>,
}

async fn serve_mock(
    card: Value,
    errors: bool,
    delay: bool,
    route: MockRoute,
    tasks_status: StatusCode,
    tasks_body: Vec<u8>,
) -> String {
    let state = MockState {
        card: card.to_string(),
        pkg: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/web-ui-wasm/pkg"),
        sends: Arc::new(AtomicU32::new(0)),
        errors,
        delay,
        route,
        tasks_status,
        tasks_body,
    };
    let app = Router::new()
        .route(
            "/.well-known/agent-card.json",
            axum::routing::get(card_route),
        )
        .route("/ui", axum::routing::get(index_route))
        .route("/ui/", axum::routing::get(index_route))
        .route("/ui/index.html", axum::routing::get(index_route))
        .route("/ui/assets/{name}", axum::routing::get(asset_route))
        .route("/message:send", axum::routing::post(send_route))
        .route("/tasks", axum::routing::get(tasks_route))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}")
}

async fn card_route(State(state): State<MockState>) -> Response {
    if state.delay {
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    bytes(StatusCode::OK, "application/json", state.card.into_bytes())
}

async fn index_route(State(state): State<MockState>) -> Response {
    let body = std::fs::read(state.pkg.join("index.html")).unwrap_or_default();
    bytes(StatusCode::OK, "text/html; charset=utf-8", body)
}

async fn asset_route(State(state): State<MockState>, Path(name): Path<String>) -> Response {
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '.')
    {
        return bytes(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        );
    }
    let path = state.pkg.join("assets").join(&name);
    let body = std::fs::read(&path).unwrap_or_default();
    if body.is_empty() {
        return bytes(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        );
    }
    bytes(StatusCode::OK, content_type(&name), body)
}

async fn tasks_route(State(state): State<MockState>) -> Response {
    bytes(state.tasks_status, "application/json", state.tasks_body)
}

async fn send_route(State(state): State<MockState>, body: Bytes) -> Response {
    match state.route {
        MockRoute::Workbench => return workbench_browser::scripted_send(&body),
        MockRoute::Inventory => return inventory_browser::scripted_send(body).await,
        MockRoute::Default => {}
    }
    let hit = state.sends.fetch_add(1, Ordering::Relaxed);
    if state.errors && hit == 0 {
        return bytes(
            StatusCode::FORBIDDEN,
            "application/json",
            json!({
                "error": {
                    "code": 403,
                    "status": "PERMISSION_DENIED",
                    "message": "not allowed",
                    "details": [{
                        "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                        "reason": "PERMISSION_DENIED",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        );
    }
    if state.errors {
        return bytes(StatusCode::OK, "application/json", b"not-json".to_vec());
    }
    bytes(
        StatusCode::OK,
        "application/json",
        br#"{"nope":true}"#.to_vec(),
    )
}

fn bytes(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(POLICY));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn anonymous_card(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Lab",
        "version": "1",
        "supportedInterfaces": [{
            "url": "http://127.0.0.1",
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": "1.0"
        }],
        "capabilities": {"streaming": true},
        "skills": [{"id": "list-tasks", "name": "List tasks", "description": "List"}]
    })
}

fn xss_card() -> Value {
    json!({
        "name": "<img id=\"xss-name\" src=x onerror=\"window.__xss=1\">",
        "description": "<script>window.__xss=1</script>",
        "version": "1",
        "supportedInterfaces": [{
            "url": "http://evil.example/agent",
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": "1.0"
        }],
        "capabilities": {"streaming": false},
        "skills": [{
            "id": "list-tasks",
            "name": "<a id=\"xss-skill\" href=\"javascript:void(0)\">owned</a>",
            "description": "desc"
        }],
        "securitySchemes": {"mtls": {"mtlsSecurityScheme": {}}},
        "securityRequirements": [{"mtls": []}]
    })
}

fn grpc_card() -> Value {
    json!({
        "name": "grpc-only",
        "description": "no http",
        "version": "1",
        "supportedInterfaces": [{
            "url": "127.0.0.1:1",
            "protocolBinding": "GRPC",
            "protocolVersion": "1.0"
        }],
        "capabilities": {},
        "skills": []
    })
}
