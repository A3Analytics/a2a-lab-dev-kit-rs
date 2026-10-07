use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2A_PROTOCOL_VERSION, A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult,
    A2aLabService, A2aServer, AgentCard, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, GetTaskStatusRequest, HttpAuthSecurityScheme,
    JsonObject, LAB_MEDIA_TYPE, ListLogSourcesRequest, ListMetricsRequest, ListTasksRequest,
    LogLevel, LogRecord, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor,
    MetricId, MetricPoint, PageRequest, QueryLogsRequest, QueryMetricRequest, RunId,
    SecurityScheme, SourceId, StartTaskRequest, StreamResponse, TaskDefinition, TaskId,
    TaskPushNotificationConfig, TaskState, TimeRange, UtcTimestamp, bind_local,
};
use axum::{Json, Router, routing::post};
use serde_json::json;

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

struct Lab {
    logs: MemoryLogs,
    tasks: MemoryTasks,
    service: Arc<dyn A2aLabApi>,
}

async fn lab() -> Lab {
    let logs = MemoryLogs::new();
    let metrics = MemoryMetrics::new();
    let tasks = MemoryTasks::new();
    let source_id = SourceId::new("app").unwrap();
    logs.insert_source(LogSource {
        id: source_id.clone(),
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_source(LogSource {
        id: SourceId::new("worker").unwrap(),
        name: "Worker".to_owned(),
        description: "Worker logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    for (stamp, message) in [
        ("2024-01-01T00:00:00Z", "one"),
        ("2024-01-01T00:01:00Z", "two"),
        ("2024-01-01T00:02:00Z", "three"),
    ] {
        logs.insert_record(LogRecord {
            source_id: source_id.clone(),
            timestamp: timestamp(stamp),
            level: LogLevel::Info,
            message: message.to_owned(),
            attributes: JsonObject::empty(),
        })
        .await
        .unwrap();
    }
    let metric_id = MetricId::new("cpu").unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: "CPU".to_owned(),
            description: "CPU load".to_owned(),
            unit: "percent".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            metric_id,
            MetricPoint::new(timestamp("2024-01-01T00:00:00Z"), 0.5).unwrap(),
        )
        .await
        .unwrap();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(logs.clone(), metrics, tasks.clone()).share();
    Lab {
        logs,
        tasks,
        service,
    }
}

async fn serve(service: Arc<dyn A2aLabApi>) -> String {
    serve_with(service, |server| server).await
}

async fn serve_with(
    service: Arc<dyn A2aLabApi>,
    configure: impl FnOnce(A2aServer) -> A2aServer + Send + 'static,
) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = configure(A2aServer::new(&service));
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn lab_results(events: &[StreamResponse]) -> Vec<A2aLabResult> {
    events
        .iter()
        .filter_map(|event| {
            let StreamResponse::ArtifactUpdate(update) = event else {
                return None;
            };
            let value = serde_json::to_value(&update.artifact).ok()?;
            value["parts"]
                .as_array()?
                .iter()
                .find_map(|part| serde_json::from_value(part.get("data")?.clone()).ok())
        })
        .collect()
}

fn protocol_state(event: &StreamResponse) -> Option<String> {
    let state = match event {
        StreamResponse::Task(task) => &task.status.state,
        StreamResponse::StatusUpdate(update) => &update.status.state,
        _ => return None,
    };
    serde_json::to_value(state)
        .ok()?
        .as_str()
        .map(ToOwned::to_owned)
}

fn last_chunks(events: &[StreamResponse]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamResponse::ArtifactUpdate(update) => update.last_chunk,
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn agent_card_advertises_every_skill() {
    let lab = lab().await;
    let base = serve(lab.service).await;
    let client = A2aClient::new(&base).unwrap();
    let card = client.agent_card().await.unwrap();
    let ids: Vec<_> = card.skills.iter().map(|skill| skill.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "list-log-sources",
            "query-logs",
            "list-metrics",
            "query-metric",
            "list-tasks",
            "start-task",
            "get-task-status",
        ]
    );
    let bindings: Vec<_> = card
        .supported_interfaces
        .iter()
        .map(|interface| interface.protocol_binding.as_str())
        .collect();
    assert_eq!(bindings, ["HTTP+JSON", "JSONRPC", "GRPC"]);
    assert!(
        card.supported_interfaces
            .iter()
            .all(|interface| { interface.protocol_version == A2A_PROTOCOL_VERSION })
    );
    assert!(card.supported_interfaces[0].url.starts_with("http://"));
    assert_eq!(
        card.supported_interfaces[0].url,
        card.supported_interfaces[1].url
    );
    assert!(!card.supported_interfaces[2].url.contains("://"));
    assert!(card.supported_interfaces[2].url.contains(':'));
    assert_eq!(card.capabilities.streaming, Some(true));
    assert_eq!(card.capabilities.push_notifications, Some(false));
    assert_eq!(card.capabilities.extended_agent_card, Some(false));
    assert!(
        card.default_input_modes
            .contains(&LAB_MEDIA_TYPE.to_owned())
    );
}

#[tokio::test]
async fn exercises_every_operation_failure_and_stream() {
    let lab = lab().await;
    let base = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&base).unwrap();

    let sources = client
        .list_log_sources(ListLogSourcesRequest { page: page(1) })
        .await
        .unwrap();
    assert_eq!(sources.items()[0].id.as_str(), "app");
    assert!(sources.next_cursor().is_some());

    let logs = client
        .query_logs(QueryLogsRequest {
            source_id: SourceId::new("app").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert_eq!(logs.items().len(), 3);

    let raw = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": "wire-1",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": LAB_MEDIA_TYPE,
                    "data": {
                        "operation": "query_logs",
                        "params": {
                            "source_id": "app",
                            "range": {
                                "start": "2024-01-01T00:00:00Z",
                                "end": "2024-01-01T01:00:00Z"
                            },
                            "page": {"limit": 10}
                        }
                    }
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        raw.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or(""),
        "application/a2a+json"
    );
    let body = raw.text().await.unwrap();
    assert!(body.contains("TASK_STATE_COMPLETED"), "{body}");
    assert!(body.contains(LAB_MEDIA_TYPE), "{body}");
    let task_body: serde_json::Value = serde_json::from_str(&body).unwrap();
    let task = task_body.get("task").unwrap_or(&task_body);
    assert!(task["history"].as_array().is_some());

    let events = client
        .send_stream(A2aLabCommand::QueryLogs(QueryLogsRequest {
            source_id: SourceId::new("app").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        }))
        .await
        .unwrap();
    let messages: Vec<_> = lab_results(&events)
        .into_iter()
        .filter_map(|result| match result {
            A2aLabResult::QueryLogs(page) => {
                page.items().first().map(|record| record.message.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(messages, ["one", "two", "three"]);
    let chunks = last_chunks(&events);
    assert_eq!(chunks.last().copied(), Some(true));
    assert!(
        chunks
            .iter()
            .take(chunks.len().saturating_sub(1))
            .all(|last| !last)
    );
}

#[tokio::test]
async fn reads_metrics_and_task_transitions() {
    let lab = lab().await;
    let base = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&base).unwrap();
    let metrics = client
        .list_metrics(ListMetricsRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(metrics.items()[0].unit, "percent");
    let points = client
        .query_metric(QueryMetricRequest {
            metric_id: MetricId::new("cpu").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert!((points.items()[0].value - 0.5).abs() < f64::EPSILON);

    let tasks = client
        .list_tasks(ListTasksRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(tasks.items()[0].id.as_str(), "build");
    let started = client
        .start_task(
            StartTaskRequest::new(
                TaskId::new("build").unwrap(),
                JsonObject::parse(r#"{"branch":"main"}"#).unwrap(),
            )
            .immediate(),
        )
        .await
        .unwrap();
    assert_eq!(started.state, TaskState::Submitted);
    let A2aLabResult::StartTask(run) = &started.result else {
        panic!("start result");
    };
    let run_id = run.id.clone();
    let a2a_id = started.id.clone();
    let listed = client
        .list_a2a_tasks(Some(&started.context_id))
        .await
        .unwrap();
    assert!(listed.tasks.iter().any(|task| task.id == a2a_id));
    let stored = client.get_a2a_task(&a2a_id).await.unwrap();
    assert!(stored.history.is_some());
    let stream_a = tokio::spawn({
        let client = A2aClient::new(&base).unwrap();
        let a2a_id = a2a_id.clone();
        async move { client.subscribe(&a2a_id).await.unwrap() }
    });
    let stream_b = tokio::spawn({
        let client = A2aClient::new(&base).unwrap();
        let a2a_id = a2a_id.clone();
        async move { client.subscribe(&a2a_id).await.unwrap() }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    lab.tasks
        .transition(&run_id, TaskState::Completed, Some("built".to_owned()))
        .await
        .unwrap();
    let events_a = tokio::time::timeout(Duration::from_secs(2), stream_a)
        .await
        .unwrap()
        .unwrap();
    let events_b = tokio::time::timeout(Duration::from_secs(2), stream_b)
        .await
        .unwrap()
        .unwrap();
    for events in [&events_a, &events_b] {
        assert!(
            events
                .iter()
                .any(|event| protocol_state(event).as_deref() == Some("TASK_STATE_SUBMITTED"))
        );
        assert!(
            events
                .iter()
                .any(|event| protocol_state(event).as_deref() == Some("TASK_STATE_COMPLETED"))
        );
    }
    let refreshed = client.task(&a2a_id).await.unwrap();
    assert_eq!(refreshed.state, TaskState::Completed);

    let status = client
        .task_status(GetTaskStatusRequest { id: run_id })
        .await
        .unwrap();
    assert_eq!(status.state, TaskState::Completed);
}

#[tokio::test]
async fn reports_invalid_missing_unavailable_and_unsupported_content() {
    let lab = lab().await;
    let base = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&base).unwrap();
    let invalid = client
        .query_logs(
            serde_json::from_value(json!({
                "source_id": "app",
                "range": {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:00Z"},
                "page": {"limit": 10}
            }))
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), "invalid");
    let missing = client
        .query_logs(QueryLogsRequest {
            source_id: SourceId::new("missing").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
    lab.logs.set_unavailable("logs offline").await;
    let unavailable = client
        .list_log_sources(ListLogSourcesRequest { page: page(10) })
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");

    let rejected = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": "bad-mode",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": "application/xml",
                    "data": {"operation": "list_log_sources", "params": {"page": {"limit": 10}}}
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        rejected.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let error = rejected.text().await.unwrap();
    assert!(error.contains("error"), "{error}");

    let version = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "0.3")
        .json(&json!({
            "message": {
                "messageId": "old",
                "role": "ROLE_USER",
                "parts": [{"text": "hello"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(version.status().is_client_error());
}

#[tokio::test]
async fn cancels_running_lab_task_and_rejects_terminal_cancel() {
    let lab = lab().await;
    let base = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&base).unwrap();
    let started = client
        .start_task(
            StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty()).immediate(),
        )
        .await
        .unwrap();
    let canceled = client.cancel(&started.id).await.unwrap();
    assert_eq!(
        serde_json::to_value(&canceled.status.state).unwrap(),
        "TASK_STATE_CANCELED"
    );
    let finished = client
        .list_log_sources(ListLogSourcesRequest { page: page(10) })
        .await
        .unwrap();
    let snapshot = client
        .list_a2a_tasks(None)
        .await
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| {
            task.artifacts.as_ref().is_some_and(|artifacts| {
                artifacts.iter().any(|artifact| {
                    serde_json::to_value(artifact).ok().and_then(|value| {
                        value["parts"][0]["data"]["operation"]
                            .as_str()
                            .map(ToOwned::to_owned)
                    }) == Some("list_log_sources".to_owned())
                })
            })
        })
        .unwrap();
    let error = client.cancel(&snapshot.id).await.unwrap_err();
    assert_eq!(error.code(), "protocol");
    let terminal = reqwest::Client::new()
        .post(format!("{base}/tasks/{}:cancel", snapshot.id))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(terminal.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(finished.items().len(), 2);
}

fn extra_card() -> AgentCard {
    serde_json::from_value(json!({
        "name": "a2a-lab-extended",
        "description": "Authenticated card",
        "version": "0.1.0",
        "supportedInterfaces": [{
            "url": "http://127.0.0.1:1",
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": "1.0"
        }],
        "capabilities": {
            "streaming": true,
            "pushNotifications": true,
            "extendedAgentCard": true
        },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    }))
    .unwrap()
}

struct HookCall {
    auth: Option<String>,
    token: Option<String>,
}

#[tokio::test]
async fn disabled_optional_capabilities_return_unsupported() {
    let lab = lab().await;
    let disabled = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&disabled).unwrap();
    let started = client
        .start_task(
            StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty()).immediate(),
        )
        .await
        .unwrap();
    let push_error = client
        .create_push_config(TaskPushNotificationConfig {
            url: "http://127.0.0.1:9/hook".to_owned(),
            id: Some("cfg".to_owned()),
            task_id: started.id.clone(),
            token: None,
            authentication: None,
            tenant: None,
        })
        .await
        .unwrap_err();
    assert_eq!(push_error.code(), "protocol");
    assert_eq!(
        client.extended_agent_card().await.unwrap_err().code(),
        "protocol"
    );
}

#[tokio::test]
async fn extended_card_and_security_are_advertised_when_configured() {
    let lab = lab().await;
    let extra = extra_card();
    let enabled = serve_with(Arc::clone(&lab.service), {
        let extra = extra.clone();
        move |server| {
            server.with_extended_card(extra).with_security(
                [(
                    "bearerAuth".to_owned(),
                    SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                        scheme: "bearer".to_owned(),
                        description: None,
                        bearer_format: Some("JWT".to_owned()),
                    }),
                )]
                .into(),
                vec![[("bearerAuth".to_owned(), Vec::new())].into()],
            )
        }
    })
    .await;
    let anonymous = A2aClient::new(&enabled).unwrap();
    assert_eq!(
        anonymous.extended_agent_card().await.unwrap_err().code(),
        "invalid"
    );
    let client = anonymous.with_bearer_token("lab-token");
    let card = client.agent_card().await.unwrap();
    assert_eq!(card.capabilities.extended_agent_card, Some(true));
    assert!(card.security_schemes.is_some());
    assert_eq!(
        client.extended_agent_card().await.unwrap().name,
        "a2a-lab-extended"
    );
}

#[tokio::test]
async fn push_webhook_delivers_authenticated_stream_payloads() {
    let lab = lab().await;
    let hooks: Arc<Mutex<Vec<HookCall>>> = Arc::new(Mutex::new(Vec::new()));
    let (hook_listener, hook_address) = bind_local().await.unwrap();
    let hook_state = Arc::clone(&hooks);
    tokio::spawn(async move {
        let app = Router::new().route(
            "/",
            post({
                let hook_state = Arc::clone(&hook_state);
                move |headers: axum::http::HeaderMap, Json(_body): Json<serde_json::Value>| {
                    let hook_state = Arc::clone(&hook_state);
                    async move {
                        hook_state.lock().unwrap().push(HookCall {
                            auth: headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(ToOwned::to_owned),
                            token: headers
                                .get("a2a-notification-token")
                                .and_then(|value| value.to_str().ok())
                                .map(ToOwned::to_owned),
                        });
                        Json(json!({"ok": true}))
                    }
                }
            }),
        );
        axum::serve(hook_listener, app).await.unwrap();
    });
    let enabled = serve_with(Arc::clone(&lab.service), |server| {
        server.with_loopback_push()
    })
    .await;
    let client = A2aClient::new(&enabled).unwrap();
    assert_eq!(
        client
            .agent_card()
            .await
            .unwrap()
            .capabilities
            .push_notifications,
        Some(true)
    );
    let running = client
        .start_task(
            StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty()).immediate(),
        )
        .await
        .unwrap();
    let created = client
        .create_push_config(
            serde_json::from_value(json!({
                "url": format!("http://{hook_address}/"),
                "id": "hook-1",
                "taskId": running.id,
                "token": "notice",
                "authentication": {"scheme": "Bearer", "credentials": "secret"}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.id.as_deref(), Some("hook-1"));
    assert_eq!(
        client
            .list_push_configs(&running.id)
            .await
            .unwrap()
            .configs
            .len(),
        1
    );
    let fetched = client.get_push_config(&running.id, "hook-1").await.unwrap();
    assert_eq!(fetched.url, format!("http://{hook_address}/"));
    lab.tasks
        .transition(
            match &running.result {
                A2aLabResult::StartTask(run) => &run.id,
                _ => panic!("run"),
            },
            TaskState::Completed,
            Some("done".to_owned()),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ok = hooks.lock().unwrap().iter().any(|call| {
        call.auth.as_deref() == Some("Bearer secret") && call.token.as_deref() == Some("notice")
    });
    assert!(ok);
    client
        .delete_push_config(&running.id, "hook-1")
        .await
        .unwrap();
}

#[test]
fn omitted_wait_deserializes_as_wait() {
    let request: StartTaskRequest =
        serde_json::from_str(r#"{"task_id":"build","input":{}}"#).unwrap();
    assert!(request.wait);
    assert_eq!(request.timeout_seconds, None);
}

#[tokio::test]
async fn start_task_returns_immediately_when_wait_is_false() {
    let lab = lab().await;
    let outcome = lab
        .service
        .execute(A2aLabCommand::StartTask(
            StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty()).immediate(),
        ))
        .await
        .unwrap();
    assert_eq!(outcome.task.state, TaskState::Submitted);
}

#[tokio::test]
async fn start_task_wait_returns_terminal_state() {
    let lab = lab().await;
    let tasks = lab.tasks.clone();
    tokio::spawn(async move {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if tasks
                .transition(
                    &RunId::new("run-1").unwrap(),
                    TaskState::Completed,
                    Some("done".to_owned()),
                )
                .await
                .is_ok()
            {
                break;
            }
        }
    });
    let mut request = StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty());
    request.timeout_seconds = Some(2);
    let outcome = lab
        .service
        .execute(A2aLabCommand::StartTask(request))
        .await
        .unwrap();
    assert_eq!(outcome.task.state, TaskState::Completed);
}

struct ScriptedMessages {
    calls: Mutex<Vec<AgentMessageRequest>>,
    fail: bool,
}

impl AgentMessageHandler for ScriptedMessages {
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>> {
        let fail = self.fail;
        let text = request.text.clone();
        self.calls.lock().expect("calls").push(request);
        Box::pin(async move {
            if fail {
                Err(A2aLabError::unavailable("model offline"))
            } else {
                Ok(AgentMessageReply {
                    text: format!("echo {text}"),
                })
            }
        })
    }
}

#[tokio::test]
async fn agent_message_continues_context_and_keeps_lab_commands() {
    let lab = lab().await;
    let handler = Arc::new(ScriptedMessages {
        calls: Mutex::new(Vec::new()),
        fail: false,
    });
    let base = serve_with(Arc::clone(&lab.service), {
        let handler = Arc::clone(&handler);
        move |server| server.with_message_handler(handler)
    })
    .await;
    let client = A2aClient::new(&base).unwrap();
    let ids: Vec<_> = client
        .agent_card()
        .await
        .unwrap()
        .skills
        .into_iter()
        .map(|skill| skill.id)
        .collect();
    assert_eq!(ids.last().map(String::as_str), Some("agent-message"));
    assert_eq!(ids.len(), 8);

    let first = client.agent_message("hello", None).await.unwrap();
    assert_eq!(first.text, "echo hello");
    let first_task = first.task_id.clone().unwrap();
    let first_record = client.get_a2a_task(&first_task).await.unwrap();
    let first_message = &first_record.history.unwrap()[0].message_id;
    assert!(is_sender_message_id(first_message), "{first_message}");

    let second = client
        .agent_message_with_references("again", Some(&first.context_id), &[first_task.as_str()])
        .await
        .unwrap();
    assert_eq!(second.text, "echo again");
    assert_eq!(second.context_id, first.context_id);
    assert_ne!(second.task_id.as_deref(), Some(first_task.as_str()));

    let calls = handler.calls.lock().expect("calls").clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].context_id, first.context_id);
    assert_eq!(calls[1].context_id, first.context_id);
    assert_eq!(calls[0].task_id, first_task);
    assert_eq!(calls[1].task_id, second.task_id.clone().unwrap());
    assert!(calls[0].reference_task_ids.is_empty());
    assert_eq!(calls[1].reference_task_ids, vec![first_task]);

    client
        .list_tasks(ListTasksRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(handler.calls.lock().expect("calls").len(), 2);
}

#[tokio::test]
async fn agent_message_maps_handler_errors() {
    let lab = lab().await;
    let handler = Arc::new(ScriptedMessages {
        calls: Mutex::new(Vec::new()),
        fail: true,
    });
    let base = serve_with(Arc::clone(&lab.service), move |server| {
        server.with_message_handler(handler)
    })
    .await;
    let error = A2aClient::new(&base)
        .unwrap()
        .agent_message("hello", None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unavailable");
}

fn is_sender_message_id(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    parts.len() == 5
        && [
            parts[0].len(),
            parts[1].len(),
            parts[2].len(),
            parts[3].len(),
            parts[4].len(),
        ] == [8, 4, 4, 4, 12]
        && value
            .chars()
            .all(|character| character == '-' || character.is_ascii_hexdigit())
}

#[tokio::test]
async fn agent_message_rejects_non_user_and_non_text_parts() {
    let lab = lab().await;
    let base = serve_with(Arc::clone(&lab.service), |server| {
        server.with_message_handler(Arc::new(ScriptedMessages {
            calls: Mutex::new(Vec::new()),
            fail: false,
        }))
    })
    .await;
    let role = send_message(
        &base,
        json!({
            "message": {
                "messageId": "role-1",
                "role": "ROLE_AGENT",
                "parts": [{"text": "hello", "mediaType": "text/plain"}]
            }
        }),
    )
    .await;
    assert_eq!(role.status(), reqwest::StatusCode::BAD_REQUEST);
    let mixed = send_message(
        &base,
        json!({
            "message": {
                "messageId": "mixed-1",
                "role": "ROLE_USER",
                "parts": [
                    {"text": "hello", "mediaType": "text/plain"},
                    {"url": "https://example.com/note"}
                ]
            }
        }),
    )
    .await;
    assert_eq!(mixed.status(), reqwest::StatusCode::BAD_REQUEST);
}

async fn send_message(base: &str, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", A2A_PROTOCOL_VERSION)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn security_requirement_hides_another_callers_context() {
    let lab = lab().await;
    let base = serve_with(Arc::clone(&lab.service), |server| {
        server
            .with_message_handler(Arc::new(ScriptedMessages {
                calls: Mutex::new(Vec::new()),
                fail: false,
            }))
            .with_security(
                [(
                    "bearerAuth".to_owned(),
                    SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                        scheme: "bearer".to_owned(),
                        description: None,
                        bearer_format: None,
                    }),
                )]
                .into(),
                vec![[("bearerAuth".to_owned(), Vec::new())].into()],
            )
    })
    .await;
    let anonymous = A2aClient::new(&base).unwrap();
    assert_eq!(
        anonymous
            .agent_message("hello", None)
            .await
            .unwrap_err()
            .code(),
        "invalid"
    );
    let alice = A2aClient::new(&base).unwrap().with_bearer_token("alice");
    let bob = A2aClient::new(&base).unwrap().with_bearer_token("bob");
    let first = alice.agent_message("hello", None).await.unwrap();
    let hidden = bob
        .agent_message("nope", Some(&first.context_id))
        .await
        .unwrap_err();
    assert_eq!(hidden.code(), "invalid");
    assert_eq!(
        bob.get_a2a_task(first.task_id.as_deref().unwrap())
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    let listed = bob.list_a2a_tasks(None).await.unwrap();
    let first_task = first.task_id.clone().unwrap();
    assert!(listed.tasks.iter().all(|task| task.id != first_task));
    let again = alice
        .agent_message("yes", Some(&first.context_id))
        .await
        .unwrap();
    assert_eq!(again.context_id, first.context_id);
}

#[tokio::test]
async fn start_task_wait_times_out() {
    let lab = lab().await;
    let mut request = StartTaskRequest::new(TaskId::new("build").unwrap(), JsonObject::empty());
    request.timeout_seconds = Some(1);
    let error = lab
        .service
        .execute(A2aLabCommand::StartTask(request))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unavailable");
}
