//! Browser coverage for the primitive command workbench.

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{Browser, bytes, header, request_method, request_url, serve_mock};

const MARKUP: &str = r#"<img id="workbench-xss" src=x onerror="window.__workbenchXss=1">"#;
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const SVG_BYTES: &str =
    "PGltZyBpZD0id29ya2JlbmNoLXhzcyIgc3JjPXggb25lcnJvcj0id2luZG93Ll9fd29ya2JlbmNoWHNzPTEiPg==";

#[tokio::test]
async fn browser_workbench_sends_lab_commands_without_storing_them() {
    let base = workbench_agent().await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || workbench_flow(&address))
        .await
        .unwrap();
}

fn workbench_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#workbench");
    assert_commands(&mut page);
    reject_local_json(&mut page);
    open_protocol(&mut page);
    page.set_value("context-id", "keep-me");
    show_structured_error(&mut page);
    prefill_and_send(&mut page);
    preview_images(&mut page);
    assert_history(&mut page, base);
}

fn open_protocol(page: &mut Browser) {
    page.show("#agent");
    if page.eval("document.getElementById('protocol-advanced')?.open === true") != Value::Bool(true)
    {
        page.click("protocol-advanced-summary");
    }
    page.wait_until("document.getElementById('protocol-advanced')?.open === true");
}

