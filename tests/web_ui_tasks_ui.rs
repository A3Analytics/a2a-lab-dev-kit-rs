//! Tasks reader controls, validation, and list layout.

#![cfg(feature = "web-ui")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, GetTaskStatusRequest, MemoryLogs, MemoryMetrics, MemoryTasks, RunId,
    TaskDefinition, TaskId, TaskProvider, TaskState, bind_local,
};
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const ALPHA_READY: &str =
    "document.getElementById('task-pick-0')?.getAttribute('data-task') === 'alpha'";
const ALPHA_SELECTED: &str = "document.getElementById('task-detail-id')?.textContent === 'alpha'";
const BETA_SELECTED: &str = "document.getElementById('task-detail-id')?.textContent === 'beta'";
const LOADING_HELD: &str = "document.getElementById('task-view-status')?.textContent.includes('Loading') && typeof window.__releaseTasks === 'function'";
const EMPTY_STATUS: &str =
    "document.getElementById('task-view-status')?.textContent.includes('empty')";
const START_FAILED: &str =
    "document.getElementById('task-view-status')?.textContent.includes('could not be started')";
const POLL_FAILED: &str =
    "document.getElementById('task-view-status')?.textContent.includes('could not be read')";
const RUN_DONE: &str = "document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('completed') && document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('done')";

const HOLD_LIST: &str = r"( () => {
  const orig = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    let hold = false;
    if (String(url).includes('/message:send')) {
      try {
        const copy = input instanceof Request ? input.clone() : new Request(input, init);
        hold = (await copy.text()).includes('list_tasks');
      } catch (_) {}
    }
    if (hold) await new Promise((resolve) => { window.__releaseTasks = resolve; });
    return orig(input, init);
  };
})();";

const ALPHA_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["name"],
  "properties": {
    "name": {"type": "string", "title": "Name", "default": "lab", "minLength": 2, "pattern": "^lab$"},
    "count": {"type": "integer", "title": "Count", "default": 4, "minimum": 1, "maximum": 9},
    "note": {"type": "string", "title": "Note"},
    "ready": {"type": "boolean", "title": "Ready", "default": true},
    "mode": {"type": "string", "title": "Mode", "enum": ["fast", "slow"], "default": "fast"}
  }
}"#;

#[tokio::test]
async fn browser_tasks_reader_fills_controls_and_keeps_runs() {
    let (base, tasks) = populated_lab().await;
    let stop = Arc::new(AtomicBool::new(false));
    drive(tasks.clone(), Arc::clone(&stop));
    let address = base.clone();
    let handle = tokio::runtime::Handle::current();
    let result =
        tokio::task::spawn_blocking(move || populated_flow(&address, &tasks, &handle)).await;
    stop.store(true, Ordering::Relaxed);
    result.unwrap();
}

#[tokio::test]
async fn browser_tasks_reader_distinguishes_loading_and_empty() {
    let base = empty_lab().await;
    tokio::task::spawn_blocking(move || empty_flow(&base))
        .await
        .unwrap();
}

fn populated_flow(base: &str, tasks: &MemoryTasks, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#tasks");
    page.wait_until(ALPHA_READY);
    assert_task_search(&mut page);
    assert_catalog_hidden(&mut page);
    reselect_alpha(&mut page);
    assert_report(&mut page, DEFAULTS, "defaults");
    assert_report(&mut page, CONSTRAINTS, "constraints");
    assert_radio_keys(&mut page);
    assert_report(&mut page, DEFAULTS, "defaults after arrows");
    assert_layout(&mut page);
    finish_first_run(&mut page, base);
    assert_invalid_blocks(&mut page, base);
    assert_start_failure(&mut page, tasks, handle);
    assert_poll_failure(&mut page, tasks, handle);
}

fn empty_flow(base: &str) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(HOLD_LIST);
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#tasks");
    page.wait_until(LOADING_HELD);
    let loading = status(&mut page);
    page.eval("window.__releaseTasks()");
    page.wait_until(EMPTY_STATUS);
    let empty = status(&mut page);
    assert_ne!(loading, empty);
    assert!(loading.contains("Loading"), "{loading}");
    assert!(!loading.contains("empty"), "{loading}");
    assert!(empty.contains("empty"), "{empty}");
    assert!(!empty.contains("Loading"), "{empty}");
    assert_catalog_hidden(&mut page);
}

