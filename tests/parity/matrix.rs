use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabCommand, A2aLabResult, A2aLabService, A2aServer, AgentCard,
    GetCurrentImageRequest, GetImageRequest, GetTaskStatusRequest, HttpAuthSecurityScheme, Image,
    ImageDescriptor, ImageId, ImageSource, ImageSourceId, JsonObject, ListImageSourcesRequest,
    ListImagesRequest, ListLogSourcesRequest, ListMetricsRequest, ListTasksRequest, LogLevel,
    LogRecord, LogSource, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor,
    MetricId, MetricPoint, PageRequest, QueryLogsRequest, QueryMetricRequest, RunId,
    SearchImagesRequest, SecurityScheme, SourceId, StartTaskRequest, TaskDefinition, TaskId,
    TaskState, TimeRange, UtcTimestamp, bind_local,
};
use a2a_types::SecurityRequirement;
use base64::Engine;
use serde_json::{Value, json};

use super::{
    ALICE, BOB, END, Echo, Failure, Gate, Lab, MARKUP, NOSCOPE, OPERATIONS, PNG, PREVIEW_LIMIT,
    START, Success,
};

pub(crate) fn assert_covers_commands(cases: &[Success]) {
    for operation in OPERATIONS {
        assert!(
            cases.iter().any(|case| case.operation == operation),
            "{operation}"
        );
    }
}

pub(crate) async fn success_cases(client: &A2aClient) -> Vec<Success> {
    let mut cases = catalog_cases(client).await;
    cases.extend(run_and_image_cases(client).await);
    cases.extend(page_cases(client).await);
    cases
}

async fn catalog_cases(client: &A2aClient) -> Vec<Success> {
    let mut cases = Vec::new();
    cases.push(
        listed(
            client,
            "list log sources",
            sources_command(page_of(10)),
            &[],
            None,
        )
        .await,
    );
    cases.push(
        queried(
            client,
            "query logs",
            logs_command(page_of(10), START, END),
            &["before-window", "at-end-exclusive"],
        )
        .await,
    );
    cases.push(metrics(client, "list metrics", metrics_command(page_of(10))).await);
    cases.push(
        metric(
            client,
            "query metric",
            metric_command(page_of(10), START, END),
        )
        .await,
    );
    cases.push(tasks(client, "list tasks", tasks_command(page_of(10))).await);
    cases
}

