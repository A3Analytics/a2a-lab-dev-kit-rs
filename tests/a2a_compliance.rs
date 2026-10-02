use std::time::Duration;

use a2a_client::A2AClient;
use a2a_grpc::GrpcTransport;
use a2a_lab_dev_kit::{
    A2A_PROTOCOL_VERSION, A2aServer, LabService, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks,
    SourceId, TaskDefinition, TaskId, bind_local,
};
use a2a_types::{
    GetTaskRequest, Message, Part, PartContent, Role, SendMessageRequest, SendMessageResponse,
    StreamResponse, TaskState, error_code,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

async fn serve() -> String {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app").unwrap(),
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let service = LabService::new(logs, MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&service).listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn message(id: &str, text: &str) -> Value {
    json!({
        "message": {
            "messageId": id,
            "role": "ROLE_USER",
            "parts": [{"text": text}]
        }
    })
}

async fn send(base: &str, id: &str) -> reqwest::Response {
    http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&message(id, "hello"))
        .send()
        .await
        .unwrap()
}

fn reason(body: &Value) -> Option<&str> {
    body["error"]["details"]
        .as_array()?
        .iter()
        .find_map(|detail| detail["reason"].as_str())
}

fn domain(body: &Value) -> Option<&str> {
    body["error"]["details"]
        .as_array()?
        .iter()
        .find_map(|detail| detail["domain"].as_str())
}

fn sse_values(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).expect("sse json"))
        .collect()
}

