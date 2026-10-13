//! Browser coverage for the primitive inventory.

use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, ImageSource, ImageSourceId, LogSource,
    MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId,
    SecurityScheme, SourceId, TaskDefinition, TaskId, bind_local,
};
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{Browser, MARKUP, TOKEN, bytes, header, request_method, request_url, serve_mock};

#[tokio::test]
async fn browser_inventory_pages_the_connected_agent() {
    let base = inventory_lab().await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || inventory_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_inventory_states_stay_independent() {
    let base = scripted_inventory().await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || inventory_states(&address))
        .await
        .unwrap();
}

fn inventory_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    let stored = seed_session(&mut page);
    open_protocol(&mut page);
    page.set_value("context-id", "keep-me");
    assert_advertised(&mut page);
    reject_without_token(&mut page, &stored);
    page.show("#overview");
    page.set_value("bearer-token", TOKEN);
    page_all(&mut page);
    assert_log_text(&mut page);
    assert_task_text(&mut page);
    assert_inventory_requests(&page, base);
    assert_eq!(stored_session(&mut page), stored);
    assert_narrow(&mut page);
    reload_forgets_token(&mut page, &stored);
}

fn assert_advertised(page: &mut Browser) {
    for slug in ["logs", "metrics", "tasks", "images"] {
        page.show(&format!("#inventory/{slug}"));
        wait_inventory(page, slug, "unauthorized");
        let view = inventory_view(page, slug);
        assert_eq!(view["configured"], "false");
        assert_eq!(view["advertised"], "true");
        assert_eq!(view["live"], "polite");
        assert_eq!(view["alertRole"], "alert");
        assert!(view["items"].as_array().unwrap().is_empty());
        assert!(!view["heading"].as_str().unwrap_or("").is_empty());
    }
}

fn reject_without_token(page: &mut Browser, stored: &str) {
    page.show("#inventory/logs");
    wait_inventory(page, "logs", "unauthorized");
    let rejected = inventory_view(page, "logs");
    assert_eq!(rejected["configured"], "false");
    assert!(rejected["items"].as_array().unwrap().is_empty());
    assert_eq!(
        eval_str(page, "document.getElementById('connection').dataset.state"),
        "connected"
    );
    open_protocol(page);
    assert_eq!(
        eval_str(page, "document.getElementById('alert').textContent"),
        ""
    );
    assert_eq!(
        eval_str(page, "document.getElementById('context-id').value"),
        "keep-me"
    );
    assert_eq!(stored_session(page), stored);
}

fn page_all(page: &mut Browser) {
    for (slug, first, second) in [
        ("logs", "log-a", "log-b"),
        ("metrics", "metric-a", "metric-b"),
        ("tasks", "task-a", "task-b"),
        ("images", "image-a", "image-b"),
    ] {
        page_one(page, slug, first, second);
        let logs = inventory_view(page, "logs");
        assert_eq!(logs["items"][0], "log-a");
        assert_eq!(logs["items"][1], "log-b");
    }
    page.show("#inventory/images");
    commit(page, "inventory-images-limit", "100");
    page.wait_until("document.querySelectorAll('#inventory-images-items [data-id]').length === 2");
    let images = inventory_view(page, "images");
    assert_eq!(images["items"][0], "image-a");
    assert_eq!(images["items"][1], "image-b");
}

fn page_one(page: &mut Browser, slug: &str, first: &str, second: &str) {
    page.show(&format!("#inventory/{slug}"));
    commit(page, &format!("inventory-{slug}-limit"), "1");
    commit(page, &format!("inventory-{slug}-cursor"), "");
    wait_inventory(page, slug, "populated");
    page.wait_until(&format!(
        "document.querySelectorAll('#inventory-{slug}-items [data-id]').length === 1"
    ));
    let first_page = inventory_view(page, slug);
    assert_eq!(first_page["configured"], "true");
    assert_eq!(first_page["items"][0], first);
    assert_eq!(first_page["next"], "1");
    page.click(&format!("inventory-{slug}-more"));
    page.wait_until(&format!(
        "document.querySelectorAll('#inventory-{slug}-items [data-id]').length === 2"
    ));
    let both = inventory_view(page, slug);
    assert_eq!(both["items"][0], first);
    assert_eq!(both["items"][1], second);
    assert_eq!(both["next"], "");
}

