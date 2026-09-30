//! In-memory logs, metrics, and tasks.
//!
//! Lists log sources one at a time, then queries a half-open UTC range
//! `[start, end)`.

use a2a_lab_sdk::{
    GetTaskStatusRequest, JsonObject, ListLogSourcesRequest, ListMetricsRequest, ListTasksRequest,
    LogLevel, LogProvider, LogRecord, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks,
    MetricDescriptor, MetricId, MetricPoint, MetricProvider, PageRequest, QueryLogsRequest,
    QueryMetricRequest, SdkError, SourceId, StartTaskRequest, TaskDefinition, TaskId, TaskProvider,
    TaskState, TimeRange, UtcTimestamp,
};

#[tokio::main]
async fn main() -> Result<(), SdkError> {
    demonstrate_logs().await?;
    demonstrate_metrics().await?;
    demonstrate_tasks().await?;
    Ok(())
}

async fn demonstrate_logs() -> Result<(), SdkError> {
    let logs = MemoryLogs::new();
    logs.insert_source(source("beta")).await;
    logs.insert_source(source("alpha")).await;

    let first = logs
        .list_sources(ListLogSourcesRequest {
            page: PageRequest::new(None, 1)?,
        })
        .await?;
    let first_id = first.items()[0].id.as_str();
    let cursor = first.next_cursor().map(str::to_owned);
    println!("log sources page 1: {first_id} next={cursor:?}");

    let second = logs
        .list_sources(ListLogSourcesRequest {
            page: PageRequest::new(cursor, 1)?,
        })
        .await?;
    println!(
        "log sources page 2: {} next={:?}",
        second.items()[0].id,
        second.next_cursor()
    );

    let source_id = SourceId::new("alpha")?;
    for (stamp, message) in [
        ("2024-01-01T00:00:00Z", "start"),
        ("2024-01-01T00:30:00Z", "middle"),
        ("2024-01-01T01:00:00Z", "end"),
    ] {
        logs.insert_record(LogRecord {
            source_id: source_id.clone(),
            timestamp: UtcTimestamp::parse(stamp)?,
            level: LogLevel::Info,
            message: message.to_owned(),
            attributes: JsonObject::empty(),
        })
        .await?;
    }

    let queried = logs
        .query(QueryLogsRequest {
            source_id,
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
                UtcTimestamp::parse("2024-01-01T01:00:00Z")?,
            )?,
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    let kept = queried
        .items()
        .iter()
        .map(|record| record.message.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!("half-open [2024-01-01T00:00:00Z, 2024-01-01T01:00:00Z) kept: {kept}");
    Ok(())
}

async fn demonstrate_metrics() -> Result<(), SdkError> {
    let metrics = MemoryMetrics::new();
    let metric_id = MetricId::new("latency")?;
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
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:00:00Z")?, 5.0)?,
        )
        .await?;
    let listed = metrics
        .list_metrics(ListMetricsRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    let samples = metrics
        .query(QueryMetricRequest {
            metric_id,
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
                UtcTimestamp::parse("2024-01-01T01:00:00Z")?,
            )?,
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!(
        "metric {} sample {}",
        listed.items()[0].id,
        samples.items()[0].value
    );
    Ok(())
}

async fn demonstrate_tasks() -> Result<(), SdkError> {
    let tasks = MemoryTasks::new();
    let task_id = TaskId::new("build")?;
    tasks
        .insert(TaskDefinition {
            id: task_id.clone(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let listed = tasks
        .list_tasks(ListTasksRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    let run = tasks
        .start(StartTaskRequest::new(
            task_id,
            JsonObject::parse(r#"{"branch":"main"}"#)?,
        ))
        .await?;
    tasks
        .transition(&run.id, TaskState::Completed, Some("done".to_owned()))
        .await?;
    let status = tasks
        .status(GetTaskStatusRequest { id: run.id.clone() })
        .await?;
    println!(
        "task {} run {} {:?}",
        listed.items()[0].id,
        status.id,
        status.state
    );
    Ok(())
}

fn source(id: &str) -> LogSource {
    LogSource {
        id: SourceId::new(id).expect("source id"),
        name: id.to_owned(),
        description: format!("{id} logs"),
        asset_id: None,
        semantic_id: None,
    }
}