async fn run_and_image_cases(client: &A2aClient) -> Vec<Success> {
    let mut cases = Vec::new();
    let start = StartTaskRequest::new(
        TaskId::new("mix").expect("task id"),
        JsonObject::parse(r#"{"note":"mix-note"}"#).expect("input"),
    );
    let started = client.start_task(start.clone()).await.expect("start task");
    let run_id = match &started.result {
        A2aLabResult::StartTask(run) => run.id.as_str().to_owned(),
        _ => panic!("start result"),
    };
    let mut started_body = body(&started.result);
    started_body
        .as_object_mut()
        .expect("start result")
        .remove("id");
    cases.push(Success {
        name: "start task",
        operation: "start_task".to_owned(),
        command: command_json(&A2aLabCommand::StartTask(start)),
        result: started_body,
        protocol_state: started.state.as_protocol().to_owned(),
        absent: Vec::new(),
        preview_bytes: None,
    });
    let status = GetTaskStatusRequest {
        id: RunId::new(&run_id).expect("run id"),
    };
    cases.push(status_case(client, "get task status", status).await);
    cases.push(
        image_sources(
            client,
            "list image sources",
            ListImageSourcesRequest::new(page_of(10)).expect("page"),
            &[],
        )
        .await,
    );
    cases.push(
        image_list(
            client,
            "list images",
            ListImagesRequest::new(ImageSourceId::new("cam-a").expect("source"), page_of(10))
                .expect("page"),
        )
        .await,
    );
    cases.push(
        image_search(
            client,
            "search images",
            SearchImagesRequest::new(None, None, Some("bench".to_owned()), page_of(10))
                .expect("search"),
        )
        .await,
    );
    let boundary = GetImageRequest::new(ImageId::new("boundary").expect("image"));
    let image = client
        .get_image(boundary.clone())
        .await
        .expect("boundary image");
    assert_eq!(image.data().len(), PREVIEW_LIMIT);
    cases.push(Success {
        name: "preview boundary",
        operation: "get_image".to_owned(),
        command: command_json(&A2aLabCommand::GetImage(boundary)),
        result: body(&A2aLabResult::GetImage(image)),
        protocol_state: "TASK_STATE_COMPLETED".to_owned(),
        absent: Vec::new(),
        preview_bytes: Some(PREVIEW_LIMIT),
    });
    let current = GetCurrentImageRequest::new(ImageSourceId::new("cam-a").expect("source"));
    let frame = client
        .get_current_image(current.clone())
        .await
        .expect("current image");
    cases.push(Success {
        name: "current image",
        operation: "get_current_image".to_owned(),
        command: command_json(&A2aLabCommand::GetCurrentImage(current)),
        result: body(&A2aLabResult::GetCurrentImage(frame.clone())),
        protocol_state: "TASK_STATE_COMPLETED".to_owned(),
        absent: Vec::new(),
        preview_bytes: Some(frame.data().len()),
    });
    cases
}

async fn page_cases(client: &A2aClient) -> Vec<Success> {
    let mut cases = Vec::new();
    cases.push(
        listed(
            client,
            "log source first page",
            sources_command(page_of(1)),
            &["beta", "gamma"],
            None,
        )
        .await,
    );
    cases.push(
        listed(
            client,
            "log source middle page",
            sources_command(page_at("1", 1)),
            &["alpha", "gamma"],
            None,
        )
        .await,
    );
    cases.push(
        listed(
            client,
            "log source last page",
            sources_command(page_at("2", 1)),
            &["alpha", "beta"],
            None,
        )
        .await,
    );
    cases.push(
        queried(
            client,
            "log record first page",
            logs_command(page_of(1), START, END),
            &["before-window", "at-end-exclusive", MARKUP],
        )
        .await,
    );
    cases.push(
        queried(
            client,
            "log record last page",
            logs_command(page_at("1", 1), START, END),
            &["before-window", "at-end-exclusive", "inside-early"],
        )
        .await,
    );
    cases.push(
        image_sources(
            client,
            "image source chunk",
            ListImageSourcesRequest::new(page_of(2)).expect("page"),
            &["cam-c"],
        )
        .await,
    );
    cases
}

pub(crate) async fn failure_cases(client: &A2aClient) -> Vec<Failure> {
    let mut cases = Vec::new();
    cases.push(
        failed(
            client,
            "zero page limit",
            command_value(json!({
                "operation": "list_log_sources",
                "params": {"page": {"limit": 0}}
            })),
            "rpc",
            "Status error",
        )
        .await,
    );
    cases.push(
        failed(
            client,
            "page limit above maximum",
            command_value(json!({
                "operation": "list_metrics",
                "params": {"page": {"limit": 1001}}
            })),
            "rpc",
            "Status error",
        )
        .await,
    );
    cases.push(
        failed(
            client,
            "cursor past end",
            A2aLabCommand::ListTasks(ListTasksRequest {
                page: page_at("9", 1),
            }),
            "rpc",
            "Status error",
        )
        .await,
    );
    cases.push(
        failed(
            client,
            "reversed time range",
            command_value(json!({
                "operation": "query_logs",
                "params": {
                    "source_id": "alpha",
                    "range": {"start": END, "end": START},
                    "page": {"limit": 10}
                }
            })),
            "rpc",
            "Status error",
        )
        .await,
    );
    let malformed = json!({
        "operation": "start_task",
        "params": {"task_id": "mix", "input": []}
    });
    let decoded =
        serde_json::from_value::<A2aLabCommand>(malformed.clone()).expect_err("array input");
    cases.push(Failure {
        name: "malformed task input",
        operation: "start_task".to_owned(),
        command: malformed,
        client: decoded.to_string(),
        kind: "rpc",
        status: "Status error",
    });
    cases.push(
        failed(
            client,
            "unknown image",
            A2aLabCommand::GetImage(GetImageRequest::new(
                ImageId::new("missing-image").expect("id"),
            )),
            "rpc",
            "Status error",
        )
        .await,
    );
    cases.push(
        failed(
            client,
            "unknown run",
            A2aLabCommand::GetTaskStatus(GetTaskStatusRequest {
                id: RunId::new("missing-run").expect("id"),
            }),
            "rpc",
            "Status error",
        )
        .await,
    );
    cases
}

pub(crate) async fn image_unavailable(base: &str) -> Failure {
    let client = A2aClient::new(base).expect("bare client");
    failed(
        &client,
        "unavailable image provider",
        A2aLabCommand::ListImageSources(ListImageSourcesRequest::new(page_of(10)).expect("page")),
        "unavailable",
        "Unavailable",
    )
    .await
}

pub(crate) async fn chat_cases(client: &A2aClient) -> String {
    let first = client
        .agent_message("hello parity", None)
        .await
        .expect("first turn");
    let second = client
        .agent_message("again parity", Some(&first.context_id))
        .await
        .expect("second turn");
    assert_eq!(second.context_id, first.context_id);
    let first_task = first.task_id.clone().expect("first task");
    let second_task = second.task_id.clone().expect("second task");
    assert_ne!(first_task, second_task);
    let listed = client
        .list_a2a_tasks(Some(&first.context_id))
        .await
        .expect("history");
    let blob = listed
        .tasks
        .iter()
        .map(task_blob)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(blob.contains("hello parity"), "{blob}");
    assert!(blob.contains("again parity"), "{blob}");
    assert!(blob.contains("echo:hello parity"), "{blob}");
    assert!(blob.contains("echo:again parity"), "{blob}");
    assert!(blob.contains(&first_task), "{blob}");
    assert!(blob.contains(&second_task), "{blob}");
    client
        .agent_message("unavailable", None)
        .await
        .expect_err("unavailable agent-message")
        .to_string()
}

pub(crate) async fn auth_cases(base: &str) -> (String, String) {
    let anonymous = A2aClient::new(base)
        .expect("anonymous client")
        .list_log_sources(ListLogSourcesRequest { page: page_of(10) })
        .await
        .expect_err("401")
        .to_string();
    let forbidden = A2aClient::new(base)
        .expect("noscope client")
        .with_bearer_token(NOSCOPE)
        .list_log_sources(ListLogSourcesRequest { page: page_of(10) })
        .await
        .expect_err("403")
        .to_string();
    let alice = A2aClient::new(base)
        .expect("alice")
        .with_bearer_token(ALICE);
    let turn = alice
        .agent_message("alice-only", None)
        .await
        .expect("alice turn");
    let alice_task = turn.task_id.clone().expect("alice task");
    let bob_tasks = A2aClient::new(base)
        .expect("bob")
        .with_bearer_token(BOB)
        .list_a2a_tasks(Some(&turn.context_id))
        .await
        .expect("bob list");
    assert!(
        bob_tasks.tasks.iter().all(|task| task.id != alice_task),
        "bob saw alice: {bob_tasks:?}"
    );
    let alice_tasks = alice
        .list_a2a_tasks(Some(&turn.context_id))
        .await
        .expect("alice list");
    assert!(
        alice_tasks.tasks.iter().any(|task| task.id == alice_task),
        "alice lost her task: {alice_tasks:?}"
    );
    (anonymous, forbidden)
}

pub(crate) async fn assert_feature_matrix(opted: &str, headless: &str) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let missing = http
        .get(format!("{headless}/ui/"))
        .send()
        .await
        .expect("headless ui");
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let present = http
        .get(format!("{opted}/ui/"))
        .send()
        .await
        .expect("opted ui");
    assert_eq!(present.status(), reqwest::StatusCode::OK);
    let page = page_of(10);
    let left = A2aClient::new(headless)
        .expect("headless client")
        .list_log_sources(ListLogSourcesRequest { page: page.clone() })
        .await
        .expect("headless logs");
    let right = A2aClient::new(opted)
        .expect("opted client")
        .list_log_sources(ListLogSourcesRequest { page })
        .await
        .expect("opted logs");
    assert_eq!(
        serde_json::to_value(&left).expect("left"),
        serde_json::to_value(&right).expect("right")
    );
}

