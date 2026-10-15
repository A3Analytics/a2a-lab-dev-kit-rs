//! In-memory lab served with the optional web interface.
//!
//! The process prints a local `/ui/` URL and keeps listening. Open that URL
//! to browse logs, metrics, tasks, images, and an `agent-message` echo.

use std::io::{Write, stdout};
use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, GetTaskStatusRequest, Image, ImageDescriptor, ImageId,
    ImageSource, ImageSourceId, JsonObject, LogLevel, LogRecord, LogSource, MemoryImages,
    MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId, MetricPoint, RunId,
    SourceId, TaskDefinition, TaskId, TaskProvider, TaskState, UtcTimestamp, WEB_UI_PATH,
};
use tokio::net::TcpListener;

const DEFAULT_BIND: &str = "127.0.0.1:3847";

const FRAME: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xFC, 0xCF, 0xC0, 0x50,
    0x0F, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xA9, 0x8C, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

struct Echo;

impl AgentMessageHandler for Echo {
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>> {
        let text = request.text.clone();
        Box::pin(async move {
            Ok(AgentMessageReply {
                text: format!("echo {text}"),
            })
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = lab().await?;
    let listener = bind().await?;
    let address = listener.local_addr()?;
    writeln!(stdout(), "http://{address}{WEB_UI_PATH}")?;
    stdout().flush()?;
    A2aServer::new(&service)
        .with_message_handler(Arc::new(Echo))
        .with_web_ui()
        .listen(listener)
        .await?;
    Ok(())
}

async fn bind() -> Result<TcpListener, Box<dyn std::error::Error + Send + Sync>> {
    let address = std::env::var("WEB_UI_BIND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BIND.to_owned());
    Ok(TcpListener::bind(&address).await?)
}

async fn lab() -> Result<Arc<dyn A2aLabApi>, A2aLabError> {
    let logs = seed_logs().await?;
    let metrics = seed_metrics().await?;
    let tasks = seed_tasks().await?;
    complete_started(tasks.clone());

    let images = MemoryImages::new();
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new("bench")?,
            name: "Bench".to_owned(),
            description: "Bench camera".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await?;
    images.insert_image(frame()?).await?;
    images
        .set_current(ImageSourceId::new("bench")?, ImageId::new("current")?)
        .await?;

    Ok(A2aLabService::new(logs, metrics, tasks)
        .with_images(images)
        .share())
}

async fn seed_logs() -> Result<MemoryLogs, A2aLabError> {
    let logs = MemoryLogs::new();
    insert_source(&logs, "app", "App", "Application logs").await?;
    insert_source(&logs, "device", "Device", "Device logs").await?;
    for (timestamp, level, message, attributes) in [
        (
            "2024-01-01T00:00:00Z",
            LogLevel::Info,
            "lab ready",
            JsonObject::empty(),
        ),
        (
            "2024-01-01T00:00:01Z",
            LogLevel::Debug,
            "configuration loaded",
            JsonObject::empty(),
        ),
        (
            "2024-01-01T00:00:02Z",
            LogLevel::Info,
            "agent card published",
            JsonObject::empty(),
        ),
        (
            "2024-01-01T00:00:05Z",
            LogLevel::Warn,
            "listener retry",
            JsonObject::parse(r#"{"attempt":"1"}"#)?,
        ),
        (
            "2024-01-01T00:00:08Z",
            LogLevel::Info,
            "listener bound",
            JsonObject::parse(r#"{"address":"127.0.0.1"}"#)?,
        ),
        (
            "2024-01-01T00:00:15Z",
            LogLevel::Error,
            "command timed out",
            JsonObject::parse(r#"{"operation":"query_logs"}"#)?,
        ),
        (
            "2024-01-01T00:01:00Z",
            LogLevel::Trace,
            "heartbeat",
            JsonObject::empty(),
        ),
        (
            "2024-01-01T00:02:30Z",
            LogLevel::Info,
            "session started",
            JsonObject::empty(),
        ),
    ] {
        insert_record(&logs, "app", timestamp, level, message, attributes).await?;
    }
    for (timestamp, level, message) in [
        ("2024-01-01T00:00:03Z", LogLevel::Info, "motor enabled"),
        ("2024-01-01T00:00:20Z", LogLevel::Warn, "limit switch"),
        ("2024-01-01T00:01:15Z", LogLevel::Error, "axis stalled"),
        ("2024-01-01T00:02:00Z", LogLevel::Info, "home complete"),
    ] {
        insert_record(
            &logs,
            "device",
            timestamp,
            level,
            message,
            JsonObject::empty(),
        )
        .await?;
    }
    Ok(logs)
}

async fn insert_source(
    logs: &MemoryLogs,
    id: &str,
    name: &str,
    description: &str,
) -> Result<(), A2aLabError> {
    logs.insert_source(LogSource {
        id: SourceId::new(id)?,
        name: name.to_owned(),
        description: description.to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    Ok(())
}

async fn insert_record(
    logs: &MemoryLogs,
    source: &str,
    timestamp: &str,
    level: LogLevel,
    message: &str,
    attributes: JsonObject,
) -> Result<(), A2aLabError> {
    logs.insert_record(LogRecord {
        source_id: SourceId::new(source)?,
        timestamp: UtcTimestamp::parse(timestamp)?,
        level,
        message: message.to_owned(),
        attributes,
    })
    .await
}

async fn seed_metrics() -> Result<MemoryMetrics, A2aLabError> {
    let metrics = MemoryMetrics::new();
    seed_metric(
        &metrics,
        "door",
        "Door",
        "Door state",
        "open",
        &[
            ("2024-01-01T00:00:00Z", 0.0),
            ("2024-01-01T00:00:10Z", 0.0),
            ("2024-01-01T00:00:20Z", 1.0),
            ("2024-01-01T00:00:30Z", 1.0),
            ("2024-01-01T00:01:00Z", 0.0),
            ("2024-01-01T00:01:30Z", 0.0),
            ("2024-01-01T00:02:00Z", 1.0),
            ("2024-01-01T00:03:00Z", 0.0),
        ],
    )
    .await?;
    seed_metric(
        &metrics,
        "latency",
        "Latency",
        "Request latency",
        "ms",
        &[
            ("2024-01-01T00:00:00Z", 5.0),
            ("2024-01-01T00:00:15Z", 8.0),
            ("2024-01-01T00:00:30Z", 12.0),
            ("2024-01-01T00:01:00Z", 4.0),
            ("2024-01-01T00:02:00Z", 6.0),
            ("2024-01-01T00:03:00Z", 7.0),
        ],
    )
    .await?;
    seed_metric(
        &metrics,
        "temperature",
        "Temperature",
        "Bench temperature",
        "C",
        &[
            ("2024-01-01T00:00:00Z", 21.5),
            ("2024-01-01T00:01:00Z", 21.6),
            ("2024-01-01T00:02:00Z", 22.0),
            ("2024-01-01T00:03:00Z", 21.8),
        ],
    )
    .await?;
    Ok(metrics)
}

async fn seed_metric(
    metrics: &MemoryMetrics,
    id: &str,
    name: &str,
    description: &str,
    unit: &str,
    points: &[(&str, f64)],
) -> Result<(), A2aLabError> {
    let metric_id = MetricId::new(id)?;
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: name.to_owned(),
            description: description.to_owned(),
            unit: unit.to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    for (timestamp, value) in points {
        metrics
            .insert_point(
                metric_id.clone(),
                MetricPoint::new(UtcTimestamp::parse(timestamp)?, *value)?,
            )
            .await?;
    }
    Ok(())
}

const BUILD_INPUT_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["name"],
  "properties": {
    "name": {
      "type": "string",
      "title": "Name",
      "minLength": 2,
      "pattern": "^[a-z]+$",
      "default": "lab"
    },
    "count": {
      "type": "integer",
      "title": "Count",
      "minimum": 1,
      "maximum": 9,
      "default": 1
    },
    "ratio": {"type": "number", "title": "Ratio", "minimum": 0.5},
    "enabled": {"type": "boolean", "title": "Enabled", "default": true},
    "mode": {"type": "string", "title": "Mode", "enum": ["fast", "slow"]},
    "day": {"type": "string", "format": "date", "title": "Day"},
    "config": {
      "type": "object",
      "title": "Config",
      "properties": {
        "retries": {"type": "integer", "title": "Retries", "default": 2}
      }
    },
    "tags": {"type": "array", "title": "Tags", "items": {"type": "string"}},
    "choice": {"title": "Choice", "oneOf": [{"type": "string"}, {"type": "number"}]}
  }
}"#;

async fn seed_tasks() -> Result<MemoryTasks, A2aLabError> {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build")?,
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: Some(BUILD_INPUT_SCHEMA.to_owned()),
            output_schema: None,
        })
        .await;
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("home")?,
            name: "Home".to_owned(),
            description: "Home the axes".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    Ok(tasks)
}

fn complete_started(tasks: MemoryTasks) {
    tokio::spawn(async move {
        loop {
            for number in 1..=32 {
                let Ok(id) = RunId::new(format!("run-{number}")) else {
                    continue;
                };
                let Ok(run) = tasks.status(GetTaskStatusRequest { id: id.clone() }).await else {
                    continue;
                };
                let (state, message) = match run.state {
                    TaskState::Submitted => (TaskState::Working, Some("working".to_owned())),
                    TaskState::Working => (TaskState::Completed, Some("done".to_owned())),
                    _ => continue,
                };
                let _ = tasks.transition(&id, state, message).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
}

fn frame() -> Result<Image, A2aLabError> {
    Image::new(
        ImageDescriptor::new(
            ImageId::new("current")?,
            ImageSourceId::new("bench")?,
            UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
            "image/png",
            1,
            1,
            Some("Bench frame".to_owned()),
            JsonObject::empty(),
        )?,
        FRAME.to_vec(),
    )
}
