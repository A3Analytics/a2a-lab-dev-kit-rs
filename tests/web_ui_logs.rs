//! Logs list page: filter rail, result cards, and collection states.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, JsonObject, LogLevel, LogRecord, LogSource, MemoryLogs,
    MemoryMetrics, MemoryTasks, SourceId, UtcTimestamp, bind_local,
};
use serde_json::Value;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const LONG: &str =
    "line overflow: the laboratory message is longer than the compact card and must expand";

#[tokio::test]
async fn browser_logs_list_page_follows_the_filters() {
    let logs = populated().await;
    let populated = serve(logs.clone()).await;
    let empty = serve(empty_logs().await).await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || flow(&populated, &empty, &logs, &handle))
        .await
        .unwrap();
}

async fn populated() -> MemoryLogs {
    let logs = MemoryLogs::new();
    insert_source(&logs, "log-a", "App").await;
    insert_source(&logs, "log-b", "Kernel").await;
    insert_record(
        &logs,
        "log-a",
        "2024-01-01T00:00:00Z",
        LogLevel::Info,
        "pump ready",
    )
    .await;
    insert_record(
        &logs,
        "log-a",
        "2024-01-01T00:01:00Z",
        LogLevel::Error,
        LONG,
    )
    .await;
    insert_record(
        &logs,
        "log-b",
        "2024-01-01T00:02:00Z",
        LogLevel::Warn,
        "kernel boot",
    )
    .await;
    logs
}

async fn empty_logs() -> MemoryLogs {
    let logs = MemoryLogs::new();
    insert_source(&logs, "log-a", "App").await;
    logs
}

async fn insert_source(logs: &MemoryLogs, id: &str, name: &str) {
    logs.insert_source(LogSource {
        id: SourceId::new(id).unwrap(),
        name: name.to_owned(),
        description: format!("{name} log"),
        asset_id: None,
        semantic_id: None,
    })
    .await;
}

async fn insert_record(logs: &MemoryLogs, id: &str, at: &str, level: LogLevel, message: &str) {
    logs.insert_record(LogRecord {
        source_id: SourceId::new(id).unwrap(),
        timestamp: UtcTimestamp::parse(at).unwrap(),
        level,
        message: message.to_owned(),
        attributes: JsonObject::empty(),
    })
    .await
    .unwrap();
}

async fn serve(logs: MemoryLogs) -> String {
    let service = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn flow(populated: &str, empty: &str, logs: &MemoryLogs, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{populated}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.viewport(1280, 800);
    page.show("#logs");
    assert_default_last_day(&mut page);
    show_every_log(&mut page);
    page.wait_until(READY);
    assert_rail(&mut page);
    assert_cards(&mut page);
    assert_page_chrome(&mut page);
    assert_search_and_clear(&mut page);
    assert_message_fold(&mut page);
    assert_dates_follow_input(&mut page);
    page.viewport(767, 900);
    assert_stacked(&mut page);
    page.viewport(1280, 800);
    handle.block_on(logs.set_unavailable("logs offline"));
    page.click("log-refresh");
    page.wait_until(ERROR);
    assert_eq!(
        text(&mut page, "log-view-status"),
        "Logs could not be read.",
    );
    assert!(text(&mut page, "log-view-alert").contains("logs offline"));
    assert_eq!(pressed(&mut page, "log-source-all"), "true");
    page.goto(&format!("{empty}/ui/#logs"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    show_every_log(&mut page);
    page.wait_until(EMPTY);
    assert_eq!(
        text(&mut page, "log-view-status"),
        "No logs for this source.",
    );
    assert_eq!(pressed(&mut page, "log-source-all"), "true");
    assert!(!text(&mut page, "log-view-status").contains("Choose a log source"));
    assert_eq!(
        page.eval("document.getElementById('log-result-clear')"),
        Value::Null,
    );
}

const READY: &str = "document.querySelectorAll('#log-view-entries .log-card').length === 3 && document.getElementById('log-result-state')?.dataset.state === 'ready'";
const ERROR: &str = "document.getElementById('log-result-state')?.dataset.state === 'error'";
const EMPTY: &str = "document.getElementById('log-result-state')?.dataset.state === 'empty' && document.getElementById('log-result-state')?.dataset.gen !== '0'";

fn assert_default_last_day(page: &mut Browser) {
    page.wait_until(
        "document.getElementById('log-range-start')?.value && document.getElementById('log-range-end')?.value",
    );
    let range = page.eval(
        "(() => { const start = document.getElementById('log-range-start').value; const end = document.getElementById('log-range-end').value; return { span: new Date(end + 'T00:00:00Z') - new Date(start + 'T00:00:00Z'), end, today: new Date().toISOString().slice(0, 10) }; })()",
    );
    assert_eq!(range["span"], serde_json::json!(86_400_000), "{range}");
    assert_eq!(range["end"], range["today"], "{range}");
}

fn show_every_log(page: &mut Browser) {
    page.wait_until("document.querySelector('#log-filter-chips [data-clear=\"range\"]')");
    page.eval("document.querySelector('#log-filter-chips [data-clear=\"range\"]').click()");
}

fn assert_rail(page: &mut Browser) {
    let rail = page.eval(RAIL);
    assert_eq!(rail, Value::Bool(true), "filter rail");
}

const RAIL: &str = r"( () => {
  const filters = document.getElementById('log-filters');
  const actions = document.querySelector('#log-filters .log-actions');
  const primary = document.getElementById('log-refresh');
  const outline = document.getElementById('log-clear');
  const search = document.getElementById('log-filter-search');
  const mark = document.querySelector('.log-facet-mark');
  const count = document.getElementById('log-filter-count');
  if (!filters || !actions || !primary || !outline || !search || !mark || !count) return false;
  const pad = parseFloat(getComputedStyle(actions).paddingLeft) + parseFloat(getComputedStyle(actions).paddingRight);
  const box = actions.getBoundingClientRect().width - pad;
  const width = (el) => el.getBoundingClientRect().width;
  const size = (el) => ({ w: parseFloat(getComputedStyle(el).width), h: parseFloat(getComputedStyle(el).height) });
  const toggle = size(mark);
  return Math.abs(width(filters) - 240) < 1
    && Math.abs(width(primary) - box) < 1
    && Math.abs(width(outline) - box) < 1
    && getComputedStyle(primary).backgroundColor !== getComputedStyle(outline).backgroundColor
    && Math.abs(size(search).h - 32) < 1
    && Math.abs(toggle.w - 20) < 1
    && Math.abs(toggle.h - 12) < 1
    && getComputedStyle(count).fontVariantNumeric.includes('tabular-nums');
})()";

fn assert_cards(page: &mut Browser) {
    let cards = page.eval(CARDS);
    assert_eq!(cards, Value::Bool(true), "result cards: {cards}");
    assert!(!body(page).contains("Choose a log source"));
    assert_eq!(pressed(page, "log-source-all"), "true");
}

const CARDS: &str = r"( () => {
  const id = document.querySelector('.log-card-id');
  const chips = [...document.querySelectorAll('.log-card .log-meta')];
  if (!id) return 'missing id';
  if (chips.length === 0) return 'missing chips';
  const font = getComputedStyle(id).fontFamily.toLowerCase();
  if (!font.includes('mono')) return font;
  if (!chips.some((chip) => chip.textContent.includes('Severity'))) return 'missing severity';
  return true;
})()";