pub(crate) async fn assert_public_assets(base: &str) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let html = http
        .get(format!("{base}/ui/"))
        .send()
        .await
        .expect("shell")
        .text()
        .await
        .expect("shell text");
    let mut bodies = vec![html.clone()];
    for marker in ["href=\"", "src=\""] {
        if let Some(path) = quoted(&html, marker) {
            bodies.push(fetch_text(&http, &format!("{base}{path}")).await);
        }
    }
    let glue = bodies
        .iter()
        .find(|body| body.contains(".wasm"))
        .cloned()
        .unwrap_or_default();
    if let Some(name) = glue.split(['\'', '"']).find(|part| {
        part.starts_with("app-")
            && std::path::Path::new(part)
                .extension()
                .is_some_and(|ext| ext == "wasm")
    }) {
        let bytes = http
            .get(format!("{base}/ui/assets/{name}"))
            .send()
            .await
            .expect("wasm")
            .bytes()
            .await
            .expect("wasm bytes");
        bodies.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    for body in &bodies {
        for secret in [ALICE, BOB, NOSCOPE] {
            assert!(!body.contains(secret), "public asset contains {secret}");
        }
    }
}

pub(crate) async fn assert_agent_message_advertised(base: &str, present: bool) {
    let card = A2aClient::new(base)
        .expect("card client")
        .agent_card()
        .await
        .expect("card");
    assert_eq!(skill_present(&card), present, "{base}");
}

