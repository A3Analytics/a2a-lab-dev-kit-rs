#![cfg(feature = "web-ui")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
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
#[path = "harness/logs.rs"]
mod logs_browser;
#[path = "harness/tasks.rs"]
mod tasks_browser;
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
async fn browser_routes_scroll_in_main_without_the_bootstrap_line() {
    let base = lab(false).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || route_scroll_flow(&address))
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
    page.wait_until(
        "document.getElementById('connection')?.dataset.state === 'loading' || document.getElementById('connection')?.dataset.state === 'offline'",
    );
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

fn route_scroll_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('shell')");
    page.viewport(1843, 768);
    assert_bootstrap(&mut page);
    for hash in ["#agent", "#logs", "#metrics", "#tasks", "#images"] {
        page.show(hash);
        assert_route_scroll(&mut page, hash);
    }
    page.show("#chat");
    assert_chat_scroll(&mut page);
}

fn assert_bootstrap(page: &mut Browser) {
    let report = page.eval(BOOTSTRAP);
    assert_eq!(report["children"], json!(["shell"]), "{report}");
    assert_eq!(report["leftover"], Value::Bool(false), "{report}");
    assert!(
        report["logo"].as_str().is_some_and(|logo| !logo.is_empty()),
        "{report}"
    );
}

fn assert_route_scroll(page: &mut Browser, hash: &str) {
    let report = page.eval(ROUTE_SCROLL);
    assert_eq!(report["views"], json!(1), "{hash} {report}");
    assert!(
        report["shellY"].as_f64().unwrap_or(1.0).abs() < 1.0,
        "{hash} {report}"
    );
    assert!(
        report["padding"].as_f64().unwrap_or(0.0) > 0.0,
        "{hash} {report}"
    );
    assert_eq!(report["overflow"], json!("auto"), "{hash} {report}");
    assert_eq!(report["bodyOverflow"], json!("hidden"), "{hash} {report}");
    assert_eq!(report["docTop"], json!(0), "{hash} {report}");
    assert_eq!(report["docMoved"], Value::Bool(false), "{hash} {report}");
    assert_eq!(report["inside"], Value::Bool(true), "{hash} {report}");
    assert!(
        report["docHeight"].as_f64().unwrap_or(f64::MAX)
            <= report["innerHeight"].as_f64().unwrap_or(0.0) + 1.0,
        "{hash} {report}"
    );
}

fn assert_chat_scroll(page: &mut Browser) {
    let report = page.eval(CHAT_SCROLL);
    assert_eq!(report["views"], json!(1), "{report}");
    assert!(
        report["shellY"].as_f64().unwrap_or(1.0).abs() < 1.0,
        "{report}"
    );
    assert_eq!(report["padding"], json!(0), "{report}");
    assert_eq!(report["mainOverflow"], json!("hidden"), "{report}");
    assert_eq!(report["logOverflow"], json!("auto"), "{report}");
    assert_eq!(report["chatOverflow"], json!("hidden"), "{report}");
    assert_eq!(report["bodyOverflow"], json!("hidden"), "{report}");
    assert_eq!(report["logScrolled"], Value::Bool(true), "{report}");
    assert_eq!(report["docTop"], json!(0), "{report}");
    assert_eq!(report["mainTop"], json!(0), "{report}");
    assert_eq!(report["docMoved"], Value::Bool(false), "{report}");
    assert_eq!(report["mainMoved"], Value::Bool(false), "{report}");
    assert!(
        report["docHeight"].as_f64().unwrap_or(f64::MAX)
            <= report["innerHeight"].as_f64().unwrap_or(0.0) + 1.0,
        "{report}"
    );
}

const BOOTSTRAP: &str = r"( () => {
  const app = document.getElementById('app');
  const logo = app?.getAttribute('data-logo') ?? '';
  const leftover = [...(app?.childNodes ?? [])].some(node => (node.textContent || '').includes('Loading the lab interface.'));
  return { children: [...(app?.children ?? [])].map(node => node.id), leftover, logo };
})()";

