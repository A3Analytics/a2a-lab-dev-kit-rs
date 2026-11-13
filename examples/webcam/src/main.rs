//! Serves the web interface through the declarative video runtime.
//!
//! Open the printed `/ui/` URL and choose Videos. Capture, live H.264 retention,
//! and optional SQLite DVR are composed by [`DeclarativeVideoRuntime`].

use std::io::{Write, stdout};

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, CameraSelector, ConfigDuration, DeclarativeVideoRuntime,
    MemoryLogs, MemoryMetrics, MemoryTasks, VideoConfig, VideoInputFormat, VideoRuntimeConfig,
    VideoRuntimeInjections, VideoSourceConfig, VideoSourceId, WEB_UI_PATH,
};
use tokio::net::TcpListener;

const DEFAULT_BIND: &str = "127.0.0.1:3847";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = bind().await?;
    let address = listener.local_addr()?;
    let storage = std::env::temp_dir().join(format!(
        "a2a-lab-webcam-dvr-{}",
        std::process::id()
    ));
    let runtime = DeclarativeVideoRuntime::start(
        VideoRuntimeConfig::new(webcam_config(), &storage),
        VideoRuntimeInjections::none(),
    )
    .await?;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(runtime.provider())
        .share();
    writeln!(stdout(), "http://{address}{WEB_UI_PATH}")?;
    stdout().flush()?;
    A2aServer::new(&service)
        .with_web_ui()
        .with_capture_settings(runtime.settings_handle())
        .listen(listener)
        .await?;
    Ok(())
}

fn webcam_config() -> VideoConfig {
    VideoConfig {
        sources: vec![VideoSourceConfig {
            id: VideoSourceId::new("webcam").expect("id"),
            name: Some("Webcam".to_owned()),
            description: Some("Local camera".to_owned()),
            format: VideoInputFormat::Auto,
            camera_id: Some(CameraSelector::Index(camera_index())),
            width: None,
            height: None,
            capture_rate: None,
            semantic_indexing: None,
        }],
        fps: Some(5),
        live_retention: Some(ConfigDuration::parse("videos.live_retention", "120s").expect("120s")),
        dvr: None,
    }
}

fn camera_index() -> u32 {
    std::env::var("WEB_UI_WEBCAM_INDEX")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

async fn bind() -> Result<TcpListener, Box<dyn std::error::Error + Send + Sync>> {
    let address = std::env::var("WEB_UI_BIND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BIND.to_owned());
    Ok(TcpListener::bind(&address).await?)
}