fn assert_task_search(page: &mut Browser) {
    set_input(page, "task-search", "HOME");
    page.wait_until(
        "document.getElementById('task-pick-0')?.getAttribute('data-task') === 'beta' && document.querySelectorAll('#task-sources [data-task]').length === 1",
    );
    set_input(page, "task-search", "no-such-task");
    page.wait_until("document.getElementById('task-search-empty')?.textContent.includes('No matching tasks.') && document.querySelectorAll('#task-sources [data-task]').length === 0");
    set_input(page, "task-search", "   ");
    page.wait_until(
        "document.getElementById('task-pick-0')?.getAttribute('data-task') === 'alpha' && document.getElementById('task-pick-1')?.getAttribute('data-task') === 'beta'",
    );
    set_input(page, "task-search", "");
}

fn reselect_alpha(page: &mut Browser) {
    page.click("task-pick-1");
    page.wait_until(BETA_SELECTED);
    page.click("task-pick-0");
    page.wait_until(ALPHA_SELECTED);
}

fn assert_radio_keys(page: &mut Browser) {
    assert_report(page, ONE_STOP, "radio tab stop");
    press_arrow(page, "ArrowDown");
    page.wait_until(BETA_SELECTED);
    assert_report(page, ONE_STOP, "radio tab stop after down");
    press_arrow(page, "ArrowUp");
    page.wait_until(ALPHA_SELECTED);
    assert_report(page, ONE_STOP, "radio tab stop after up");
    page.focus("task-pick-0");
    page.press_tab();
    assert_ne!(active_id(page), "task-pick-1");
}

fn assert_layout(page: &mut Browser) {
    page.viewport(1280, 800);
    assert_report(page, &layout_script("row"), "desktop rail");
    page.viewport(360, 800);
    assert_report(page, &layout_script("column"), "narrow stack");
    page.viewport(1280, 800);
}

fn finish_first_run(page: &mut Browser, base: &str) {
    let before = start_sends(page, base);
    page.click("task-start");
    page.wait_until("document.querySelector('[data-run=\"run-1\"]')");
    assert!(start_sends(page, base) > before);
    let body = last_start(page, base);
    assert!(
        body.contains("\"wait\":false") || body.contains("\"wait\": false"),
        "{body}"
    );
    assert!(body.contains("lab"), "{body}");
    page.wait_until(RUN_DONE);
    assert!(status_sends(page, base) >= 1);
}

fn assert_invalid_blocks(page: &mut Browser, base: &str) {
    let before = start_sends(page, base);
    set_input(page, "task-input-name", "no");
    page.click("task-start");
    page.wait_until("document.getElementById('task-input-name-error')");
    assert_eq!(start_sends(page, base), before);
    assert_eq!(
        text(page, "task-input-name-error"),
        "Must match the required pattern."
    );
    assert_eq!(
        page.eval("document.getElementById('task-input-name')?.getAttribute('aria-invalid')"),
        json!("true")
    );
    assert_report(page, DESTRUCTIVE, "destructive error");
}

fn assert_start_failure(page: &mut Browser, tasks: &MemoryTasks, handle: &tokio::runtime::Handle) {
    page.click("task-pick-1");
    page.wait_until(BETA_SELECTED);
    let before = run_count(page);
    handle.block_on(tasks.set_unavailable("task provider is not configured"));
    page.click("task-start");
    page.wait_until(START_FAILED);
    assert_eq!(run_count(page), before);
    let shown = status(page);
    assert!(shown.contains("could not be started"), "{shown}");
    assert!(!shown.contains("could not be read"), "{shown}");
    assert!(!shown.contains("empty"), "{shown}");
    handle.block_on(tasks.clear_unavailable());
}

fn assert_poll_failure(page: &mut Browser, tasks: &MemoryTasks, handle: &tokio::runtime::Handle) {
    let before = run_count(page);
    page.click("task-start");
    page.wait_until(&format!(
        "document.querySelectorAll('#task-runs [data-run]').length === {}",
        before + 1
    ));
    handle.block_on(tasks.set_unavailable("status is unavailable"));
    page.wait_until(POLL_FAILED);
    assert_eq!(run_count(page), before + 1);
    let shown = status(page);
    assert!(shown.contains("could not be read"), "{shown}");
    assert!(!shown.contains("could not be started"), "{shown}");
    assert!(!shown.contains("Loading"), "{shown}");
}