const ROUTE_SCROLL: &str = r"( () => {
  const shell = document.getElementById('shell');
  const main = document.getElementById('main');
  const doc = document.scrollingElement;
  main.scrollTop = 0;
  const mainStyle = getComputedStyle(main);
  const visible = [...main.querySelectorAll('button, a, input, textarea, select, summary')].filter(el => {
    if (el.closest('[hidden]')) return false;
    const closed = el.closest('details:not([open])');
    if (closed && el.tagName !== 'SUMMARY') return false;
    const box = el.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  });
  const target = visible.at(-1);
  const beforeDoc = doc.scrollTop;
  const before = target?.getBoundingClientRect();
  const mainBox = main.getBoundingClientRect();
  const needed = !!before && before.bottom > mainBox.bottom + 1;
  if (needed) main.scrollTop += before.bottom - mainBox.bottom;
  const box = target?.getBoundingClientRect();
  const after = main.getBoundingClientRect();
  const inside = !!box && box.top >= after.top - 1 && box.bottom <= after.bottom + 1;
  return {
    shellY: shell.getBoundingClientRect().y,
    padding: parseFloat(mainStyle.paddingTop),
    overflow: mainStyle.overflowY,
    bodyOverflow: getComputedStyle(document.body).overflowY,
    docTop: doc.scrollTop,
    docMoved: doc.scrollTop !== beforeDoc,
    inside,
    needed,
    views: document.querySelectorAll('#main > .view:not([hidden])').length,
    target: target?.id ?? '',
    top: box?.top ?? null,
    bottom: box?.bottom ?? null,
    mainTop: after.top,
    mainBottom: after.bottom,
    mainScroll: main.scrollTop,
    docHeight: doc.scrollHeight,
    innerHeight: window.innerHeight
  };
})()";

const CHAT_SCROLL: &str = r"( () => {
  const main = document.getElementById('main');
  const log = document.getElementById('chat-log');
  const chat = document.getElementById('chat');
  const doc = document.scrollingElement;
  const saved = log.textContent;
  log.textContent = Array.from({ length: 80 }, (_, index) => 'line ' + index).join('\n');
  const beforeDoc = doc.scrollTop;
  const beforeMain = main.scrollTop;
  log.scrollTop = log.scrollHeight;
  const report = {
    shellY: document.getElementById('shell').getBoundingClientRect().y,
    padding: parseFloat(getComputedStyle(main).paddingTop),
    mainOverflow: getComputedStyle(main).overflowY,
    logOverflow: getComputedStyle(log).overflowY,
    chatOverflow: getComputedStyle(chat).overflowY,
    bodyOverflow: getComputedStyle(document.body).overflowY,
    logScrolled: log.scrollTop > 0,
    docTop: doc.scrollTop,
    mainTop: main.scrollTop,
    docMoved: doc.scrollTop !== beforeDoc,
    mainMoved: main.scrollTop !== beforeMain,
    views: document.querySelectorAll('#main > .view:not([hidden])').length,
    docHeight: doc.scrollHeight,
    innerHeight: window.innerHeight
  };
  log.textContent = saved;
  return report;
})()";

fn navigation_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    assert_eq!(
        page.eval("document.querySelectorAll('#main > .view:not([hidden])').length"),
        json!(1)
    );
    assert_eq!(
        page.eval("document.getElementById('view-agent')?.parentElement?.id"),
        json!("main")
    );
    assert_eq!(
        page.eval("document.getElementById('view-chat')?.hasAttribute('hidden')"),
        Value::Bool(true)
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
    assert_eq!(page.eval("document.activeElement?.id"), json!("view-title"));
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
        page.eval("document.getElementById('view-logs')?.hasAttribute('hidden')"),
        Value::Bool(true)
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
    page.click("sidebar-toggle");
    page.wait_until(
        "document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'true'",
    );
    page.eval(
        "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
    );
    page.wait_until("document.getElementById('primary-nav')?.hasAttribute('hidden')");
    page.click("sidebar-toggle");
    page.click("nav-logs");
    page.wait_until(
        "document.getElementById('view-logs')?.parentElement?.id === 'main' && document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'false'",
    );
    page.viewport(1280, 800);
    assert_eq!(
        page.eval("document.getElementById('primary-nav')?.hasAttribute('hidden')"),
        Value::Bool(false)
    );
    assert_eq!(
        page.eval("document.getElementById('nav-toggle')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('sidebar-toggle')?.hasAttribute('hidden')"),
        Value::Bool(false)
    );
    assert_shell_scripts(&mut page);
}

