//! Browser coverage for the task-definition reader.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, GetTaskStatusRequest, HttpAuthSecurityScheme, MemoryLogs,
    MemoryMetrics, MemoryTasks, RunId, SecurityScheme, TaskDefinition, TaskId, TaskProvider,
    TaskState, bind_local,
};
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{Browser, MARKUP, MockRoute, TOKEN, bytes, request_url, serve_mock};

#[tokio::test]
async fn browser_tasks_reader_selects_definitions_without_starting_runs() {
    let base = task_lab(true, true).await;
    tokio::task::spawn_blocking(move || tasks_reader_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_tasks_reader_states_match_logs_and_metrics() {
    let states = tasks_state_lab().await;
    let empty = task_lab(false, false).await;
    tokio::task::spawn_blocking(move || tasks_state_flow(&states, &empty))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_tasks_reader_starts_and_monitors_runs() {
    let (base, tasks, stop) = run_lab().await;
    drive_runs(tasks.clone(), stop.clone());
    let address = base.clone();
    let handle = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || runs_flow(&address, &tasks, &handle)).await;
    stop.store(true, Ordering::Relaxed);
    result.unwrap();
}

#[tokio::test]
async fn browser_tasks_reader_reports_run_progress_and_start_faults() {
    let base = run_script_lab().await;
    tokio::task::spawn_blocking(move || run_script_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_tasks_reader_falls_back_for_unsupported_schema() {
    let (base, tasks, stop) = fallback_lab().await;
    drive_runs(tasks, stop.clone());
    let result = tokio::task::spawn_blocking(move || fallback_flow(&base)).await;
    stop.store(true, Ordering::Relaxed);
    result.unwrap();
}

fn tasks_reader_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    authorize_catalog(&mut page);
    assert_first_definition(&mut page);
    assert_selection_keyboard_and_refresh(&mut page, base);
    assert_lifecycle_and_layout(&mut page, base);
}

fn authorize_catalog(page: &mut Browser) {
    page.show("#tasks");
    page.wait_until(
        "document.getElementById('task-view-status')?.textContent.includes('not authorized')",
    );
    page.show("#overview");
    page.set_value("bearer-token", TOKEN);
    page.show("#tasks");
    commit(page, "inventory-tasks-limit", "100");
    page.wait_until(
        "document.getElementById('task-pick-0')?.getAttribute('data-task') === 'build'",
    );
}

fn assert_first_definition(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('task-pick-0')?.getAttribute('aria-checked')"),
        json!("true")
    );
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('task-detail-id').textContent"
        ),
        "build"
    );
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('task-detail-name').textContent"
        ),
        MARKUP
    );
    let detail = eval_str(page, "document.getElementById('task-detail').textContent");
    assert!(detail.contains("<b>in</b>"));
    assert!(detail.contains("<b>out</b>"));
    assert!(detail.contains("asset-task"));
    assert!(detail.contains("semantic-task"));
    assert!(!detail.contains(TOKEN));
    assert_inert_and_redacted(page);
}

fn assert_inert_and_redacted(page: &mut Browser) {
    assert_eq!(
        page.eval("document.querySelector('#task-view i')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.querySelector('#task-view b')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('xss-payload')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__xss"), Value::Null);
    assert_eq!(
        page.eval("document.getElementById('task-input-name')?.tagName"),
        json!("INPUT")
    );
    let help = eval_str(
        page,
        "document.getElementById('task-input-name-help')?.textContent ?? ''",
    );
    assert!(help.contains("[redacted]"));
    assert!(!help.contains(TOKEN));
}