fn assert_catalog_hidden(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('inventory-tasks')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
}

fn assert_report(page: &mut Browser, expression: &str, case: &str) {
    assert_eq!(string_value(page, expression), "ok", "{case}");
}

fn press_arrow(page: &mut Browser, key: &str) {
    let key = serde_json::to_string(key).unwrap_or_default();
    let moved = page.eval(&format!(
        "(() => {{ const current = document.querySelector('#task-sources [aria-checked=\"true\"]'); if (!current) return false; current.focus(); current.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}})); return true; }})()"
    ));
    assert_eq!(moved, Value::Bool(true));
}

fn set_input(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('input', {{bubbles:true}}))"
    ));
}

fn layout_script(direction: &str) -> String {
    let direction = serde_json::to_string(direction).unwrap_or_default();
    format!(
        "(() => {{
            const rail = document.getElementById('task-filters')?.getBoundingClientRect();
            const form = document.getElementById('task-form')?.getBoundingClientRect();
            const details = document.getElementById('task-details');
            const axis = getComputedStyle(document.getElementById('task-browser')).flexDirection;
            if (!rail || !form || !details) return 'missing';
            if (details.open) return 'details open';
            if (form.top > details.getBoundingClientRect().top) return 'form after details';
            if (axis !== {direction}) return 'direction ' + axis;
            const rem = 15 * parseFloat(getComputedStyle(document.documentElement).fontSize);
            if ({direction} === 'row' && Math.abs(rail.width - rem) > 1) return 'width ' + rail.width;
            if ({direction} === 'row' && form.left + 1 < rail.right) return 'side';
            if ({direction} === 'column' && form.top + 1 < rail.bottom) return 'stack';
            return 'ok';
        }})()"
    )
}

fn status(page: &mut Browser) -> String {
    text(page, "task-view-status")
}

fn text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    string_value(
        page,
        &format!("document.getElementById({id})?.textContent ?? ''"),
    )
}

fn active_id(page: &mut Browser) -> String {
    string_value(page, "document.activeElement?.id ?? ''")
}

fn run_count(page: &mut Browser) -> u64 {
    match page.eval("document.querySelectorAll('#task-runs [data-run]').length") {
        Value::Number(value) => value.as_u64().unwrap_or(u64::MAX),
        _ => u64::MAX,
    }
}

fn string_value(page: &mut Browser, expression: &str) -> String {
    match page.eval(expression) {
        Value::String(value) => value,
        other => panic!("expected string from {expression}, observed {other}"),
    }
}

fn start_sends(page: &Browser, base: &str) -> usize {
    operation_sends(page, base, "start_task")
}

fn status_sends(page: &Browser, base: &str) -> usize {
    operation_sends(page, base, "get_task_status")
}

fn operation_sends(page: &Browser, base: &str, operation: &str) -> usize {
    page.requests(base)
        .iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .filter(|request| post_data(request).contains(operation))
        .count()
}

fn last_start(page: &Browser, base: &str) -> String {
    page.requests(base)
        .iter()
        .rev()
        .filter(|request| request_url(request).contains("/message:send"))
        .map(post_data)
        .find(|body| body.contains("start_task"))
        .unwrap_or_default()
}