fn assert_shell_scripts(page: &mut Browser) {
    assert_eq!(page.eval("document.scripts.length"), json!(1));
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
        "document.getElementById('metric-range-start')?.value && document.getElementById('metric-range-end')?.value",
    );
    page.eval("document.querySelector('#metric-filter-chips [data-clear=\"range\"]')?.click()");
    page.wait_until("document.querySelectorAll('#metric-chart .dx-line-point').length >= 2");
    assert_initial_series(&mut page);
    assert_point_tooltip(&mut page);
    assert_metric_switch(&mut page);
    assert_chart_resizes(&mut page);
    assert_time_range(&mut page);
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
    assert_eq!(initial["svg"], Value::Bool(true));
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
            const point = document.querySelectorAll('#metric-chart .dx-line-point')[1];
            const rect = point.getBoundingClientRect();
            return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        })()",
    );
    page.move_mouse(
        point["x"].as_f64().unwrap_or(0.0),
        point["y"].as_f64().unwrap_or(0.0),
    );
    page.eval(
        "(() => {
            const point = document.querySelectorAll('#metric-chart .dx-line-point')[1];
            if (!point) return false;
            point.dispatchEvent(new PointerEvent('pointerenter', { bubbles: false }));
            point.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
            return true;
        })()",
    );
    page.wait_until(
        "(() => {
            const label = document.querySelector('#metric-chart .dx-line-point-label');
            return label && label.textContent.includes('2024-01-01 00:01 UTC');
        })()",
    );
    let tip = page.eval(
        "(() => {
            const label = document.querySelector('#metric-chart .dx-line-point-label');
            return { text: label ? label.textContent : '' };
        })()",
    );
    let text = tip["text"].as_str().unwrap_or_default();
    assert!(text.contains("2024-01-01 00:01 UTC"), "{tip}");
    assert!(text.contains('1'), "{tip}");
    assert!(text.contains("open"), "{tip}");
}

fn assert_metric_switch(page: &mut Browser) {
    page.click("metric-pick-1");
    page.wait_until(
        "(() => {
            const values = [...document.querySelectorAll('#metric-chart .dx-line-point')].map(
                (point) => Number(point.getAttribute('data-value'))
            );
            return values.includes(12);
        })()",
    );
    let switched = page.eval(CHART_STATE);
    assert_eq!(switched["values"], json!([5, 12]));
    assert_eq!(switched["label"], "2 samples in ms");
}

fn assert_chart_resizes(page: &mut Browser) {
    page.viewport(1280, 900);
    let wide = page
        .eval("document.querySelector('#metric-chart svg')?.getBoundingClientRect().width || 0");
    page.viewport(520, 900);
    let narrow = page
        .eval("document.querySelector('#metric-chart svg')?.getBoundingClientRect().width || 0");
    let wide = wide.as_f64().unwrap_or(0.0);
    let narrow = narrow.as_f64().unwrap_or(0.0);
    assert!(wide > narrow + 40.0, "{wide} vs {narrow}");
}

fn assert_time_range(page: &mut Browser) {
    page.eval(
        "(() => {
            const start = document.getElementById('metric-range-start');
            const end = document.getElementById('metric-range-end');
            start.value = '2024-01-01T00:01:00';
            end.value = '2024-01-01T00:02:00';
            end.dispatchEvent(new Event('change', { bubbles: true }));
            return true;
        })()",
    );
    page.wait_until(
        "(() => {
            const values = [...document.querySelectorAll('#metric-chart .dx-line-point')].map(
                (point) => Number(point.getAttribute('data-value'))
            );
            return values.length === 1 && values[0] === 12;
        })()",
    );
    let filtered = page.eval(CHART_STATE);
    assert_eq!(filtered["values"], json!([12]));
    assert_eq!(filtered["ariaHidden"], Value::Null);
    let readings = filtered["readings"].as_str().unwrap_or_default();
    assert!(readings.contains("12 ms"), "{filtered}");
    assert_eq!(
        page.eval("document.getElementById('metric-filter-search')"),
        Value::Null
    );
}

const CHART_STATE: &str = "(() => {
    const host = document.getElementById('metric-chart');
    const values = [...document.querySelectorAll('#metric-chart .dx-line-point')].map(
        (point) => Number(point.getAttribute('data-value'))
    );
    const list = document.getElementById('metric-view-entries');
    return {
        values,
        label: host?.getAttribute('aria-label'),
        readings: list?.textContent ?? '',
        ariaHidden: list?.getAttribute('aria-hidden'),
        hidden: document.getElementById('metric-plot').hasAttribute('hidden'),
        svg: !!document.querySelector('#metric-chart svg'),
        width: host?.getBoundingClientRect().width ?? 0
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
    Tasks,
    TaskRuns,
    Logs,
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
    run_starts: Arc<AtomicU32>,
    run_polls: Arc<Mutex<BTreeMap<String, u32>>>,
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
        run_starts: Arc::new(AtomicU32::new(0)),
        run_polls: Arc::new(Mutex::new(BTreeMap::new())),
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
    if name.is_empty() || name.contains("..") {
        return bytes(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        );
    }
    if !name.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || character == '.'
            || character == '/'
    }) {
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
        MockRoute::Tasks => return tasks_browser::scripted_send(body).await,
        MockRoute::TaskRuns => {
            return tasks_browser::scripted_run_send(&body, &state.run_starts, &state.run_polls);
        }
        MockRoute::Logs => return logs_browser::scripted_send(&body).await,
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