fn assert_selection_keyboard_and_refresh(page: &mut Browser, base: &str) {
    page.click("task-pick-1");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'home'");
    assert_eq!(
        page.eval("document.getElementById('task-pick-1')?.getAttribute('aria-checked')"),
        json!("true")
    );
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('task-form-empty').textContent"
        ),
        "This task has no input fields."
    );
    page.eval(
        "document.getElementById('task-sources').dispatchEvent(new KeyboardEvent('keydown', {key: 'ArrowUp', bubbles: true}))",
    );
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'build'");
    page.click("task-pick-1");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'home'");
    let before = list_task_sends(page, base);
    commit(page, "inventory-tasks-limit", "50");
    page.wait_until(
        "document.getElementById('task-pick-1')?.getAttribute('data-task') === 'home' && document.getElementById('task-pick-1')?.getAttribute('aria-checked') === 'true'",
    );
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('task-detail-id').textContent"
        ),
        "home"
    );
    assert!(list_task_sends(page, base) > before);
}

fn assert_lifecycle_and_layout(page: &mut Browser, base: &str) {
    let after_refresh = list_task_sends(page, base);
    page.show("#logs");
    assert_eq!(
        page.eval("document.getElementById('view-tasks')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    page.show("#tasks");
    page.wait_until(
        "document.getElementById('task-pick-1')?.getAttribute('aria-checked') === 'true'",
    );
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('task-detail-id').textContent"
        ),
        "home"
    );
    assert_eq!(list_task_sends(page, base), after_refresh);
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('inventory-tasks')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('task-view-status')?.getAttribute('aria-live')"),
        json!("polite")
    );
    assert_eq!(
        page.eval("document.getElementById('task-view-alert')?.getAttribute('role')"),
        json!("alert")
    );
    page.viewport(1280, 800);
    assert_eq!(
        page.eval("getComputedStyle(document.getElementById('task-browser')).flexDirection"),
        json!("row")
    );
    page.viewport(360, 800);
    let narrow = page.snapshot();
    assert!(
        narrow["scroll"].as_u64().unwrap_or(u64::MAX) <= narrow["width"].as_u64().unwrap_or(0) + 24
    );
    assert_eq!(
        page.eval("getComputedStyle(document.getElementById('task-browser')).flexDirection"),
        json!("column")
    );
    page.viewport(1280, 800);
    let wide = page.snapshot();
    assert!(
        wide["scroll"].as_u64().unwrap_or(u64::MAX) <= wide["width"].as_u64().unwrap_or(0) + 24
    );
    assert!(page.requests(base).iter().all(|request| {
        !request_url(request).contains("/tasks?") && !request_url(request).contains(TOKEN)
    }));
    assert!(page.urls().iter().all(|url| !url.contains("/tasks?")));
}