async fn grpc(base: &str) -> A2AClient<GrpcTransport> {
    let card: Value = http()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let url = card["supportedInterfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|interface| interface["protocolBinding"] == "GRPC")
        .unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut last = None;
    for _ in 0..20 {
        match GrpcTransport::connect(&url).await {
            Ok(transport) => return A2AClient::new(transport),
            Err(error) => {
                last = Some(error);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    panic!("gRPC connect failed: {last:?}");
}

fn grpc_message(id: &str, text: &str) -> SendMessageRequest {
    let mut message = Message::new(Role::User, vec![Part::text(text)]);
    id.clone_into(&mut message.message_id);
    SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

#[tokio::test]
async fn http_json_agent_card_advertises_three_bindings() {
    let base = serve().await;
    let response = http()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let card: Value = response.json().await.unwrap();
    assert_eq!(card["name"], "a2a-lab");
    assert_eq!(card["capabilities"]["streaming"], true);
    let interfaces = card["supportedInterfaces"].as_array().unwrap();
    let bindings: Vec<_> = interfaces
        .iter()
        .map(|interface| interface["protocolBinding"].as_str().unwrap())
        .collect();
    assert_eq!(bindings, ["HTTP+JSON", "JSONRPC", "GRPC"]);
    assert!(
        interfaces
            .iter()
            .all(|interface| { interface["protocolVersion"] == A2A_PROTOCOL_VERSION })
    );
    assert!(
        interfaces[0]["url"]
            .as_str()
            .unwrap()
            .starts_with("http://")
    );
    assert_eq!(interfaces[0]["url"], interfaces[1]["url"]);
    let grpc_url = interfaces[2]["url"].as_str().unwrap();
    assert!(!grpc_url.contains("://"));
    assert!(grpc_url.contains(':'));
}

#[tokio::test]
async fn http_json_svc_001_success_content_type_is_a2a_json() {
    let base = serve().await;
    let response = send(&base, "content-type").await;
    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("application/a2a+json"));
}

#[tokio::test]
async fn http_json_url_001_and_002_routes_and_methods() {
    let base = serve().await;
    let sent = send(&base, "routes").await;
    assert!(sent.status().is_success());
    let task: Value = sent.json().await.unwrap();
    let id = task["task"]["id"].as_str().unwrap();

    let loaded = http()
        .get(format!("{base}/tasks/{id}"))
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .send()
        .await
        .unwrap();
    assert!(loaded.status().is_success());

    let listed = http()
        .get(format!("{base}/tasks?pageSize=1"))
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .send()
        .await
        .unwrap();
    assert!(listed.status().is_success());

    let missing = http()
        .post(format!("{base}/tasks/missing-route:cancel"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(matches!(missing.status().as_u16(), 404 | 409));
}

#[tokio::test]
async fn http_json_qp_001_page_size_is_camel_case() {
    let base = serve().await;
    let response = http()
        .get(format!("{base}/tasks?pageSize=1"))
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
}

#[tokio::test]
async fn http_json_svc_002_version_header_and_query() {
    let base = serve().await;
    let missing = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .json(&message("no-version", "hello"))
        .send()
        .await
        .unwrap();
    assert!(missing.status().is_success());

    let query = http()
        .post(format!("{base}/message:send?A2A-Version=1.0"))
        .header("Content-Type", "application/a2a+json")
        .json(&message("query-version", "hello"))
        .send()
        .await
        .unwrap();
    assert!(query.status().is_success());

    let rejected = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "99.0")
        .json(&message("bad-version", "hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = rejected.json().await.unwrap();
    assert_eq!(reason(&body), Some("VERSION_NOT_SUPPORTED"));
}

#[tokio::test]
async fn http_json_err_001_and_002_task_not_found_is_aip_193() {
    let base = serve().await;
    let response = http()
        .get(format!("{base}/tasks/missing-task"))
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["code"].is_number());
    assert!(body["error"]["message"].is_string());
    assert_eq!(reason(&body), Some("TASK_NOT_FOUND"));
    assert_eq!(domain(&body), Some("a2a-protocol.org"));
}

#[tokio::test]
async fn http_json_status_001_error_status_mapping() {
    let base = serve().await;
    let cancel = http()
        .post(format!("{base}/tasks/missing-cancel:cancel"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(matches!(cancel.status().as_u16(), 404 | 409));

    let part = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&json!({
            "message": {
                "messageId": "bad-part",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": "application/xml",
                    "data": {"operation": "list_log_sources", "params": {}}
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(part.status(), reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let part_body: Value = part.json().await.unwrap();
    assert_eq!(reason(&part_body), Some("CONTENT_TYPE_NOT_SUPPORTED"));

    let push = http()
        .post(format!("{base}/tasks/missing/pushNotificationConfigs"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&json!({"url": "http://example.com/callback"}))
        .send()
        .await
        .unwrap();
    assert_eq!(push.status(), reqwest::StatusCode::BAD_REQUEST);
    let push_body: Value = push.json().await.unwrap();
    assert_eq!(reason(&push_body), Some("PUSH_NOTIFICATION_NOT_SUPPORTED"));

    let plain = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "text/plain")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .body(message("plain", "hello").to_string())
        .send()
        .await
        .unwrap();
    assert!(plain.status().is_client_error());
    let plain_body: Value = plain.json().await.unwrap();
    assert!(plain_body["error"]["message"].is_string());
    assert_eq!(reason(&plain_body), Some("CONTENT_TYPE_NOT_SUPPORTED"));
    assert_eq!(domain(&plain_body), Some("a2a-protocol.org"));
}

#[tokio::test]
async fn http_json_sse_001_and_tck_profiles() {
    let base = serve().await;
    let response = http()
        .post(format!("{base}/message:stream"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&message("tck-artifact-text-1", "hello"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let events = sse_values(&response.text().await.unwrap());
    assert!(events.iter().any(|event| {
        event.get("artifactUpdate").is_some()
            || event.get("statusUpdate").is_some()
            || event.get("message").is_some()
            || event.get("task").is_some()
    }));
    let body = serde_json::to_string(&events).unwrap();
    assert!(body.contains("Generated text content"));
    assert!(body.contains("TASK_STATE_COMPLETED"));
    assert!(events.iter().any(|event| {
        event
            .pointer("/artifactUpdate/lastChunk")
            .and_then(Value::as_bool)
            == Some(true)
    }));

    for (id, marker) in [
        ("tck-message-response-1", "Direct message response"),
        ("tck-artifact-data-1", "\"count\":42"),
        ("tck-artifact-file-url-1", "https://example.com/output.txt"),
        ("tck-artifact-file-1", "output.txt"),
    ] {
        let response = http()
            .post(format!("{base}/message:stream"))
            .header("Content-Type", "application/a2a+json")
            .header("A2A-Version", A2A_PROTOCOL_VERSION)
            .json(&message(id, "hello"))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        assert!(text.contains(marker), "{id}: {text}");
    }
}

async fn rpc(base: &str, body: Value) -> reqwest::Response {
    http()
        .post(format!("{base}/"))
        .header("Content-Type", "application/json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn jsonrpc_send_message_returns_result() {
    let base = serve().await;
    let response = rpc(
        &base,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "SendMessage",
            "params": message("rpc-send", "hello")
        }),
    )
    .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 7);
    assert!(body.get("result").is_some(), "{body}");
}

#[tokio::test]
async fn jsonrpc_unknown_method_and_parse_error() {
    let base = serve().await;
    let unknown = rpc(
        &base,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "NoSuchMethod",
            "params": {}
        }),
    )
    .await;
    assert_eq!(unknown.status(), reqwest::StatusCode::OK);
    let unknown_body: Value = unknown.json().await.unwrap();
    assert_eq!(unknown_body["error"]["code"], error_code::METHOD_NOT_FOUND);

    let parsed = http()
        .post(format!("{base}/"))
        .header("Content-Type", "application/json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(parsed.status(), reqwest::StatusCode::OK);
    let parsed_body: Value = parsed.json().await.unwrap();
    assert_eq!(parsed_body["error"]["code"], error_code::PARSE_ERROR);
}

#[tokio::test]
async fn jsonrpc_get_task_unknown_id() {
    let base = serve().await;
    let response = rpc(
        &base,
        json!({
            "jsonrpc": "2.0",
            "id": "task",
            "method": "GetTask",
            "params": {"id": "missing-rpc"}
        }),
    )
    .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], error_code::TASK_NOT_FOUND);
    let data = serde_json::to_string(&body["error"]["data"]).unwrap();
    assert!(data.contains("TASK_NOT_FOUND"));
    assert!(data.contains("a2a-protocol.org"));
}

#[tokio::test]
async fn jsonrpc_send_streaming_message_is_sse() {
    let base = serve().await;
    let response = http()
        .post(format!("{base}/"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "SendStreamingMessage",
            "params": message("tck-artifact-text-rpc", "hello")
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let text = response.text().await.unwrap();
    assert!(text.contains("Generated text content"), "{text}");
}

#[tokio::test]
async fn grpc_send_message_returns_task() {
    let base = serve().await;
    let client = grpc(&base).await;
    let response = client
        .send_message(&grpc_message("grpc-send", "hello"))
        .await
        .unwrap();
    assert!(matches!(response, SendMessageResponse::Task(_)));
}

#[tokio::test]
async fn grpc_get_task_unknown_id_is_not_found() {
    let base = serve().await;
    let client = grpc(&base).await;
    let error = client
        .get_task(&GetTaskRequest {
            id: "missing-grpc".to_owned(),
            history_length: None,
            tenant: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, error_code::TASK_NOT_FOUND);
}

#[tokio::test]
async fn grpc_send_streaming_message_artifact_text() {
    let base = serve().await;
    let client = grpc(&base).await;
    let mut stream = client
        .send_streaming_message(&grpc_message("tck-artifact-text-grpc", "hello"))
        .await
        .unwrap();
    let mut saw_text = false;
    let mut saw_last = false;
    let mut saw_completed = false;
    while let Some(item) = stream.next().await {
        match item.unwrap() {
            StreamResponse::ArtifactUpdate(update) => {
                saw_last |= update.last_chunk == Some(true);
                saw_text |= update.artifact.parts.iter().any(|part| {
                    matches!(&part.content, PartContent::Text(text) if text == "Generated text content")
                });
            }
            StreamResponse::StatusUpdate(update) => {
                saw_completed |= update.status.state == TaskState::Completed;
            }
            StreamResponse::Task(_) | StreamResponse::Message(_) => {}
        }
    }
    assert!(saw_text);
    assert!(saw_last);
    assert!(saw_completed);
}