fn skill_present(card: &AgentCard) -> bool {
    serde_json::to_value(card).expect("card json")["skills"]
        .as_array()
        .is_some_and(|skills| {
            skills
                .iter()
                .any(|skill| skill["id"].as_str() == Some("agent-message"))
        })
}

async fn fetch_text(http: &reqwest::Client, url: &str) -> String {
    http.get(url)
        .send()
        .await
        .unwrap_or_else(|_| panic!("get {url}"))
        .text()
        .await
        .unwrap_or_else(|_| panic!("read {url}"))
}

fn quoted<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    rest.find('"').map(|end| &rest[..end])
}

fn task_blob(task: &a2a_lab_dev_kit::Task) -> String {
    let mut parts = vec![task.id.clone()];
    if let Some(history) = &task.history {
        for message in history {
            for part in &message.parts {
                if let Some(text) = part.as_text() {
                    parts.push(text.to_owned());
                }
            }
        }
    }
    if let Some(artifacts) = &task.artifacts {
        for artifact in artifacts {
            for part in &artifact.parts {
                if let Some(text) = part.as_text() {
                    parts.push(text.to_owned());
                }
            }
        }
    }
    parts.join("\n")
}

async fn listed(
    client: &A2aClient,
    name: &'static str,
    command: A2aLabCommand,
    absent: &[&'static str],
    preview: Option<usize>,
) -> Success {
    let A2aLabCommand::ListLogSources(request) = command.clone() else {
        panic!("log sources");
    };
    let page = client.list_log_sources(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::ListLogSources(page)),
        absent,
        preview,
    )
}

async fn queried(
    client: &A2aClient,
    name: &'static str,
    command: A2aLabCommand,
    absent: &[&'static str],
) -> Success {
    let A2aLabCommand::QueryLogs(request) = command.clone() else {
        panic!("query logs");
    };
    let page = client.query_logs(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::QueryLogs(page)),
        absent,
        None,
    )
}

async fn metrics(client: &A2aClient, name: &'static str, command: A2aLabCommand) -> Success {
    let A2aLabCommand::ListMetrics(request) = command.clone() else {
        panic!("metrics");
    };
    let page = client.list_metrics(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::ListMetrics(page)),
        &[],
        None,
    )
}

async fn metric(client: &A2aClient, name: &'static str, command: A2aLabCommand) -> Success {
    let A2aLabCommand::QueryMetric(request) = command.clone() else {
        panic!("metric");
    };
    let page = client.query_metric(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::QueryMetric(page)),
        &[],
        None,
    )
}

async fn tasks(client: &A2aClient, name: &'static str, command: A2aLabCommand) -> Success {
    let A2aLabCommand::ListTasks(request) = command.clone() else {
        panic!("tasks");
    };
    let page = client.list_tasks(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::ListTasks(page)),
        &[],
        None,
    )
}

async fn status_case(
    client: &A2aClient,
    name: &'static str,
    request: GetTaskStatusRequest,
) -> Success {
    let command = A2aLabCommand::GetTaskStatus(request.clone());
    let run = client.task_status(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::GetTaskStatus(run)),
        &[],
        None,
    )
}

async fn image_sources(
    client: &A2aClient,
    name: &'static str,
    request: ListImageSourcesRequest,
    absent: &[&'static str],
) -> Success {
    let command = A2aLabCommand::ListImageSources(request.clone());
    let page = client.list_image_sources(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::ListImageSources(page)),
        absent,
        None,
    )
}