fn tasks_state_flow(states: &str, empty: &str) {
    let mut page = Browser::open(&format!("{states}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#tasks");
    wait_case(
        &mut page,
        "loading or empty catalog",
        "document.getElementById('task-view-status')?.textContent.includes('Loading') || document.getElementById('task-view-status')?.textContent.includes('empty')",
    );
    wait_case(
        &mut page,
        "empty catalog",
        "document.getElementById('task-view-status')?.textContent.includes('empty')",
    );
    commit(&mut page, "inventory-tasks-limit", "1");
    wait_case(
        &mut page,
        "unauthorized",
        "document.getElementById('task-view-status')?.textContent.includes('not authorized')",
    );
    commit(&mut page, "inventory-tasks-limit", "2");
    wait_case(
        &mut page,
        "forbidden",
        "document.getElementById('task-view-status')?.textContent.includes('forbidden')",
    );
    commit(&mut page, "inventory-tasks-limit", "3");
    wait_case(
        &mut page,
        "unavailable",
        "document.getElementById('task-view-status')?.textContent.includes('unavailable')",
    );
    commit(&mut page, "inventory-tasks-limit", "4");
    wait_case(
        &mut page,
        "malformed",
        "document.getElementById('task-view-status')?.textContent.includes('malformed')",
    );
    page.goto(&format!("{empty}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#tasks");
    wait_case(
        &mut page,
        "empty catalog",
        "document.getElementById('task-view-status')?.textContent.includes('empty')",
    );
    expect(
        "empty catalog",
        &Value::Null,
        &page.eval("document.getElementById('task-pick-0')?.getAttribute('data-task')"),
    );
}

fn runs_flow(base: &str, tasks: &MemoryTasks, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    authorize_catalog(&mut page);
    page.wait_until("document.getElementById('task-start')");
    assert_validation_does_not_send(&mut page, base);
    start_current(&mut page);
    page.wait_until(
        "document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('completed') && document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('done')",
    );
    assert_start_shape(&page, base, "build", true);
    assert!(status_sends(&page, base) >= 1);
    assert_no_status_after_terminal(&mut page, base, "run-1");
    page.click("task-pick-2");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'home'");
    assert_eq!(
        eval_str(
            &mut page,
            "document.querySelector('[data-run=\"run-1\"]') ? 'yes' : ''"
        ),
        "yes"
    );
    start_current(&mut page);
    page.wait_until("document.querySelector('[data-run=\"run-2\"]')");
    start_current(&mut page);
    page.wait_until("document.querySelector('[data-run=\"run-3\"]')");
    assert_start_shape(&page, base, "home", false);
    assert_eq!(
        page.eval("document.querySelectorAll('#task-runs [data-run]').length"),
        json!(3)
    );
    page.click("task-pick-1");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'fail'");
    start_current(&mut page);
    page.wait_until(
        "document.querySelector('[data-run=\"run-4\"]')?.textContent.includes('failed') && document.querySelector('[data-run=\"run-4\"]')?.textContent.includes('boom')",
    );
    page.click("task-pick-3");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'stop'");
    start_current(&mut page);
    page.wait_until(
        "document.querySelector('[data-run=\"run-5\"]')?.textContent.includes('canceled')",
    );
    let after_starts = start_sends(&page, base);
    let after_status = status_sends(&page, base);
    page.show("#logs");
    page.show("#tasks");
    page.wait_until("document.querySelectorAll('#task-runs [data-run]').length === 5");
    page.wait_until(
        "document.querySelector('[data-run=\"run-2\"]')?.textContent.includes('completed')",
    );
    assert_eq!(start_sends(&page, base), after_starts);
    assert!(status_sends(&page, base) >= after_status);
    assert_eq!(
        page.eval("document.querySelector('#task-runs button')"),
        Value::Null
    );
    let cards = eval_str(
        &mut page,
        "document.getElementById('task-runs').textContent",
    );
    assert!(!cards.contains(TOKEN));
    assert!(page.requests(base).iter().all(|request| {
        !request_url(request).contains("/tasks?") && !request_url(request).contains(TOKEN)
    }));
    assert_start_faults_keep_cards(&mut page, tasks, handle);
}

fn assert_start_faults_keep_cards(
    page: &mut Browser,
    tasks: &MemoryTasks,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(tasks.set_unavailable("task provider is not configured"));
    start_current(page);
    page.wait_until(
        "document.getElementById('task-view-alert')?.textContent.includes('not configured')",
    );
    assert_eq!(
        page.eval("document.querySelectorAll('#task-runs [data-run]').length"),
        json!(5)
    );
    handle.block_on(tasks.clear_unavailable());
    page.show("#overview");
    page.set_value("bearer-token", "");
    page.show("#tasks");
    page.wait_until("document.getElementById('task-start')");
    start_current(page);
    page.wait_until(
        "document.getElementById('task-view-status')?.textContent.includes('could not be started')",
    );
    page.show("#overview");
    page.set_value("bearer-token", TOKEN);
    page.show("#tasks");
    page.wait_until("document.getElementById('task-start')");
    page.fail_network(true);
    start_current(page);
    page.wait_until(
        "document.getElementById('task-view-alert')?.textContent.includes('could not reach')",
    );
    page.fail_network(false);
    assert_eq!(
        page.eval("document.querySelectorAll('#task-runs [data-run]').length"),
        json!(5)
    );
    page.reload();
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    authorize_catalog(page);
    assert_eq!(
        page.eval("document.querySelector('#task-runs [data-run]')"),
        Value::Null
    );
}

fn run_script_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.set_value("bearer-token", TOKEN);
    page.show("#tasks");
    page.wait_until("document.getElementById('task-pick-0')?.getAttribute('data-task') === 'pace'");
    start_current(&mut page);
    page.wait_until(
        "document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('0.5') && document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('working')",
    );
    page.wait_until(
        "document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('completed') && document.querySelector('[data-run=\"run-1\"]')?.textContent.includes('ok')",
    );
    let completed = eval_str(
        &mut page,
        "document.querySelector('[data-run=\"run-1\"]').textContent",
    );
    expect_true(
        "progress then completed",
        completed.contains("completed") && completed.contains("[redacted]"),
        "completed card with redacted bearer",
        &completed,
    );
    expect_true(
        "bearer redaction",
        !completed.contains(TOKEN),
        "omitted bearer token",
        &completed,
    );
    let after = status_sends_for(&page, base, "run-1");
    std::thread::sleep(Duration::from_millis(400));
    let _ = page.eval("1");
    expect(
        "stale or duplicate status suppression",
        &json!(after),
        &json!(status_sends_for(&page, base, "run-1")),
    );
    page.click("task-pick-1");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'fault'");
    start_current(&mut page);
    page.wait_until(
        "document.querySelector('[data-run=\"run-2\"]')?.textContent.includes('failed') && document.querySelector('[data-run=\"run-2\"]')?.textContent.includes('execution') && document.querySelector('[data-run=\"run-2\"]')?.textContent.includes('boom')",
    );
    page.click("task-pick-3");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'reject'");
    let before = start_sends(&page, base);
    start_current(&mut page);
    page.wait_until(
        "document.getElementById('task-view-status')?.textContent.includes('could not be started')",
    );
    assert_eq!(
        page.eval("document.querySelector('[data-run=\"run-3\"]')"),
        Value::Null
    );
    assert!(start_sends(&page, base) > before);
    page.click("task-pick-2");
    page.wait_until("document.getElementById('task-detail-id')?.textContent === 'broken'");
    start_current(&mut page);
    page.wait_until(
        "document.getElementById('task-view-alert')?.textContent.includes('could not be read')",
    );
    assert_eq!(
        page.eval("document.querySelectorAll('#task-runs [data-run]').length"),
        json!(2)
    );
}

fn fallback_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#tasks");
    wait_case(
        &mut page,
        "unsupported-schema fallback",
        "document.getElementById('task-input-choice')?.tagName === 'TEXTAREA'",
    );
    page.set_value("task-input-choice", "not-json");
    page.eval(
        "document.getElementById('task-input-choice').dispatchEvent(new Event('input', {bubbles:true}))",
    );
    let before = start_sends(&page, base);
    start_current(&mut page);
    wait_case(
        &mut page,
        "invalid JSON fallback",
        "!!document.getElementById('task-input-choice-error')",
    );
    expect(
        "invalid JSON fallback does not send",
        &json!(before),
        &json!(start_sends(&page, base)),
    );
    let error = eval_str(
        &mut page,
        "document.getElementById('task-input-choice-error')?.textContent ?? ''",
    );
    expect_true(
        "invalid JSON fallback",
        error.contains("valid JSON"),
        "field error Enter valid JSON",
        &error,
    );
    page.set_value("task-input-choice", r#"{"picked":1}"#);
    page.eval(
        "document.getElementById('task-input-choice').dispatchEvent(new Event('input', {bubbles:true}))",
    );
    start_current(&mut page);
    wait_case(
        &mut page,
        "valid JSON fallback",
        "document.querySelector('#task-runs [data-run]')",
    );
    let body = last_operation(&page, base, "start_task");
    expect_true(
        "valid JSON fallback start",
        (body.contains("\"wait\":false") || body.contains("\"wait\": false"))
            && !body.contains("contextId")
            && body.contains("picked"),
        "start_task wait:false without contextId and with JSON object",
        &body,
    );
}

fn assert_validation_does_not_send(page: &mut Browser, base: &str) {
    page.set_value("task-input-name", "");
    page.eval(
        "document.getElementById('task-input-name').dispatchEvent(new Event('input', {bubbles:true}))",
    );
    let before = start_sends(page, base);
    start_current(page);
    page.wait_until("document.getElementById('task-input-name-error')");
    expect(
        "invalid schema submission",
        &json!(before),
        &json!(start_sends(page, base)),
    );
    page.set_value("task-input-name", "lab");
    page.eval(
        "document.getElementById('task-input-name').dispatchEvent(new Event('input', {bubbles:true}))",
    );
}

fn start_current(page: &mut Browser) {
    page.click("task-start");
}

fn assert_start_shape(page: &Browser, base: &str, task_id: &str, structured: bool) {
    let body = last_operation(page, base, "start_task");
    expect_true(
        "start_task wait:false",
        body.contains("\"wait\":false") || body.contains("\"wait\": false"),
        "wait:false",
        &body,
    );
    expect_true(
        "start_task task_id",
        body.contains(&format!("\"task_id\":\"{task_id}\""))
            || body.contains(&format!("\"task_id\": \"{task_id}\"")),
        &format!("task_id {task_id}"),
        &body,
    );
    expect_true(
        "start_task omits contextId",
        !body.contains("contextId"),
        "no contextId",
        &body,
    );
    if structured {
        expect_true(
            "valid schema submission",
            body.contains("\"name\""),
            "typed name field",
            &body,
        );
    } else {
        expect_true(
            "missing-schema empty input",
            body.contains("\"input\":{}") || body.contains("\"input\": {}"),
            "empty input object",
            &body,
        );
    }
}

fn assert_no_status_after_terminal(page: &mut Browser, base: &str, run_id: &str) {
    let before = status_sends_for(page, base, run_id);
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        let _ = page.eval("1");
        std::thread::sleep(Duration::from_millis(50));
    }
    expect(
        "stale or duplicate status suppression",
        &json!(before),
        &json!(status_sends_for(page, base, run_id)),
    );
}

