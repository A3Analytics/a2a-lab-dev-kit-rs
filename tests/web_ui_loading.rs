//! Shared loading surface for the first agent-card fetch.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MemoryLogs, MemoryMetrics, MemoryTasks, TaskDefinition, TaskId,
    bind_local,
};
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const HOLD_CARD: &str = r"( () => {
  const orig = window.fetch.bind(window);
  window.fetch = (input, init) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    if (String(url).includes('/.well-known/agent-card.json')) return new Promise(() => {});
    return orig(input, init);
  };
})();";

#[tokio::test]
async fn browser_loading_surface_covers_the_first_card_fetch() {
    let base = lab().await;
    tokio::task::spawn_blocking(move || loading_flow(&base))
        .await
        .unwrap();
}

async fn lab() -> String {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn loading_flow(base: &str) {
    let mut page = Browser::open("about:blank");
    let script = page.prelude(HOLD_CARD);
    page.reduced_motion(false);
    page.goto(&format!("{base}/ui/"));
    page.wait_until(LOADING);
    assert_card_surface(&mut page);
    assert_progress_contract(&mut page);
    assert_skeletons(&mut page);
    assert_routes_held(&mut page);
    page.reduced_motion(true);
    page.wait_until(MOTION_STOPPED);
    page.clear_prelude(&script);
    page.reduced_motion(false);
    page.reload();
    page.wait_until(CONNECTED);
    assert_routes_restored(&mut page);
}

const LOADING: &str = "document.getElementById('connection')?.dataset.state === 'loading' && document.getElementById('card-loading') && !document.getElementById('loading-root')?.hasAttribute('hidden')";

const MOTION_STOPPED: &str = "getComputedStyle(document.querySelector('#card-loading .strand-wave')).animationName === 'none' && getComputedStyle(document.querySelector('#skeleton-rail .skeleton-block'), '::after').animationName === 'none'";

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected' && document.getElementById('loading-root')?.hasAttribute('hidden') && !document.getElementById('view-logs')?.hasAttribute('hidden')";

fn assert_card_surface(page: &mut Browser) {
    assert_eq!(attr(page, "card-loading", "role"), "status");
    assert_eq!(attr(page, "card-loading", "aria-live"), "polite");
    assert_eq!(attr(page, "card-loading", "aria-busy"), "true");
    assert_eq!(text(page, "card-loading-title"), "Loading agent card.");
    assert_eq!(
        attr(page, "card-loading", "aria-labelledby"),
        "card-loading-title"
    );
    assert_eq!(count(page, "#card-loading .strand-wave"), 5);
    assert_eq!(count(page, "#card-loading .strand-mark"), 0);
    assert!(!text(page, "card-loading").contains("% complete"));
    assert_eq!(
        animation(page, "#card-loading .strand-wave"),
        "strands-wave"
    );
    assert!(style_has(
        page,
        "#card-loading .strand-wave",
        "--a3-strand-1"
    ));
    assert!(style_has(
        page,
        "#card-loading .strand-wave:last-child",
        "0.48s"
    ));
}

fn assert_progress_contract(page: &mut Browser) {
    assert_measured_progress(page);
    assert_ready_progress(page);
    assert_unknown_progress(page);
}

fn assert_measured_progress(page: &mut Browser) {
    assert_eq!(attr(page, "loading-progress", "role"), "status");
    assert_eq!(attr(page, "loading-progress", "aria-busy"), "true");
    assert_eq!(text(page, "loading-progress-title"), "Uploading files");
    assert!(text(page, "loading-progress").contains("2 of 3 files uploaded"));
    assert!(text(page, "loading-progress").contains("66% complete"));
    assert_eq!(
        progress_attr(page, "loading-progress", "aria-label"),
        "Uploading files"
    );
    assert_eq!(
        progress_attr(page, "loading-progress", "aria-valuenow"),
        "66"
    );
}

fn assert_ready_progress(page: &mut Browser) {
    assert_eq!(attr(page, "loading-ready", "aria-busy"), "false");
    assert_eq!(text(page, "loading-ready-title"), "Ready");
    assert!(text(page, "loading-ready").contains("100% complete"));
    assert_eq!(count(page, "#loading-ready .strand-wave"), 0);
    assert_eq!(count(page, "#loading-ready .strand-mark"), 1);
    assert_eq!(progress_attr(page, "loading-ready", "aria-valuenow"), "100");
}

fn assert_unknown_progress(page: &mut Browser) {
    assert_eq!(attr(page, "loading-unknown", "aria-busy"), "true");
    assert_eq!(unknown_value(page), "");
    assert_eq!(unknown_state(page), "indeterminate");
    assert_strand_sizes(page);
}

fn progress_attr(page: &mut Browser, id: &str, name: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    let name = serde_json::to_string(name).unwrap_or_default();
    string_value(
        page,
        &format!(
            "document.querySelector('#' + {id} + ' [role=\"progressbar\"]')?.getAttribute({name}) ?? ''"
        ),
    )
}

fn assert_skeletons(page: &mut Browser) {
    assert_status(page, "skeleton-rail", "Loading navigation row");
    assert_status(page, "skeleton-list", "Loading list row");
    assert_status(page, "skeleton-chart", "Loading chart");
    assert_hidden_blocks(page, "#skeleton-rail");
    assert_hidden_blocks(page, "#skeleton-list");
    assert_hidden_blocks(page, "#skeleton-chart");
    assert_rail_matches_nav(page);
    assert_list_matches_inventory(page);
    assert_chart_matches_plot(page);
}

fn assert_routes_held(page: &mut Browser) {
    for id in [
        "view-logs",
        "view-metrics",
        "view-tasks",
        "view-images",
        "view-chat",
    ] {
        assert!(
            flag(
                page,
                &format!("document.getElementById('{id}')?.hasAttribute('hidden')")
            ),
            "{id}"
        );
    }
    assert_eq!(
        text(page, "inventory-logs-status"),
        "Waiting for the agent card."
    );
    assert!(flag(
        page,
        "document.getElementById('log-view-status')?.closest('#view-logs')"
    ));
    assert!(flag(
        page,
        "document.getElementById('metric-view-heading')?.closest('#view-metrics')"
    ));
    assert!(flag(
        page,
        "document.getElementById('chat-log')?.closest('#view-chat')"
    ));
    assert_eq!(
        count(
            page,
            "#view-logs .skeleton, #view-metrics .skeleton, #view-chat .skeleton"
        ),
        0
    );
}

fn assert_routes_restored(page: &mut Browser) {
    assert_eq!(text(page, "view-title"), "Logs");
    assert!(flag(
        page,
        "document.getElementById('loading-root')?.hasAttribute('hidden')"
    ));
    page.show("#logs");
    assert_kept(page, "view-logs", "inventory-logs-status");
    page.show("#metrics");
    assert_kept(page, "view-metrics", "metric-view-heading");
    page.show("#chat");
    assert_kept(page, "view-chat", "chat-log");
}

fn assert_kept(page: &mut Browser, view: &str, marker: &str) {
    assert!(flag(
        page,
        &format!("document.getElementById('{marker}')?.closest('#{view}')")
    ));
    assert_eq!(count(page, &format!("#{view} .skeleton")), 0);
    assert_eq!(count(page, &format!("#{view} .loading-surface")), 0);
}

fn assert_strand_sizes(page: &mut Browser) {
    let widths = page.eval(
        "(() => { const box = document.getElementById('strand-sizes'); box.hidden = false; const width = (selector) => Math.round(document.querySelector(selector).getBoundingClientRect().width); const result = [width('#strand-sizes .strand-xs'), width('#strand-sizes .strand-sm'), width('#card-loading .strand-md')]; box.hidden = true; return result; })()",
    );
    assert_eq!(widths, json!([16, 24, 96]));
}

fn unknown_value(page: &mut Browser) -> String {
    string_value(
        page,
        "document.querySelector('#loading-unknown [role=\"progressbar\"]')?.getAttribute('aria-valuenow') ?? ''",
    )
}

fn unknown_state(page: &mut Browser) -> String {
    string_value(
        page,
        "document.querySelector('#loading-unknown [role=\"progressbar\"]')?.getAttribute('data-state') ?? ''",
    )
}

fn assert_status(page: &mut Browser, id: &str, label: &str) {
    assert_eq!(attr(page, id, "role"), "status");
    assert_eq!(attr(page, id, "aria-live"), "polite");
    assert_eq!(attr(page, id, "aria-label"), label);
}

fn assert_hidden_blocks(page: &mut Browser, selector: &str) {
    let selector = serde_json::to_string(selector).unwrap_or_default();
    let hidden = page.eval(&format!(
        "[...document.querySelectorAll({selector} + ' .skeleton-block')].every(node => node.getAttribute('aria-hidden') === 'true') && document.querySelectorAll({selector} + ' .skeleton-block').length > 0"
    ));
    assert_eq!(hidden, Value::Bool(true), "{selector}");
}

fn assert_rail_matches_nav(page: &mut Browser) {
    let close = page.eval(
        "(() => { const nav = document.querySelector('#primary-nav a').getBoundingClientRect(); const row = document.getElementById('skeleton-rail').getBoundingClientRect(); const glyph = document.querySelector('#nav-logs .nav-glyph').getBoundingClientRect(); const bone = document.querySelector('#skeleton-rail .skeleton-glyph').getBoundingClientRect(); return Math.abs(nav.height - row.height) < 2 && Math.abs(glyph.width - bone.width) < 1 && Math.abs(glyph.height - bone.height) < 1; })()",
    );
    assert_eq!(close, Value::Bool(true), "{close}");
}

fn assert_list_matches_inventory(page: &mut Browser) {
    let matched = page.eval(
        "(() => { const row = getComputedStyle(document.getElementById('skeleton-list')); return row.paddingTop === '12px' && row.borderTopWidth === '1px' && row.backgroundColor !== 'rgba(0, 0, 0, 0)'; })()",
    );
    assert_eq!(matched, Value::Bool(true), "{matched}");
}

fn assert_chart_matches_plot(page: &mut Browser) {
    let matched = page.eval(
        "(() => { const plot = getComputedStyle(document.querySelector('.metric-plot')).minHeight; const bone = getComputedStyle(document.querySelector('#skeleton-chart .skeleton-plot')).minHeight; return plot === '256px' && bone === plot; })()",
    );
    assert_eq!(matched, Value::Bool(true), "{matched}");
}

fn attr(page: &mut Browser, id: &str, name: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    let name = serde_json::to_string(name).unwrap_or_default();
    string_value(
        page,
        &format!("document.getElementById({id})?.getAttribute({name}) ?? ''"),
    )
}

fn text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    string_value(
        page,
        &format!("document.getElementById({id})?.textContent ?? ''"),
    )
}

fn animation(page: &mut Browser, selector: &str) -> String {
    let selector = serde_json::to_string(selector).unwrap_or_default();
    string_value(
        page,
        &format!("getComputedStyle(document.querySelector({selector})).animationName"),
    )
}

fn style_has(page: &mut Browser, selector: &str, needle: &str) -> bool {
    let selector = serde_json::to_string(selector).unwrap_or_default();
    let needle = serde_json::to_string(needle).unwrap_or_default();
    flag(
        page,
        &format!(
            "(document.querySelector({selector})?.getAttribute('style') ?? '').includes({needle})"
        ),
    )
}

fn count(page: &mut Browser, selector: &str) -> i64 {
    let selector = serde_json::to_string(selector).unwrap_or_default();
    page.eval(&format!("document.querySelectorAll({selector}).length"))
        .as_i64()
        .unwrap_or(-1)
}

fn flag(page: &mut Browser, expression: &str) -> bool {
    page.eval(&format!("!!({expression})")) == Value::Bool(true)
}

fn string_value(page: &mut Browser, expression: &str) -> String {
    page.eval(expression)
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
