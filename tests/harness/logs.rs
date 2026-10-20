//! Browser coverage for the logs viewer.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{Browser, MARKUP, bytes, request_method, request_url, serve_mock};

static LOG_A_QUERIES: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn browser_logs_filters_pages_and_keeps_route_state() {
    LOG_A_QUERIES.store(0, Ordering::Relaxed);
    let base = logs_agent().await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || logs_flow(&address))
        .await
        .unwrap();
}

fn logs_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#logs");
    page.wait_until("document.querySelectorAll('#log-view-entries .log-card').length >= 3");
    assert_eq!(
        eval_str(
            &mut page,
            "document.getElementById('log-filter-count').textContent"
        ),
        "3 logs"
    );
    assert!(
        eval_str(
            &mut page,
            "document.getElementById('log-view-entries').textContent"
        )
        .contains(MARKUP)
    );
    assert_eq!(
        page.eval("document.getElementById('xss-payload')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__xss"), Value::Null);

    commit(&mut page, "log-filter-search", "pump");
    page.wait_until("document.getElementById('log-filter-count').textContent === '1 of 3 logs'");
    assert!(
        eval_str(
            &mut page,
            "document.getElementById('log-filter-chips').textContent"
        )
        .contains("pump")
    );
    page.click_selector("#log-filter-chips [data-clear='search']");
    page.wait_until("document.getElementById('log-filter-count').textContent === '3 logs'");

    page.click("log-filter-error");
    page.wait_until("document.getElementById('log-filter-count').textContent === '1 of 3 logs'");
    page.click("log-filter-all");
    page.wait_until("document.getElementById('log-filter-count').textContent === '3 logs'");
    commit(&mut page, "log-filter-search", "no-such-log");
    page.wait_until("document.getElementById('log-result-state')?.dataset.state === 'nomatch'");
    page.click_selector("#log-filter-chips [data-clear='search']");
    page.wait_until("document.getElementById('log-filter-count').textContent === '3 logs'");

    page.click("log-source-1");
    page.wait_until(
        "document.getElementById('log-view-entries').textContent.includes('kernel boot')",
    );
    page.wait_until("document.querySelectorAll('#log-view-entries .log-card').length === 1");

    page.click("log-source-0");
    page.click("log-source-0");
    page.wait_until(
        "document.getElementById('log-view-entries').textContent.includes('kernel boot')",
    );
    assert!(
        !eval_str(
            &mut page,
            "document.getElementById('log-view-entries').textContent"
        )
        .contains("pump ready")
    );

    commit(&mut page, "log-range-start", "1999-01-01");
    commit(&mut page, "log-range-end", "1999-01-02");
    page.wait_until(
        "document.getElementById('log-view-alert').textContent.includes('not authorized')",
    );

    page.show("#agent");
    page.show("#logs");
    page.wait_until(
        "document.getElementById('log-view-alert').textContent.includes('not authorized')",
    );
    assert_eq!(
        eval_str(
            &mut page,
            "document.getElementById('log-range-start').value"
        ),
        "1999-01-01"
    );

    for request in page.requests(base) {
        if request_url(&request).contains("message:send") {
            assert_eq!(request_method(&request), "POST");
            let post = request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("");
            if post.contains("query_logs") {
                assert!(post.contains("\"limit\":100"), "{post}");
            }
        }
    }
}

async fn logs_agent() -> String {
    serve_mock(
        json!({
            "name": "logs-agent",
            "description": "Lab",
            "version": "1",
            "supportedInterfaces": [{
                "url": "http://127.0.0.1",
                "protocolBinding": "HTTP+JSON",
                "protocolVersion": "1.0"
            }],
            "capabilities": {"streaming": false},
            "skills": [
                {"id": "list-log-sources", "name": "List log sources", "description": "List"}
            ]
        }),
        false,
        false,
        super::MockRoute::Logs,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}

pub(super) async fn scripted_send(body: &Bytes) -> Response {
    let value: Value = serde_json::from_slice(body).unwrap_or(json!({}));
    let data = &value["message"]["parts"][0]["data"];
    let operation = data["operation"].as_str().unwrap_or("");
    match operation {
        "list_log_sources" => bytes(
            StatusCode::OK,
            "application/json",
            listed(
                "list_log_sources",
                &json!([
                    {"id": "log-a", "name": "App", "description": "App log"},
                    {"id": "log-b", "name": "Kernel", "description": "Kernel log"}
                ]),
                None,
            ),
        ),
        "query_logs" => query_logs(data).await,
        _ => bytes(StatusCode::OK, "application/json", b"not-json".to_vec()),
    }
}

async fn query_logs(data: &Value) -> Response {
    let source = data["params"]["source_id"].as_str().unwrap_or("");
    let start = data["params"]["range"]["start"].as_str().unwrap_or("");
    if start.starts_with("1999-01-01") {
        return bytes(
            StatusCode::UNAUTHORIZED,
            "application/json",
            json!({
                "error": {
                    "code": 401,
                    "status": "UNAUTHENTICATED",
                    "message": "not authorized to read logs"
                }
            })
            .to_string()
            .into_bytes(),
        );
    }
    if source == "log-a" {
        let hit = LOG_A_QUERIES.fetch_add(1, Ordering::Relaxed);
        if hit >= 1 {
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
        return bytes(
            StatusCode::OK,
            "application/json",
            listed(
                "query_logs",
                &json!([
                    {
                        "source_id": "log-a",
                        "timestamp": "2020-01-01T00:00:00Z",
                        "level": "info",
                        "message": "pump ready",
                        "attributes": {"code": "ok"}
                    },
                    {
                        "source_id": "log-a",
                        "timestamp": "2020-01-01T00:00:01Z",
                        "level": "error",
                        "message": MARKUP,
                        "attributes": {"code": "fail"}
                    }
                ]),
                Some("1"),
            ),
        );
    }
    bytes(
        StatusCode::OK,
        "application/json",
        listed(
            "query_logs",
            &json!([{
                "source_id": "log-b",
                "timestamp": "2020-01-01T00:00:02Z",
                "level": "warn",
                "message": "kernel boot",
                "attributes": {}
            }]),
            None,
        ),
    )
}

fn listed(operation: &str, items: &Value, next: Option<&str>) -> Vec<u8> {
    let mut result = json!({ "items": items });
    if let Some(next) = next {
        result["next_cursor"] = json!(next);
    }
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