fn assert_page_chrome(page: &mut Browser) {
    assert_eq!(text(page, "view-title"), "Logs");
    assert_eq!(page.eval(CHROME), Value::Bool(true), "logs chrome");
}

const CHROME: &str = r"( () => {
  const off = (id) => {
    const el = document.getElementById(id);
    if (!el || el.closest('#log-view')) return !el;
    let node = el;
    while (node) {
      if (node.hasAttribute && node.hasAttribute('hidden')) return true;
      node = node.parentElement;
    }
    return getComputedStyle(el).display === 'none';
  };
  return !document.querySelector('#log-view h1, #log-view h2, #log-view h3')
    && off('inventory-logs')
    && off('workbench');
})()";

fn assert_search_and_clear(page: &mut Browser) {
    type_input(page, "log-filter-search", "pump");
    page.wait_until("document.getElementById('log-filter-count').textContent === '1 of 3 logs'");
    type_input(page, "log-filter-search", "no-such-log");
    page.wait_until("document.getElementById('log-result-state')?.dataset.state === 'nomatch'");
    assert_eq!(
        text(page, "log-view-status"),
        "No logs match these filters.",
    );
    page.click("log-result-clear");
    page.wait_until(READY);
    assert_eq!(value(page, "log-filter-search"), "");
}

fn assert_message_fold(page: &mut Browser) {
    assert_eq!(
        page.eval("document.querySelectorAll('.log-expand').length"),
        Value::from(1)
    );
    page.click_selector(".log-expand");
    page.wait_until("document.querySelector('.log-expand')?.getAttribute('aria-expanded') === 'true' && document.querySelector('.log-card-title.is-open')");
    page.click_selector(".log-expand");
    page.wait_until("document.querySelector('.log-expand')?.getAttribute('aria-expanded') === 'false' && !document.querySelector('.log-card-title.is-open')");
}

fn assert_dates_follow_input(page: &mut Browser) {
    page.set_value("log-range-start", "2030-01-01");
    page.set_value("log-range-end", "2030-01-02");
    dispatch_input(page, "log-range-start");
    dispatch_input(page, "log-range-end");
    page.wait_until("document.getElementById('log-result-state')?.dataset.state === 'nomatch'");
    assert_eq!(value(page, "log-range-start"), "2030-01-01");
    assert_eq!(value(page, "log-range-end"), "2030-01-02");
    page.click("log-clear");
    page.wait_until(READY);
}

fn assert_stacked(page: &mut Browser) {
    assert_eq!(page.eval(STACK), Value::Bool(true), "stacked rail");
}

const STACK: &str = r"( () => {
  const browser = document.getElementById('log-browser');
  const filters = document.getElementById('log-filters')?.getBoundingClientRect();
  const results = document.getElementById('log-results')?.getBoundingClientRect();
  if (!browser || !filters || !results) return false;
  return getComputedStyle(browser).flexDirection === 'column'
    && filters.height > 0
    && results.height > 0
    && filters.bottom <= results.top + 1;
})()";

fn type_input(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    dispatch_input(page, id);
}

fn dispatch_input(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('input', {{bubbles:true}}))"
    ));
}

fn text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.textContent.trim() ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn value(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!("document.getElementById({id})?.value ?? ''"))
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn pressed(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.getAttribute('aria-pressed') ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn body(page: &mut Browser) -> String {
    page.eval("document.body.innerText")
        .as_str()
        .unwrap_or("")
        .to_owned()
}