async fn image_list(client: &A2aClient, name: &'static str, request: ListImagesRequest) -> Success {
    let command = A2aLabCommand::ListImages(request.clone());
    let page = client.list_images(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::ListImages(page)),
        &[],
        None,
    )
}

async fn image_search(
    client: &A2aClient,
    name: &'static str,
    request: SearchImagesRequest,
) -> Success {
    let command = A2aLabCommand::SearchImages(request.clone());
    let page = client.search_images(request).await.expect(name);
    finished(
        name,
        &command,
        body(&A2aLabResult::SearchImages(page)),
        &[],
        None,
    )
}

fn finished(
    name: &'static str,
    command: &A2aLabCommand,
    result: Value,
    absent: &[&'static str],
    preview_bytes: Option<usize>,
) -> Success {
    Success {
        name,
        operation: command_json(command)["operation"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        command: command_json(command),
        result,
        protocol_state: "TASK_STATE_COMPLETED".to_owned(),
        absent: absent.to_vec(),
        preview_bytes,
    }
}

async fn failed(
    client: &A2aClient,
    name: &'static str,
    command: A2aLabCommand,
    kind: &'static str,
    status: &'static str,
) -> Failure {
    let client_text = match &command {
        A2aLabCommand::ListLogSources(request) => client
            .list_log_sources(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::ListMetrics(request) => client
            .list_metrics(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::ListTasks(request) => client
            .list_tasks(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::QueryLogs(request) => client
            .query_logs(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::GetImage(request) => client
            .get_image(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::GetTaskStatus(request) => client
            .task_status(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        A2aLabCommand::ListImageSources(request) => client
            .list_image_sources(request.clone())
            .await
            .expect_err(name)
            .to_string(),
        _ => panic!("unsupported failure command"),
    };
    Failure {
        name,
        operation: command_json(&command)["operation"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        command: command_json(&command),
        client: client_text,
        kind,
        status,
    }
}

fn command_json(command: &A2aLabCommand) -> Value {
    serde_json::to_value(command).expect("command json")
}

fn command_value(value: Value) -> A2aLabCommand {
    serde_json::from_value(value).expect("command")
}

fn body(result: &A2aLabResult) -> Value {
    let value = serde_json::to_value(result).expect("result");
    value.get("result").cloned().unwrap_or(value)
}

fn sources_command(page: PageRequest) -> A2aLabCommand {
    A2aLabCommand::ListLogSources(ListLogSourcesRequest { page })
}

fn logs_command(page: PageRequest, start: &str, end: &str) -> A2aLabCommand {
    A2aLabCommand::QueryLogs(QueryLogsRequest {
        source_id: SourceId::new("alpha").expect("source"),
        range: span(start, end),
        page,
    })
}

fn metrics_command(page: PageRequest) -> A2aLabCommand {
    A2aLabCommand::ListMetrics(ListMetricsRequest { page })
}

fn metric_command(page: PageRequest, start: &str, end: &str) -> A2aLabCommand {
    A2aLabCommand::QueryMetric(QueryMetricRequest {
        metric_id: MetricId::new("temp").expect("metric"),
        range: span(start, end),
        page,
    })
}

fn tasks_command(page: PageRequest) -> A2aLabCommand {
    A2aLabCommand::ListTasks(ListTasksRequest { page })
}

pub(crate) fn page_of(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).expect("page")
}

fn page_at(cursor: &str, limit: u32) -> PageRequest {
    PageRequest::new(Some(cursor.to_owned()), limit).expect("page")
}

fn span(start: &str, end: &str) -> TimeRange {
    TimeRange::new(
        UtcTimestamp::parse(start).expect("start"),
        UtcTimestamp::parse(end).expect("end"),
    )
    .expect("range")
}

pub(crate) fn complete_runs(tasks: MemoryTasks, stop: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut number = 1_u64;
        while !stop.load(Ordering::Relaxed) {
            let id = RunId::new(format!("run-{number}")).expect("run id");
            if tasks
                .transition(&id, TaskState::Completed, Some("done".to_owned()))
                .await
                .is_ok()
            {
                number += 1;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    });
}

pub(crate) async fn seed() -> Lab {
    let logs = seed_logs().await;
    let metrics = seed_metrics().await;
    let tasks = seed_tasks().await;
    let images = seed_images().await;
    let service = A2aLabService::new(logs, metrics, tasks.clone())
        .with_images(images.clone())
        .share();
    Lab {
        service,
        images,
        tasks,
    }
}

async fn seed_logs() -> MemoryLogs {
    let logs = MemoryLogs::new();
    for (id, name) in [("alpha", "Alpha"), ("beta", "Beta"), ("gamma", "Gamma")] {
        logs.insert_source(LogSource {
            id: SourceId::new(id).expect("source"),
            name: name.to_owned(),
            description: id.to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    }
    for (at, message) in [
        ("2020-05-31T00:00:00Z", "before-window"),
        (START, "inside-early"),
        ("2020-06-01T12:00:00Z", MARKUP),
        (END, "at-end-exclusive"),
    ] {
        logs.insert_record(LogRecord {
            source_id: SourceId::new("alpha").expect("source"),
            timestamp: UtcTimestamp::parse(at).expect("time"),
            level: LogLevel::Info,
            message: message.to_owned(),
            attributes: JsonObject::empty(),
        })
        .await
        .expect("log record");
    }
    logs
}

async fn seed_metrics() -> MemoryMetrics {
    let metrics = MemoryMetrics::new();
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("temp").expect("metric"),
            name: "Temperature".to_owned(),
            description: format!("probe {MARKUP}"),
            unit: "C".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("pressure").expect("metric"),
            name: "Pressure".to_owned(),
            description: "line".to_owned(),
            unit: "kPa".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            MetricId::new("temp").expect("metric"),
            MetricPoint::new(UtcTimestamp::parse(START).expect("time"), 12.5).expect("point"),
        )
        .await
        .expect("point");
    metrics
}

async fn seed_tasks() -> MemoryTasks {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("mix").expect("task"),
            name: "Mix".to_owned(),
            description: format!("mix {MARKUP}"),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("heat").expect("task"),
            name: "Heat".to_owned(),
            description: "heat".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    tasks
}

async fn seed_images() -> MemoryImages {
    let images = MemoryImages::new();
    for id in ["cam-a", "cam-b", "cam-c"] {
        images
            .insert_source(ImageSource {
                id: ImageSourceId::new(id).expect("source"),
                name: id.to_owned(),
                description: id.to_owned(),
                asset_id: None,
                semantic_id: None,
            })
            .await
            .expect("image source");
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .expect("png");
    images
        .insert_image(
            Image::new(
                frame("frame", "cam-a", Some(&format!("bench {MARKUP}")), START),
                png,
            )
            .expect("frame"),
        )
        .await
        .expect("frame");
    images
        .insert_image(
            Image::new(
                frame("boundary", "cam-a", None, "2020-06-01T01:00:00Z"),
                vec![7; PREVIEW_LIMIT],
            )
            .expect("boundary"),
        )
        .await
        .expect("boundary");
    images
        .set_current(
            ImageSourceId::new("cam-a").expect("source"),
            ImageId::new("frame").expect("image"),
        )
        .await
        .expect("current");
    images
}

fn frame(id: &str, source: &str, caption: Option<&str>, at: &str) -> ImageDescriptor {
    ImageDescriptor::new(
        ImageId::new(id).expect("image"),
        ImageSourceId::new(source).expect("source"),
        UtcTimestamp::parse(at).expect("time"),
        "image/png",
        1,
        1,
        caption.map(str::to_owned),
        JsonObject::empty(),
    )
    .expect("descriptor")
}

pub(crate) fn secured(service: &Arc<dyn a2a_lab_dev_kit::A2aLabApi>) -> A2aServer {
    let schemes: HashMap<String, SecurityScheme> = [(
        "bearerAuth".to_owned(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "bearer".to_owned(),
            description: None,
            bearer_format: None,
        }),
    )]
    .into();
    let requirements: Vec<SecurityRequirement> =
        vec![[("bearerAuth".to_owned(), vec!["lab".to_owned()])].into()];
    A2aServer::new(service)
        .with_web_ui()
        .with_message_handler(Arc::new(Echo))
        .with_security(schemes, requirements)
        .with_authenticator(Arc::new(Gate))
}

pub(crate) async fn listen(server: A2aServer) -> String {
    let (listener, address) = bind_local().await.expect("bind");
    tokio::spawn(async move {
        server.listen(listener).await.expect("listen");
    });
    format!("http://{address}")
}
