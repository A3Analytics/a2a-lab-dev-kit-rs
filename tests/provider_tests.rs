use a2a_lab_sdk::{
    GetWorkflowStatusRequest, JsonObject, ListLogSourcesRequest, ListMetricsRequest,
    ListWorkflowsRequest, LogLevel, LogProvider, LogRecord, LogSource, MemoryLogs, MemoryMetrics,
    MemoryWorkflows, MetricDescriptor, MetricId, MetricPoint, MetricProvider, PageRequest,
    QueryLogsRequest, QueryMetricRequest, RunState, SourceId, StartWorkflowRequest, TimeRange,
    UtcTimestamp, WorkflowDefinition, WorkflowId, WorkflowProvider,
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

fn source(id: &str) -> LogSource {
    LogSource {
        id: SourceId::new(id).unwrap(),
        name: id.to_owned(),
        description: format!("{id} logs"),
        asset_id: None,
        semantic_id: None,
    }
}

#[tokio::test]
async fn paginates_log_sources_and_queries_a_half_open_range() {
    let logs = MemoryLogs::new();
    logs.insert_source(source("beta")).await;
    logs.insert_source(source("alpha")).await;
    let first = logs
        .list_sources(ListLogSourcesRequest { page: page(1) })
        .await
        .unwrap();
    assert_eq!(first.items()[0].id.as_str(), "alpha");
    let second = logs
        .list_sources(ListLogSourcesRequest {
            page: PageRequest::new(first.next_cursor().map(str::to_owned), 1).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(second.items()[0].id.as_str(), "beta");
    assert!(second.next_cursor().is_none());

    let source_id = SourceId::new("alpha").unwrap();
    for (stamp, message) in [
        ("2024-01-01T00:00:00Z", "start"),
        ("2024-01-01T00:30:00Z", "middle"),
        ("2024-01-01T01:00:00Z", "end"),
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
    let queried = logs
        .query(QueryLogsRequest {
            source_id: source_id.clone(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert_eq!(
        queried
            .items()
            .iter()
            .map(|record| record.message.as_str())
            .collect::<Vec<_>>(),
        ["start", "middle"]
    );
    let missing = logs
        .query(QueryLogsRequest {
            source_id: SourceId::new("missing").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}

#[tokio::test]
async fn queries_metric_samples_and_reports_provider_failure() {
    let metrics = MemoryMetrics::new();
    let metric_id = MetricId::new("latency").unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: "Latency".to_owned(),
            description: "Request latency".to_owned(),
            unit: "ms".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            metric_id.clone(),
            MetricPoint::new(timestamp("2024-01-01T00:00:00Z"), 5.0).unwrap(),
        )
        .await
        .unwrap();
    let listed = metrics
        .list_metrics(ListMetricsRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(listed.items().len(), 1);
    let queried = metrics
        .query(QueryMetricRequest {
            metric_id: metric_id.clone(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert!((queried.items()[0].value - 5.0).abs() < f64::EPSILON);
    metrics.set_unavailable("metrics offline").await;
    let error = metrics
        .list_metrics(ListMetricsRequest { page: page(10) })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unavailable");
}

#[tokio::test]
async fn starts_a_workflow_and_tracks_its_status() {
    let workflows = MemoryWorkflows::new();
    let workflow_id = WorkflowId::new("build").unwrap();
    workflows
        .insert(WorkflowDefinition {
            id: workflow_id.clone(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let listed = workflows
        .list_workflows(ListWorkflowsRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(listed.items()[0].id, workflow_id);
    let run = workflows
        .start(StartWorkflowRequest {
            workflow_id: workflow_id.clone(),
            input: JsonObject::parse(r#"{"branch":"main"}"#).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Submitted);
    workflows
        .transition(&run.id, RunState::Completed, Some("done".to_owned()))
        .await
        .unwrap();
    let status = workflows
        .status(GetWorkflowStatusRequest {
            run_id: run.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(status.state, RunState::Completed);
    assert_eq!(status.message.as_deref(), Some("done"));
    let missing = workflows
        .start(StartWorkflowRequest {
            workflow_id: WorkflowId::new("missing").unwrap(),
            input: JsonObject::empty(),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}