fn assert_commands(page: &mut Browser) {
    let mut values = Vec::new();
    for hash in ["#logs", "#metrics", "#tasks", "#images"] {
        page.show(hash);
        let options = page.eval(
            "[...document.getElementById('workbench-operation').options].map(option => option.value)",
        );
        values.extend(
            options
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    for name in [
        "list_log_sources",
        "query_logs",
        "list_metrics",
        "query_metric",
        "list_tasks",
        "start_task",
        "get_task_status",
        "list_image_sources",
        "list_images",
        "search_images",
        "get_image",
        "get_current_image",
    ] {
        assert!(values.iter().any(|value| value == name), "{name}");
    }
    let note = eval_str(
        page,
        "document.getElementById('workbench-note').textContent",
    );
    assert!(note.contains("image/png"));
    assert!(note.contains("image/jpeg"));
    assert!(note.contains("image/gif"));
    assert!(note.contains("image/webp"));
    assert!(note.contains("262144"));
}

fn reject_local_json(page: &mut Browser) {
    page.set_value("workbench-params", "{");
    page.click("workbench-send");
    page.wait_until(
        "document.getElementById('workbench-alert').textContent.includes('not valid JSON')",
    );
    assert_eq!(
        eval_str(page, "document.getElementById('workbench-params').value"),
        "{"
    );
    page.set_value("workbench-params", "[]");
    page.click("workbench-send");
    page.wait_until(
        "document.getElementById('workbench-alert').textContent.includes('JSON object')",
    );
    assert_eq!(
        eval_str(page, "document.getElementById('workbench-params').value"),
        "[]"
    );
}

fn show_structured_error(page: &mut Browser) {
    page.show("#logs");
    page.set_value("workbench-operation", "query_logs");
    page.set_value(
        "workbench-params",
        r#"{"source_id":"log-a","range":{"start":"2020-01-02T00:00:00Z","end":"2020-01-01T00:00:00Z"},"page":{"limit":0}}"#,
    );
    page.click("workbench-send");
    page.wait_until("document.getElementById('workbench')?.dataset.kind === 'rpc'");
    let facts = facts(page);
    assert_eq!(facts["status"].as_str(), Some("INVALID_ARGUMENT"));
    assert_eq!(facts["reason"].as_str(), Some("INVALID_PARAMS"));
    assert_eq!(facts["domain"].as_str(), Some("a2a-protocol.org"));
    assert_eq!(facts["field"].as_str(), Some("range"));
    assert_eq!(facts["correlation"].as_str(), Some("req-range"));
    assert_ne!(facts["message"], facts["detail"]);
    assert!(eval_str(page, "document.getElementById('workbench-params').value").contains("limit"));
}

fn prefill_and_send(page: &mut Browser) {
    page.show("#inventory/logs");
    page.wait_until(
        "document.querySelector('#inventory-logs-items [data-command=\"query_logs\"]')",
    );
    page.click_selector("#inventory-logs-items [data-command=\"query_logs\"]");
    page.show("#workbench");
    page.wait_until("document.getElementById('workbench-params').value.includes('log-a')");
    let prefilled = eval_str(page, "document.getElementById('workbench-params').value");
    assert!(prefilled.contains("\"source_id\""));
    assert!(prefilled.contains("log-a"));
    assert!(prefilled.contains("\"range\""));
    let edited = prefilled.replace("2020-01-01T01:00:00Z", "2020-01-01T02:00:00Z");
    page.set_value("workbench-params", &edited);
    page.click("workbench-send");
    page.wait_until("document.getElementById('workbench')?.dataset.kind === 'success'");
    let facts = facts(page);
    assert_eq!(facts["operation"].as_str(), Some("query_logs"));
    assert_eq!(facts["protocol-task"].as_str(), Some("protocol-task"));
    assert_eq!(facts["protocol-context"].as_str(), Some("protocol-context"));
    assert_eq!(
        facts["protocol-state"].as_str(),
        Some("TASK_STATE_COMPLETED")
    );
    assert!(facts.get("lab-run").is_none());
    assert!(
        eval_str(page, "document.getElementById('workbench-params').value")
            .contains("2020-01-01T02:00:00Z")
    );
    let text = eval_str(
        page,
        "document.getElementById('workbench-tree').textContent",
    );
    assert!(text.contains(MARKUP));
    assert!(text.contains("onerror"));
    assert_eq!(
        page.eval("document.getElementById('workbench-xss')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__workbenchXss"), Value::Null);
    page.show("#agent");
    assert_eq!(
        eval_str(page, "document.getElementById('context-id').value"),
        "keep-me"
    );
}

fn preview_images(page: &mut Browser) {
    page.show("#images");
    page.set_value("workbench-operation", "get_image");
    page.set_value("workbench-params", r#"{"id":"png"}"#);
    page.click("workbench-send");
    page.wait_until(
        "document.getElementById('workbench-preview')?.getAttribute('src')?.startsWith('data:image/png')",
    );
    let src = eval_str(
        page,
        "document.getElementById('workbench-preview').getAttribute('src')",
    );
    assert!(src.starts_with("data:image/png;base64,"));
    assert!(!src.contains(MARKUP));
    let summary = eval_str(
        page,
        "document.getElementById('workbench-image').textContent",
    );
    assert!(summary.contains("image/png"));
    assert!(summary.contains("Decoded bytes"));
    page.set_value("workbench-operation", "get_current_image");
    page.set_value("workbench-params", r#"{"source_id":"svg"}"#);
    page.click("workbench-send");
    page.wait_until(
        "document.getElementById('workbench-image')?.textContent.includes('image/svg+xml')",
    );
    let summary = eval_str(
        page,
        "document.getElementById('workbench-image').textContent",
    );
    assert!(summary.contains("image/svg+xml"));
    assert!(summary.contains("allowlist"));
    assert!(!summary.contains(SVG_BYTES));
    assert_eq!(
        page.eval("document.getElementById('workbench-xss')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__workbenchXss"), Value::Null);
}

fn assert_history(page: &mut Browser, base: &str) {
    let before = page.eval("document.querySelectorAll('#workbench-history [data-retry]').length");
    assert!(before.as_u64().unwrap_or(0) >= 2);
    let stored = page.eval("JSON.stringify(localStorage) + JSON.stringify(sessionStorage)");
    let stored = stored.as_str().unwrap_or("");
    assert!(!stored.contains("workbench"));
    assert!(!stored.contains("protocol-task"));
    assert!(!stored.contains(PNG));
    page.click_selector("#workbench-history li:last-child [data-retry]");
    page.wait_until(&format!(
        "document.querySelectorAll('#workbench-history [data-retry]').length > {}",
        before.as_u64().unwrap_or(0)
    ));
    for request in page.requests(base) {
        if request_url(&request).contains("message:send") {
            assert_eq!(request_method(&request), "POST");
            let post = request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("");
            assert!(!post.contains("contextId"), "{post}");
            assert!(!post.contains("keep-me"), "{post}");
            assert!(header(&request, "authorization").is_none());
        }
    }
    page.show("#agent");
    assert_eq!(
        eval_str(page, "document.getElementById('context-id').value"),
        "keep-me"
    );
}

fn facts(page: &mut Browser) -> Value {
    page.eval(
        "Object.fromEntries([...document.querySelectorAll('#workbench-facts [data-fact]')].map(node => [node.dataset.fact, node.textContent]))",
    )
}

fn eval_str(page: &mut Browser, expression: &str) -> String {
    page.eval(expression).as_str().unwrap_or("").to_owned()
}

async fn workbench_agent() -> String {
    serve_mock(
        json!({
            "name": "workbench-agent",
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
        super::MockRoute::Workbench,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}

pub(super) fn scripted_send(body: &Bytes) -> Response {
    let value: Value = serde_json::from_slice(body).unwrap_or(json!({}));
    let data = &value["message"]["parts"][0]["data"];
    let operation = data["operation"].as_str().unwrap_or("");
    let limit = data["params"]["page"]["limit"].as_u64();
    if limit == Some(0) {
        return bytes(
            StatusCode::BAD_REQUEST,
            "application/json",
            json!({
                "error": {
                    "code": 400,
                    "status": "INVALID_ARGUMENT",
                    "message": "bad range",
                    "details": [
                        {
                            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                            "reason": "INVALID_PARAMS",
                            "domain": "a2a-protocol.org"
                        },
                        {
                            "@type": "type.googleapis.com/google.rpc.BadRequest",
                            "fieldViolations": [{
                                "field": "range",
                                "description": "start must be before end"
                            }]
                        },
                        {
                            "@type": "type.googleapis.com/google.rpc.RequestInfo",
                            "requestId": "req-range"
                        }
                    ]
                }
            })
            .to_string()
            .into_bytes(),
        );
    }
    let result = match operation {
        "list_log_sources" => json!({
            "items": [{
                "id": "log-a",
                "name": "App",
                "description": MARKUP
            }]
        }),
        "query_logs" => json!({"items": [{"id": "row-1", "message": MARKUP}]}),
        "get_image" => json!({
            "descriptor": {"id": "frame", "media_type": "image/png"},
            "data": PNG
        }),
        "get_current_image" => json!({
            "descriptor": {"id": "frame", "media_type": "image/svg+xml"},
            "data": SVG_BYTES
        }),
        _ => json!({"items": []}),
    };
    bytes(
        StatusCode::OK,
        "application/json",
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
        .into_bytes(),
    )
}