fn post_data(request: &Value) -> String {
    request["params"]["request"]["postData"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn request_url(request: &Value) -> &str {
    request["params"]["request"]["url"].as_str().unwrap_or("")
}

fn drive(tasks: MemoryTasks, stop: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut since = HashMap::<String, Instant>::new();
        while !stop.load(Ordering::Relaxed) {
            advance(&tasks, "run-1", &mut since).await;
            advance(&tasks, "run-2", &mut since).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    });
}

async fn advance(tasks: &MemoryTasks, name: &str, since: &mut HashMap<String, Instant>) {
    let Ok(id) = RunId::new(name) else {
        return;
    };
    let Ok(run) = tasks.status(GetTaskStatusRequest { id: id.clone() }).await else {
        return;
    };
    match run.state {
        TaskState::Submitted => {
            let _ = tasks
                .transition(&id, TaskState::Working, Some("working".to_owned()))
                .await;
            since.entry(name.to_owned()).or_insert_with(Instant::now);
        }
        TaskState::Working => complete_when_due(tasks, &id, since.get(name)).await,
        _ => {}
    }
}

async fn complete_when_due(tasks: &MemoryTasks, id: &RunId, started: Option<&Instant>) {
    let due = started.is_some_and(|at| at.elapsed() >= Duration::from_millis(300));
    if !due {
        return;
    }
    let _ = tasks
        .transition(id, TaskState::Completed, Some("done".to_owned()))
        .await;
}

async fn populated_lab() -> (String, MemoryTasks) {
    let tasks = MemoryTasks::new();
    tasks.insert(alpha_definition()).await;
    tasks.insert(beta_definition()).await;
    let address = serve(tasks.clone()).await;
    (address, tasks)
}

async fn empty_lab() -> String {
    serve(MemoryTasks::new()).await
}

async fn serve(tasks: MemoryTasks) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn alpha_definition() -> TaskDefinition {
    TaskDefinition {
        id: TaskId::new("alpha").unwrap(),
        name: "Mix".to_owned(),
        description: "Mix with defaults".to_owned(),
        asset_id: None,
        semantic_id: None,
        input_schema: Some(ALPHA_SCHEMA.to_owned()),
        output_schema: None,
    }
}

fn beta_definition() -> TaskDefinition {
    TaskDefinition {
        id: TaskId::new("beta").unwrap(),
        name: "Home".to_owned(),
        description: "Home the axes".to_owned(),
        asset_id: None,
        semantic_id: None,
        input_schema: None,
        output_schema: None,
    }
}

const DEFAULTS: &str = r"(() => {
  const value = (id) => document.getElementById(id)?.value ?? '';
  if (value('task-input-name') !== 'lab') return 'name ' + value('task-input-name');
  if (value('task-input-count') !== '4') return 'count ' + value('task-input-count');
  if (!document.getElementById('task-input-ready')?.checked) return 'ready';
  if (value('task-input-mode') !== 'fast') return 'mode ' + value('task-input-mode');
  if (value('task-input-note') !== '') return 'note';
  return 'ok';
})()";

const CONSTRAINTS: &str = r"(() => {
  const has = (id, name) => !!document.getElementById(id)?.hasAttribute(name);
  const attr = (id, name) => document.getElementById(id)?.getAttribute(name);
  for (const name of ['min', 'max', 'minlength', 'maxlength', 'pattern']) {
    if (has('task-input-note', name)) return 'note ' + name;
  }
  if (attr('task-input-name', 'minlength') !== '2') return 'name minlength';
  if (attr('task-input-name', 'pattern') !== '^lab$') return 'name pattern';
  if (has('task-input-name', 'min') || has('task-input-name', 'max')) return 'name range';
  if (attr('task-input-count', 'min') !== '1') return 'count min';
  if (attr('task-input-count', 'max') !== '9') return 'count max';
  if (has('task-input-count', 'minlength') || has('task-input-count', 'maxlength') || has('task-input-count', 'pattern')) return 'count text';
  return 'ok';
})()";

const ONE_STOP: &str = r#"(() => {
  const radios = [...document.querySelectorAll('#task-sources [role="radio"]')];
  const stops = radios.filter((el) => el.tabIndex >= 0);
  if (radios.length < 2) return 'radios ' + radios.length;
  if (stops.length !== 1) return 'stops ' + stops.length;
  if (stops[0].getAttribute('aria-checked') !== 'true') return 'checked';
  return 'ok';
})()"#;

const DESTRUCTIVE: &str = r"(() => {
  const error = document.querySelector('.task-field-error');
  if (!error) return 'missing';
  const probe = document.createElement('span');
  probe.style.color = 'var(--destructive)';
  document.body.appendChild(probe);
  const expected = getComputedStyle(probe).color;
  const foreground = document.createElement('span');
  foreground.style.color = 'var(--foreground)';
  document.body.appendChild(foreground);
  const plain = getComputedStyle(foreground).color;
  probe.remove();
  foreground.remove();
  if (!expected || expected === plain) return 'token';
  if (getComputedStyle(error).color !== expected) return getComputedStyle(error).color;
  return 'ok';
})()";
