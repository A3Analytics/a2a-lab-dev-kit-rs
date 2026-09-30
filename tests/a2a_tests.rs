use std::sync::Arc;
use std::time::Duration;

use a2a_lab_sdk::{
    A2aClient, A2aServer, GetTaskStatusRequest, JsonObject, LAB_MEDIA_TYPE, LabApi, LabResult,
    LabService, ListLogSourcesRequest, ListMetricsRequest, ListTasksRequest, LogLevel, LogRecord,
    LogSource, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId, MetricPoint,
    PageRequest, QueryLogsRequest, QueryMetricRequest, SourceId, StartTaskRequest, TaskDefinition,
    TaskId, TaskState, TimeRange, UtcTimestamp, bind_local,
};

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
    service: Arc<dyn LabApi>,
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
        })
        .await;
    let service = LabService::new(logs.clone(), metrics, tasks.clone()).share();
    Lab {
        logs,
        tasks,
        service,
    }
}

async fn serve(service: Arc<dyn LabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

#[tokio::test]
async fn agent_card_advertises_every_skill() {
    let lab = lab().await;
    let base = serve(lab.service).await;
    let card = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let ids: Vec<_> = card["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["id"].as_str().unwrap())
        .collect();
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
    assert_eq!(card["protocolVersion"], "1.0");
    assert_eq!(
        card["supportedInterfaces"][0]["protocolBinding"],
        "HTTP+JSON"
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
        .json(&serde_json::json!({
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
    let body = raw.text().await.unwrap();
    assert!(body.contains("TASK_STATE_COMPLETED"), "{body}");
    assert!(body.contains(LAB_MEDIA_TYPE), "{body}");
    let task_body: serde_json::Value = serde_json::from_str(&body).unwrap();
    let task_id = task_body["id"].as_str().unwrap();
    let events = client.subscribe(task_id).await.unwrap();
    let messages: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.result {
            Some(LabResult::QueryLogs(page)) => {
                page.items().first().map(|record| record.message.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(messages, ["one", "two", "three"]);
    assert!(events.iter().any(|event| event.last_chunk));
    assert!(events.windows(2).all(|pair| {
        pair[0].event != "artifactUpdate"
            || pair[1].event != "artifactUpdate"
            || !pair[0].last_chunk
    }));
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
        .start_task(StartTaskRequest {
            task_id: TaskId::new("build").unwrap(),
            input: JsonObject::parse(r#"{"branch":"main"}"#).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(started.state, TaskState::Submitted);
    let LabResult::StartTask(run) = &started.result else {
        panic!("start result");
    };
    let run_id = run.id.clone();
    let stream = tokio::spawn({
        let client = A2aClient::new(&base).unwrap();
        let run_id = run_id.clone();
        async move { client.subscribe(run_id.as_str()).await.unwrap() }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    lab.tasks
        .transition(&run_id, TaskState::Completed, Some("built".to_owned()))
        .await
        .unwrap();
    let events = tokio::time::timeout(Duration::from_secs(2), stream)
        .await
        .unwrap()
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.state == Some(TaskState::Submitted))
    );
    assert!(
        events
            .iter()
            .any(|event| event.state == Some(TaskState::Completed))
    );
    let refreshed = client.task(run_id.as_str()).await.unwrap();
    assert_eq!(refreshed.state, TaskState::Completed);

    let status = client
        .task_status(GetTaskStatusRequest { id: run_id })
        .await
        .unwrap();
    assert_eq!(status.state, TaskState::Completed);
}

#[tokio::test]
async fn reports_invalid_missing_and_unavailable_requests() {
    let lab = lab().await;
    let base = serve(Arc::clone(&lab.service)).await;
    let client = A2aClient::new(&base).unwrap();
    let invalid = client
        .query_logs(
            serde_json::from_value(serde_json::json!({
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
}
