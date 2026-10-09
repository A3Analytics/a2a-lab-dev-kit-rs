#![cfg(feature = "sila2")]

use std::sync::Arc;

use a2a_lab_dev_kit::sila::{
    LogBinding, MemberKind, MetricBinding, RequestBinding, SilaBinding, SilaIdentity, SilaMember,
    SilaProvider, SilaProviderConfig, SilaServer, TaskBinding,
};
use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabResult, A2aLabService, GetTaskStatusRequest,
    JsonObject, ListTasksRequest, LogLevel, LogProvider, LogRecord, LogSource, McpLab, McpServer,
    MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId, MetricPoint,
    MetricProvider, PageRequest, QueryLogsRequest, QueryMetricRequest, SourceId, StartTaskRequest,
    TaskDefinition, TaskId, TaskProvider, TaskState, TimeRange, UtcTimestamp, bind_local,
};

const UUID: &str = "11111111-1111-1111-1111-111111111111";
const FEATURE: &str = "com.a3analytics/lab/LabOperations/v1";

#[tokio::test]
async fn configured_server_serves_tasks_logs_and_metrics() {
    let remote = seed_remote().await;
    let server = SilaServer::new(SilaIdentity::lab_dev_kit(UUID).unwrap(), remote)
        .plaintext()
        .serve("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let provider = SilaProvider::connect(config(server.local_addr().port()))
        .await
        .unwrap();
    assert_tasks(&provider).await;
    assert_logs(&provider).await;
    assert_metric(&provider).await;
    assert_interfaces(provider).await;
}

async fn seed_remote() -> Arc<dyn a2a_lab_dev_kit::A2aLabApi> {
    let logs = MemoryLogs::new();
    let metrics = MemoryMetrics::new();
    let tasks = MemoryTasks::new();
    logs.insert_source(LogSource {
        id: SourceId::new("oven").unwrap(),
        name: "Oven".to_owned(),
        description: "Oven log".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_record(LogRecord {
        source_id: SourceId::new("oven").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap(),
        level: LogLevel::Warn,
        message: "hot".to_owned(),
        attributes: JsonObject::parse(r#"{"zone":1}"#).unwrap(),
    })
    .await
    .unwrap();
    logs.insert_record(LogRecord {
        source_id: SourceId::new("oven").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-01T00:45:00Z").unwrap(),
        level: LogLevel::Info,
        message: "ready".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await
    .unwrap();
    logs.insert_record(LogRecord {
        source_id: SourceId::new("oven").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-02T00:00:00Z").unwrap(),
        level: LogLevel::Error,
        message: "later".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await
    .unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("temp").unwrap(),
            name: "Temperature".to_owned(),
            description: "Oven temperature".to_owned(),
            unit: "C".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            MetricId::new("temp").unwrap(),
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap(), 42.5).unwrap(),
        )
        .await
        .unwrap();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("mix").unwrap(),
            name: "Mix".to_owned(),
            description: "Mix".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    A2aLabService::new(logs, metrics, tasks).share()
}

async fn assert_tasks(provider: &SilaProvider) {
    let listed = provider
        .list_tasks(a2a_lab_dev_kit::ListTasksRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(listed.items().len(), 3);
    assert!(
        listed
            .items()
            .iter()
            .any(|task| task.input_schema.is_some())
    );

    let name = provider
        .start(StartTaskRequest::new(
            TaskId::new("server-name").unwrap(),
            JsonObject::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(name.state, TaskState::Completed);
    assert!(name.result.is_some());

    let started = provider
        .start(
            StartTaskRequest::new(
                TaskId::new("mix-remote").unwrap(),
                JsonObject::parse(r#"{"TaskId":"mix","Input":"{}"}"#).unwrap(),
            )
            .immediate(),
        )
        .await
        .unwrap();
    assert!(!started.state.is_terminal());
    let canceled = provider
        .cancel(GetTaskStatusRequest {
            id: started.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(canceled.state, TaskState::Canceled);
}

async fn assert_logs(provider: &SilaProvider) {
    let records = LogProvider::query(
        provider,
        QueryLogsRequest {
            source_id: SourceId::new("events").unwrap(),
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-01-01T01:00:00Z").unwrap(),
            )
            .unwrap(),
            page: PageRequest::new(None, 1).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(records.items().len(), 1);
    assert_eq!(records.items()[0].message, "hot");
    assert_eq!(records.items()[0].level, LogLevel::Warn);
    assert_eq!(records.next_cursor(), Some("1"));
    let next = LogProvider::query(
        provider,
        QueryLogsRequest {
            source_id: SourceId::new("events").unwrap(),
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-01-01T01:00:00Z").unwrap(),
            )
            .unwrap(),
            page: PageRequest::new(Some("1".to_owned()), 1).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(next.items()[0].message, "ready");
    assert!(next.next_cursor().is_none());
}

async fn assert_metric(provider: &SilaProvider) {
    let points = MetricProvider::query(
        provider,
        QueryMetricRequest {
            metric_id: MetricId::new("temp").unwrap(),
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-01-01T01:00:00Z").unwrap(),
            )
            .unwrap(),
            page: PageRequest::new(None, 10).unwrap(),
        },
    )
    .await
    .unwrap();
    assert!((points.items()[0].value - 42.5).abs() < f64::EPSILON);
}

async fn assert_interfaces(provider: SilaProvider) {
    let service = A2aLabService::new(provider.clone(), provider.clone(), provider).share();
    let (listener, address) = bind_local().await.unwrap();
    let endpoint = Arc::clone(&service);
    tokio::spawn(async move {
        a2a_lab_dev_kit::A2aServer::new(&endpoint)
            .listen(listener)
            .await
            .unwrap();
    });
    let client = A2aClient::new(&format!("http://{address}")).unwrap();
    let tasks = client
        .list_tasks(ListTasksRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(tasks.items().len(), 3);

    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    let mcp = McpServer::new(&service);
    tokio::spawn(async move {
        mcp.serve_http(mcp_listener).await.unwrap();
    });
    let mcp_lab = McpLab::connect(&format!("http://{mcp_address}/mcp"))
        .await
        .unwrap();
    let outcome = mcp_lab
        .execute(A2aLabCommand::ListTasks(ListTasksRequest {
            page: PageRequest::new(None, 10).unwrap(),
        }))
        .await
        .unwrap();
    let A2aLabResult::ListTasks(page) = outcome.task.result else {
        panic!("expected tasks");
    };
    assert_eq!(page.items().len(), 3);
}

#[tokio::test]
async fn encrypted_connection_requires_the_sila_certificate_profile() {
    let Err(error) = SilaProvider::connect(SilaProviderConfig {
        host: "127.0.0.1".to_owned(),
        port: 1,
        server_uuid: UUID.to_owned(),
        plaintext: false,
        ca_pem: Some("not-a-sila-certificate".to_owned()),
        metadata: None,
        bindings: Vec::new(),
    })
    .await
    else {
        panic!("expected the certificate profile to be rejected");
    };
    assert_eq!(error.code(), "invalid");
}

fn config(port: u16) -> SilaProviderConfig {
    let page = JsonObject::parse(r#"{"HasCursor":false,"Cursor":"","Limit":100}"#).unwrap();
    SilaProviderConfig {
        host: "127.0.0.1".to_owned(),
        port,
        server_uuid: UUID.to_owned(),
        plaintext: true,
        ca_pem: None,
        metadata: None,
        bindings: vec![
            task("server-name", "Server name", "ServerName", None),
            task(
                "list-tasks",
                "List tasks",
                "ListTasks",
                Some(
                    JsonObject::parse(r#"{"Page":{"HasCursor":false,"Cursor":"","Limit":10}}"#)
                        .unwrap(),
                ),
            ),
            task("mix-remote", "Mix", "StartTask", None),
            logs(&page),
            metric(&page),
        ],
    }
}

fn task(id: &str, name: &str, identifier: &str, arguments: Option<JsonObject>) -> SilaBinding {
    let feature = if identifier == "ServerName" {
        "org.silastandard/core/SiLAService/v1"
    } else {
        FEATURE
    };
    let kind = if arguments.is_none() && identifier == "ServerName" {
        MemberKind::Property
    } else {
        MemberKind::Command
    };
    SilaBinding::Task(TaskBinding {
        id: id.to_owned(),
        name: name.to_owned(),
        description: name.to_owned(),
        asset_id: None,
        semantic_id: None,
        member: SilaMember {
            feature: feature.to_owned(),
            kind,
            identifier: identifier.to_owned(),
            metadata: None,
            arguments,
        },
    })
}

fn logs(page: &JsonObject) -> SilaBinding {
    SilaBinding::Logs(LogBinding {
        id: "events".to_owned(),
        name: "Events".to_owned(),
        description: "Remote log records".to_owned(),
        asset_id: None,
        semantic_id: None,
        member: command(
            "QueryLogs",
            JsonObject::parse(&format!(
                r#"{{"SourceId":"oven","Range":{{}},"Page":{page}}}"#
            ))
            .unwrap(),
        ),
        request: range_request(),
        records: "/Records".to_owned(),
        timestamp: "/Timestamp".to_owned(),
        level: "/Level".to_owned(),
        message: "/Message".to_owned(),
        attributes: Some("/Attributes".to_owned()),
        next_cursor: None,
    })
}

fn metric(page: &JsonObject) -> SilaBinding {
    SilaBinding::Metric(MetricBinding {
        id: "temp".to_owned(),
        name: "Temperature".to_owned(),
        description: "Remote temperature".to_owned(),
        unit: "C".to_owned(),
        asset_id: None,
        semantic_id: None,
        member: command(
            "QueryMetric",
            JsonObject::parse(&format!(
                r#"{{"MetricId":"temp","Range":{{}},"Page":{page}}}"#
            ))
            .unwrap(),
        ),
        request: range_request(),
        points: "/Points".to_owned(),
        timestamp: "/Timestamp".to_owned(),
        value: "/Value".to_owned(),
        next_cursor: None,
    })
}

fn command(identifier: &str, arguments: JsonObject) -> SilaMember {
    SilaMember {
        feature: FEATURE.to_owned(),
        kind: MemberKind::Command,
        identifier: identifier.to_owned(),
        metadata: None,
        arguments: Some(arguments),
    }
}

fn range_request() -> RequestBinding {
    RequestBinding {
        start: Some("/Range/Start".to_owned()),
        end: Some("/Range/End".to_owned()),
        cursor: None,
        limit: None,
    }
}