fn assert_log_text(page: &mut Browser) {
    let body = inventory_view(page, "logs")["body"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(body.contains("onerror"));
    assert!(body.contains("Asset id"));
    assert!(body.contains("semantic-log"));
    assert!(!body.contains("stored-context"));
    assert!(!body.contains("keep-me"));
    let metric_body = inventory_view(page, "metrics")["body"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(metric_body.contains("Unit"));
    assert!(metric_body.contains("ms"));
}

fn assert_task_text(page: &mut Browser) {
    let task_body = inventory_view(page, "tasks")["body"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(task_body.contains("<b>in</b>"));
    assert!(task_body.contains("<b>out</b>"));
    assert!(task_body.contains("Id task-a"));
    assert_eq!(
        eval_str(
            page,
            "document.getElementById('inventory-tasks-note').textContent"
        ),
        "Lab task definitions. This list does not use chat sessions or protocol tasks."
    );
    assert_eq!(page.eval("document.querySelector('b')"), Value::Null);
    assert_eq!(
        page.eval("document.getElementById('xss-payload')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__xss"), Value::Null);
    open_protocol(page);
    assert_eq!(
        eval_str(page, "document.getElementById('context-id').value"),
        "keep-me"
    );
    page.show("#chat");
    assert_eq!(
        eval_str(page, "document.getElementById('chat').dataset.context"),
        ""
    );
}

fn assert_narrow(page: &mut Browser) {
    page.show("#inventory/logs");
    let shell = page.snapshot();
    assert!(shell["unlabeled"].as_array().unwrap().is_empty());
    assert_eq!(shell["blankButtons"], 0);
    page.viewport(360, 800);
    let narrow = page.snapshot();
    assert!(
        narrow["scroll"].as_u64().unwrap_or(u64::MAX) <= narrow["width"].as_u64().unwrap_or(0) + 24
    );
    let button = page
        .eval("document.getElementById('log-view-get').getBoundingClientRect().width")
        .as_f64()
        .unwrap_or(0.0);
    assert!(button > 0.0);
    assert!(button <= narrow["width"].as_f64().unwrap_or(0.0));
}

fn reload_forgets_token(page: &mut Browser, stored: &str) {
    page.reload();
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#overview");
    assert_eq!(
        eval_str(page, "document.getElementById('bearer-token').value"),
        ""
    );
    assert_eq!(stored_session(page), stored);
    page.show("#inventory/logs");
    wait_inventory(page, "logs", "unauthorized");
}

fn inventory_states(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    let stored = seed_session(&mut page);
    open_protocol(&mut page);
    page.set_value("context-id", "keep-me");
    assert_scripted_advertisement(&mut page);
    page_scripted_logs(&mut page);
    page_scripted_metrics(&mut page);
    fail_remaining_primitives(&mut page, base);
    assert_scripted_requests(&mut page, base, &stored);
    assert_narrow(&mut page);
}

fn assert_scripted_advertisement(page: &mut Browser) {
    page.show("#inventory/logs");
    wait_inventory(page, "logs", "populated");
    assert_eq!(inventory_view(page, "logs")["advertised"], "true");
    assert_eq!(
        page.eval("document.getElementById('inventory-logs-refresh')"),
        Value::Null
    );
}

fn page_scripted_logs(page: &mut Browser) {
    page.show("#inventory/logs");
    wait_inventory(page, "logs", "populated");
    let logs = inventory_view(page, "logs");
    assert_eq!(logs["items"][0], "log-a");
    assert!(logs["body"].as_str().unwrap_or("").contains("<img"));
    assert_eq!(
        page.eval("document.getElementById('inventory-xss')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__inventoryXss"), Value::Null);
    page.click("inventory-logs-more");
    page.wait_until("document.querySelectorAll('#inventory-logs-items [data-id]').length === 2");
    assert_eq!(inventory_view(page, "logs")["items"][1], "log-b");
}

fn page_scripted_metrics(page: &mut Browser) {
    page.show("#inventory/metrics");
    wait_inventory(page, "metrics", "loading");
    assert_eq!(inventory_view(page, "logs")["configured"], "true");
    assert_eq!(
        inventory_view(page, "logs")["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    wait_inventory(page, "metrics", "empty");
    let metrics = inventory_view(page, "metrics");
    assert_eq!(metrics["configured"], "true");
    assert!(metrics["text"].as_str().unwrap_or("").contains("empty"));
    assert!(metrics["items"].as_array().unwrap().is_empty());
    page.click("inventory-metrics-more");
    page.wait_until(
        "document.getElementById('inventory-metrics-alert').textContent.includes('no further page')",
    );
    assert_eq!(inventory_view(page, "metrics")["state"], "empty");
}

fn fail_remaining_primitives(page: &mut Browser, base: &str) {
    page.show("#inventory/tasks");
    wait_inventory(page, "tasks", "malformed");
    assert_eq!(inventory_view(page, "metrics")["state"], "empty");
    page.show("#inventory/images");
    wait_inventory(page, "images", "unavailable");
    let images = inventory_view(page, "images");
    assert_eq!(images["configured"], "false");
    assert_eq!(images["advertised"], "false");
    assert!(
        images["text"]
            .as_str()
            .unwrap_or("")
            .contains("not configured")
    );
    assert_eq!(inventory_view(page, "metrics")["configured"], "true");
    assert_eq!(
        eval_str(page, "document.getElementById('connection').dataset.state"),
        "connected"
    );
    page.show("#inventory/metrics");
    let before = page.requests(base).len();
    commit(page, "inventory-metrics-limit", "0");
    page.wait_until(
        "document.getElementById('inventory-metrics-alert').textContent.includes('1 to 1000')",
    );
    assert_eq!(inventory_view(page, "metrics")["state"], "empty");
    assert_eq!(page.requests(base).len(), before);
    page.show("#inventory/logs");
    commit(page, "inventory-logs-cursor", "next/page");
    page.wait_until(
        "document.getElementById('inventory-logs-alert').textContent.includes('numeric cursor')",
    );
    assert_eq!(
        inventory_view(page, "logs")["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    page.fail_network(true);
    page.set_value("inventory-logs-cursor", "");
    commit(page, "inventory-logs-limit", "1");
    wait_inventory(page, "logs", "transport");
    assert_eq!(inventory_view(page, "logs")["configured"], "false");
    assert_eq!(inventory_view(page, "metrics")["state"], "empty");
}

fn assert_scripted_requests(page: &mut Browser, base: &str, stored: &str) {
    open_protocol(page);
    assert_eq!(
        eval_str(page, "document.getElementById('context-id').value"),
        "keep-me"
    );
    assert_eq!(stored_session(page), stored);
    let bodies = post_bodies(page, base);
    assert!(bodies.iter().any(|body| body.contains("list_log_sources")));
    assert!(bodies.iter().any(|body| body.contains("list_metrics")));
    assert!(bodies.iter().any(|body| body.contains("list_tasks")));
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("list_image_sources"))
    );
    assert!(bodies.iter().all(|body| !body.contains("contextId")));
    assert!(bodies.iter().all(|body| !body.contains("keep-me")));
    assert!(bodies.iter().all(|body| !body.contains("stored-context")));
}

fn eval_str(page: &mut Browser, expression: &str) -> String {
    page.eval(expression).as_str().unwrap_or("").to_owned()
}

fn commit(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('change', {{bubbles:true}}))"
    ));
}

fn inventory_view(page: &mut Browser, slug: &str) -> Value {
    page.show(&format!("#inventory/{slug}"));
    let slug = serde_json::to_string(slug).unwrap_or_default();
    page.eval(&format!(
        "(() => {{
            const slug = {slug};
            const status = document.getElementById('inventory-' + slug + '-status');
            return {{
                state: status?.dataset.state ?? '',
                configured: status?.dataset.configured ?? '',
                advertised: status?.dataset.advertised ?? '',
                next: status?.dataset.next ?? '',
                live: status?.getAttribute('aria-live') ?? '',
                alertRole: document.getElementById('inventory-' + slug + '-alert')?.getAttribute('role') ?? '',
                text: status?.textContent ?? '',
                alert: document.getElementById('inventory-' + slug + '-alert')?.textContent ?? '',
                body: document.getElementById('inventory-' + slug + '-items')?.textContent ?? '',
                items: [...document.querySelectorAll('#inventory-' + slug + '-items [data-id]')].map(el => el.dataset.id),
                heading: document.getElementById('inventory-' + slug + '-heading')?.textContent ?? ''
            }};
        }})()"
    ))
}