fn status_sends_for(page: &Browser, base: &str, run_id: &str) -> usize {
    page.requests(base)
        .iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .filter(|request| {
            let body = request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("");
            body.contains("get_task_status") && body.contains(run_id)
        })
        .count()
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
        .filter(|request| {
            request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("")
                .contains(operation)
        })
        .count()
}

fn last_operation(page: &Browser, base: &str, operation: &str) -> String {
    page.requests(base)
        .iter()
        .rev()
        .filter(|request| request_url(request).contains("/message:send"))
        .find_map(|request| {
            let body = request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("");
            body.contains(operation).then(|| body.to_owned())
        })
        .unwrap_or_default()
}

fn commit(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('change', {{bubbles:true}}))"
    ));
}

fn eval_str(page: &mut Browser, expression: &str) -> String {
    page.eval(expression).as_str().unwrap_or("").to_owned()
}

fn wait_case(page: &mut Browser, case: &str, expression: &str) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if page.eval(&format!("!!({expression})")) == Value::Bool(true) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let observed = eval_str(
        page,
        "document.getElementById('task-view-status')?.textContent ?? document.body.innerText ?? ''",
    );
    panic!("Tasks reader, case {case}: expected {expression}, observed {observed}");
}

fn expect(case: &str, expected: &Value, observed: &Value) {
    assert_eq!(
        observed, expected,
        "Tasks reader, case {case}: expected {expected}, observed {observed}"
    );
}

