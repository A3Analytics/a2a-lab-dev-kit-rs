//! In-memory lab served with the optional web interface.
//!
//! The process prints a local `/ui/` URL and keeps listening. Open that URL
//! to browse logs, metrics, tasks, images, and an `agent-message` echo.

use std::io::{Write, stdout};
use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, Image, ImageDescriptor, ImageId, ImageSource,
    ImageSourceId, JsonObject, LogLevel, LogRecord, LogSource, MemoryImages, MemoryLogs,
    MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId, MetricPoint, SourceId, TaskDefinition,
    TaskId, UtcTimestamp, WEB_UI_PATH, bind_local,
};

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
    let (listener, address) = bind_local().await?;
    writeln!(stdout(), "http://{address}{WEB_UI_PATH}")?;
    stdout().flush()?;
    A2aServer::new(&service)
        .with_message_handler(Arc::new(Echo))
        .with_web_ui()
        .listen(listener)
        .await?;
    Ok(())
}

async fn lab() -> Result<Arc<dyn A2aLabApi>, A2aLabError> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app")?,
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
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
        logs.insert_record(LogRecord {
            source_id: SourceId::new("app")?,
            timestamp: UtcTimestamp::parse(timestamp)?,
            level,
            message: message.to_owned(),
            attributes,
        })
        .await?;
    }

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
            metric_id,
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:00:00Z")?, 5.0)?,
        )
        .await?;

    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build")?,
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;

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