fn wait_inventory(page: &mut Browser, slug: &str, state: &str) {
    page.show(&format!("#inventory/{slug}"));
    let slug = serde_json::to_string(slug).unwrap_or_default();
    let state = serde_json::to_string(state).unwrap_or_default();
    page.wait_until(&format!(
        "document.getElementById('inventory-' + {slug} + '-status')?.dataset.state === {state}"
    ));
}

fn open_protocol(page: &mut Browser) {
    page.show("#agent");
    if page.eval("document.getElementById('protocol-advanced')?.open === true") != Value::Bool(true)
    {
        page.click("protocol-advanced-summary");
    }
    page.wait_until("document.getElementById('protocol-advanced')?.open === true");
}

fn seed_session(page: &mut Browser) -> String {
    page.eval(
        "(() => {
            const origin = location.origin;
            const key = 'a2a-lab.sessions.v1.' + origin;
            const value = JSON.stringify({schema:1, origin, sessions:[{contextId:'stored-context', createdAt:1, taskId:'stored-task', updatedAt:1}]});
            localStorage.setItem(key, value);
            return value;
        })()",
    )
    .as_str()
    .unwrap_or_default()
    .to_owned()
}

fn stored_session(page: &mut Browser) -> String {
    page.eval("localStorage.getItem('a2a-lab.sessions.v1.' + location.origin) ?? ''")
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn post_bodies(page: &Browser, base: &str) -> Vec<String> {
    page.requests(base)
        .into_iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .filter_map(|request| {
            request["params"]["request"]["postData"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

fn assert_inventory_requests(page: &Browser, base: &str) {
    let requests = page.requests(base);
    let sends: Vec<_> = requests
        .iter()
        .filter(|request| {
            request_url(request).contains("/message:send") && request_method(request) == "POST"
        })
        .collect();
    assert!(sends.len() >= 2);
    let expected = format!("Bearer {TOKEN}");
    let mut saw_auth = false;
    for request in &sends {
        match header(request, "authorization") {
            None => assert!(!saw_auth, "a later list call dropped the bearer token"),
            Some(auth) => {
                saw_auth = true;
                assert_eq!(auth, expected);
            }
        }
    }
    assert!(saw_auth);
    assert!(requests.iter().all(|request| {
        !request_url(request).contains(TOKEN) && !request_url(request).contains("/tasks?")
    }));
    let bodies = post_bodies(page, base);
    assert!(bodies.len() >= 2, "missing list request bodies");
    assert!(bodies.iter().all(|body| !body.contains("contextId")));
    assert!(bodies.iter().all(|body| !body.contains(TOKEN)));
    assert!(bodies.iter().all(|body| !body.contains("keep-me")));
    assert!(bodies.iter().any(|body| body.contains("\"cursor\":\"1\"")));
    for operation in [
        "list_log_sources",
        "list_metrics",
        "list_tasks",
        "list_image_sources",
    ] {
        assert!(
            bodies.iter().any(|body| body.contains(operation)),
            "{operation}"
        );
    }
}

async fn inventory_lab() -> String {
    let service = A2aLabService::new(
        inventory_logs().await,
        inventory_metrics().await,
        inventory_tasks().await,
    )
    .with_images(inventory_images().await)
    .share();
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
    format!("http://{address}")
}

async fn inventory_logs() -> MemoryLogs {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("log-a").unwrap(),
        name: "App".to_owned(),
        description: MARKUP.to_owned(),
        asset_id: Some("asset-log".to_owned()),
        semantic_id: Some("semantic-log".to_owned()),
    })
    .await;
    logs.insert_source(LogSource {
        id: SourceId::new("log-b").unwrap(),
        name: "Kernel".to_owned(),
        description: "Kernel log".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs
}

async fn inventory_metrics() -> MemoryMetrics {
    let metrics = MemoryMetrics::new();
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("metric-a").unwrap(),
            name: "Latency".to_owned(),
            description: "Round trip".to_owned(),
            unit: "ms".to_owned(),
            asset_id: Some("asset-metric".to_owned()),
            semantic_id: Some("semantic-metric".to_owned()),
        })
        .await;
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("metric-b").unwrap(),
            name: "Count".to_owned(),
            description: "Samples".to_owned(),
            unit: "count".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
}

async fn inventory_tasks() -> MemoryTasks {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("task-a").unwrap(),
            name: "Build".to_owned(),
            description: "Build a plate".to_owned(),
            asset_id: Some("asset-task".to_owned()),
            semantic_id: Some("semantic-task".to_owned()),
            input_schema: Some(r#"{"title":"<b>in</b>"}"#.to_owned()),
            output_schema: Some(r#"{"title":"<b>out</b>"}"#.to_owned()),
        })
        .await;
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("task-b").unwrap(),
            name: "Home".to_owned(),
            description: "Home the axes".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    tasks
}

async fn inventory_images() -> MemoryImages {
    let images = MemoryImages::new();
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new("image-a").unwrap(),
            name: "Camera".to_owned(),
            description: MARKUP.to_owned(),
            asset_id: Some("asset-image".to_owned()),
            semantic_id: Some("semantic-image".to_owned()),
        })
        .await
        .unwrap();
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new("image-b").unwrap(),
            name: "Deck".to_owned(),
            description: "Deck camera".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await
        .unwrap();
    images
}

async fn scripted_inventory() -> String {
    serve_mock(
        json!({
            "name": "inventory-agent",
            "description": "Lab",
            "version": "1",
            "supportedInterfaces": [{
                "url": "http://127.0.0.1",
                "protocolBinding": "HTTP+JSON",
                "protocolVersion": "1.0"
            }],
            "capabilities": {"streaming": false},
            "skills": [
                {"id": "list-log-sources", "name": "List log sources", "description": "List"},
                {"id": "list-metrics", "name": "List metrics", "description": "List"}
            ]
        }),
        false,
        false,
        super::MockRoute::Inventory,
        StatusCode::NOT_FOUND,
        Vec::new(),
    )
    .await
}
pub(super) async fn scripted_send(body: Bytes) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let data = &value["message"]["parts"][0]["data"];
    let operation = data["operation"].as_str().unwrap_or("");
    let cursor = data["params"]["page"]["cursor"].as_str().unwrap_or("");
    match operation {
        "list_log_sources" if cursor == "1" => bytes(
            StatusCode::OK,
            "application/json",
            listed(
                "list_log_sources",
                &json!([{
                    "id": "log-b",
                    "name": "Kernel",
                    "description": "Kernel log"
                }]),
                None,
            ),
        ),
        "list_log_sources" => bytes(
            StatusCode::OK,
            "application/json",
            listed(
                "list_log_sources",
                &json!([{
                    "id": "log-a",
                    "name": "App",
                    "description": "<img id=\"inventory-xss\" src=x onerror=\"window.__inventoryXss=1\">",
                    "asset_id": "asset-log",
                    "semantic_id": "semantic-log"
                }]),
                Some("1"),
            ),
        ),
        "list_metrics" => {
            tokio::time::sleep(Duration::from_millis(800)).await;
            bytes(
                StatusCode::OK,
                "application/json",
                listed("list_metrics", &json!([]), None),
            )
        }
        "list_image_sources" => bytes(
            StatusCode::INTERNAL_SERVER_ERROR,
            "application/json",
            json!({
                "error": {
                    "code": 500,
                    "status": "INTERNAL",
                    "message": "image provider is not configured",
                    "details": [{
                        "reason": "INTERNAL_ERROR",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        ),
        _ => bytes(StatusCode::OK, "application/json", b"not-json".to_vec()),
    }
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