fn expect_true(case: &str, ok: bool, expected: &str, observed: &str) {
    assert!(
        ok,
        "Tasks reader, case {case}: expected {expected}, observed {observed}"
    );
}

fn list_task_sends(page: &Browser, base: &str) -> usize {
    page.requests(base)
        .iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .filter(|request| {
            request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("")
                .contains("list_tasks")
        })
        .count()
}

pub(super) async fn scripted_send(body: Bytes) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let data = &value["message"]["parts"][0]["data"];
    let operation = data["operation"].as_str().unwrap_or("");
    let limit = data["params"]["page"]["limit"].as_u64().unwrap_or(100);
    if operation != "list_tasks" {
        return bytes(
            StatusCode::OK,
            "application/json",
            listed_items(operation, &json!([])),
        );
    }
    match limit {
        1 => bytes(
            StatusCode::UNAUTHORIZED,
            "application/json",
            json!({
                "error": {
                    "code": 401,
                    "status": "UNAUTHENTICATED",
                    "message": "missing token",
                    "details": [{
                        "reason": "UNAUTHENTICATED",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        ),
        2 => bytes(
            StatusCode::FORBIDDEN,
            "application/json",
            json!({
                "error": {
                    "code": 403,
                    "status": "PERMISSION_DENIED",
                    "message": "not allowed",
                    "details": [{
                        "reason": "PERMISSION_DENIED",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        ),
        3 => bytes(
            StatusCode::INTERNAL_SERVER_ERROR,
            "application/json",
            json!({
                "error": {
                    "code": 500,
                    "status": "INTERNAL",
                    "message": "task provider is not configured",
                    "details": [{
                        "reason": "INTERNAL_ERROR",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        ),
        4 => bytes(StatusCode::OK, "application/json", b"not-json".to_vec()),
        _ => {
            tokio::time::sleep(Duration::from_millis(800)).await;
            bytes(
                StatusCode::OK,
                "application/json",
                listed_items("list_tasks", &json!([])),
            )
        }
    }
}

fn listed_items(operation: &str, items: &Value) -> Vec<u8> {
    json!({
        "task": {
            "id": "protocol-task",
            "contextId": "protocol-context",
            "status": {"state": "TASK_STATE_COMPLETED"},
            "artifacts": [{
                "artifactId": "art-1",
                "parts": [{"data": {"operation": operation, "result": {"items": items}}}]
            }]
        }
    })
    .to_string()
    .into_bytes()
}

async fn task_lab(populated: bool, secured: bool) -> String {
    let tasks = MemoryTasks::new();
    if populated {
        tasks
            .insert(TaskDefinition {
                id: TaskId::new("build").unwrap(),
                name: MARKUP.to_owned(),
                description: format!("Use {TOKEN}"),
                asset_id: Some("asset-task".to_owned()),
                semantic_id: Some("semantic-task".to_owned()),
                input_schema: Some(
                    json!({
                        "type": "object",
                        "title": "<b>in</b>",
                        "properties": {
                            "name": {
                                "type": "string",
                                "title": "<i>Name</i>",
                                "description": format!("Use {TOKEN}"),
                                "default": format!("Bearer {TOKEN}")
                            }
                        }
                    })
                    .to_string(),
                ),
                output_schema: Some(r#"{"title":"<b>out</b>"}"#.to_owned()),
            })
            .await;
        tasks
            .insert(TaskDefinition {
                id: TaskId::new("home").unwrap(),
                name: "Home".to_owned(),
                description: "Home the axes".to_owned(),
                asset_id: None,
                semantic_id: None,
                input_schema: None,
                output_schema: None,
            })
            .await;
    }
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

async fn tasks_state_lab() -> String {
    serve_mock(
        json!({
            "name": "task-agent",
            "description": "Lab",
            "version": "1",
            "supportedInterfaces": [{
                "url": "http://127.0.0.1",
                "protocolBinding": "HTTP+JSON",
                "protocolVersion": "1.0"
            }],
            "capabilities": {"streaming": false},
            "skills": [
                {"id": "list-tasks", "name": "List tasks", "description": "List"}
            ]
        }),
        false,
        false,
        MockRoute::Tasks,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}

fn drive_runs(tasks: MemoryTasks, stop: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut working_at = BTreeMap::new();
        while !stop.load(Ordering::Relaxed) {
            for number in 1..=32 {
                let Ok(id) = RunId::new(format!("run-{number}")) else {
                    continue;
                };
                let Ok(run) = tasks.status(GetTaskStatusRequest { id: id.clone() }).await else {
                    continue;
                };
                match run.state {
                    TaskState::Submitted => {
                        let _ = tasks
                            .transition(&id, TaskState::Working, Some("working".to_owned()))
                            .await;
                        working_at
                            .entry(run.id.to_string())
                            .or_insert_with(std::time::Instant::now);
                    }
                    TaskState::Working => {
                        let ready = working_at
                            .get(run.id.as_str())
                            .is_some_and(|at| at.elapsed() >= Duration::from_millis(800));
                        if !ready {
                            continue;
                        }
                        let (state, message) = match run.task_id.as_str() {
                            "fail" => (TaskState::Failed, Some("boom".to_owned())),
                            "stop" => (TaskState::Canceled, Some("canceled".to_owned())),
                            _ => (TaskState::Completed, Some("done".to_owned())),
                        };
                        let _ = tasks.transition(&id, state, message).await;
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    });
}

async fn run_lab() -> (String, MemoryTasks, Arc<AtomicBool>) {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: MARKUP.to_owned(),
            description: format!("Use {TOKEN}"),
            asset_id: Some("asset-task".to_owned()),
            semantic_id: Some("semantic-task".to_owned()),
            input_schema: Some(
                json!({
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": {
                            "type": "string",
                            "title": "<i>Name</i>",
                            "description": format!("Use {TOKEN}"),
                            "minLength": 2,
                            "default": "lab"
                        }
                    }
                })
                .to_string(),
            ),
            output_schema: Some(r#"{"title":"<b>out</b>"}"#.to_owned()),
        })
        .await;
    for (id, name, description) in [
        ("fail", "Fail", "Fail the run"),
        ("home", "Home", "Home the axes"),
        ("stop", "Stop", "Cancel the run"),
    ] {
        tasks
            .insert(TaskDefinition {
                id: TaskId::new(id).unwrap(),
                name: name.to_owned(),
                description: description.to_owned(),
                asset_id: None,
                semantic_id: None,
                input_schema: None,
                output_schema: None,
            })
            .await;
    }
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks.clone()).share();
    let (listener, address) = bind_local().await.unwrap();
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
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_security(schemes, requirements);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    (
        format!("http://{address}"),
        tasks,
        Arc::new(AtomicBool::new(false)),
    )
}

async fn fallback_lab() -> (String, MemoryTasks, Arc<AtomicBool>) {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("open").unwrap(),
            name: "Open".to_owned(),
            description: "Unsupported schema".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: Some(
                json!({
                    "type": "object",
                    "properties": {
                        "choice": {
                            "title": "Choice",
                            "oneOf": [{"type": "string"}, {"type": "number"}]
                        }
                    }
                })
                .to_string(),
            ),
            output_schema: None,
        })
        .await;
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks.clone()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    (
        format!("http://{address}"),
        tasks,
        Arc::new(AtomicBool::new(false)),
    )
}

async fn run_script_lab() -> String {
    serve_mock(
        json!({
            "name": "task-agent",
            "description": "Lab",
            "version": "1",
            "supportedInterfaces": [{
                "url": "http://127.0.0.1",
                "protocolBinding": "HTTP+JSON",
                "protocolVersion": "1.0"
            }],
            "capabilities": {"streaming": false},
            "securitySchemes": {
                "bearerAuth": {"httpAuthSecurityScheme": {"scheme": "bearer"}}
            },
            "securityRequirements": [{"bearerAuth": []}],
            "skills": [
                {"id": "list-tasks", "name": "List tasks", "description": "List"}
            ]
        }),
        false,
        false,
        MockRoute::TaskRuns,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}

pub(super) fn scripted_run_send(
    body: &Bytes,
    starts: &AtomicU32,
    polls: &Mutex<BTreeMap<String, u32>>,
) -> Response {
    let value: Value = serde_json::from_slice(body).unwrap_or(json!({}));
    let data = &value["message"]["parts"][0]["data"];
    match data["operation"].as_str().unwrap_or("") {
        "list_tasks" => bytes(
            StatusCode::OK,
            "application/json",
            listed_items("list_tasks", &run_definitions()),
        ),
        "start_task" => start_script(data, starts),
        "get_task_status" => status_script(data, polls),
        _ => bytes(
            StatusCode::OK,
            "application/json",
            listed_items("list_tasks", &json!([])),
        ),
    }
}

fn start_script(data: &Value, starts: &AtomicU32) -> Response {
    let task_id = data["params"]["task_id"].as_str().unwrap_or("");
    match task_id {
        "reject" => bytes(
            StatusCode::OK,
            "application/json",
            json!({
                "error": {
                    "code": 5,
                    "status": "NOT_FOUND",
                    "message": "task `reject` was not found",
                    "details": [{"reason": "NOT_FOUND"}]
                }
            })
            .to_string()
            .into_bytes(),
        ),
        "broken" => bytes(StatusCode::OK, "application/json", b"not-json".to_vec()),
        _ => {
            let number = starts.fetch_add(1, Ordering::Relaxed) + 1;
            bytes(
                StatusCode::OK,
                "application/json",
                run_payload(
                    "start_task",
                    &json!({
                        "id": format!("run-{number}"),
                        "task_id": task_id,
                        "state": "submitted",
                        "input": data["params"]["input"]
                    }),
                ),
            )
        }
    }
}

fn status_script(data: &Value, polls: &Mutex<BTreeMap<String, u32>>) -> Response {
    let run_id = data["params"]["id"].as_str().unwrap_or("").to_owned();
    let count = {
        let mut polls = polls.lock().expect("run polls");
        let entry = polls.entry(run_id.clone()).or_insert(0);
        *entry += 1;
        *entry
    };
    let result = if run_id == "run-2" {
        if count == 1 {
            json!({
                "id": run_id,
                "task_id": "fault",
                "state": "working",
                "message": "working"
            })
        } else {
            json!({
                "id": run_id,
                "task_id": "fault",
                "state": "failed",
                "message": format!("bad {TOKEN}"),
                "error_kind": "execution",
                "error_identifier": "boom"
            })
        }
    } else if count == 1 {
        json!({
            "id": run_id,
            "task_id": "pace",
            "state": "working",
            "progress": 0.5,
            "message": format!("Use {TOKEN}")
        })
    } else {
        json!({
            "id": run_id,
            "task_id": "pace",
            "state": "completed",
            "message": "done",
            "result": {"ok": true, "note": format!("Bearer {TOKEN}")}
        })
    };
    bytes(
        StatusCode::OK,
        "application/json",
        run_payload("get_task_status", &result),
    )
}

fn run_definitions() -> Value {
    json!([
        {
            "id": "pace",
            "name": "Pace",
            "description": "Timed",
            "input_schema": "{\"type\":\"object\",\"properties\":{}}"
        },
        {
            "id": "fault",
            "name": "Fault",
            "description": "Fail",
            "input_schema": null
        },
        {
            "id": "broken",
            "name": "Broken",
            "description": "Malformed"
        },
        {
            "id": "reject",
            "name": "Reject",
            "description": "Missing"
        }
    ])
}

fn run_payload(operation: &str, result: &Value) -> Vec<u8> {
    json!({
        "task": {
            "id": "protocol-task",
            "contextId": "protocol-context",
            "status": {"state": "TASK_STATE_COMPLETED"},
            "artifacts": [{
                "artifactId": "art-1",
                "parts": [{"data": {"operation": operation, "result": result}}]
            }]
        }
    })
    .to_string()
    .into_bytes()
}
