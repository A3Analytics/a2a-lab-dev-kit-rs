//! Declarative runtime composition and server settings wiring (TASK-147).

#![cfg(all(feature = "yuv", feature = "openh264", feature = "web-ui"))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CameraDiscoverySnapshot, CameraEnumerator, CaptureFormat,
    CaptureMode, CaptureRate, ColorCamera, RawCameraSample, SelectedCaptureProfile,
};
use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, CAPTURE_SETTINGS_PATH, CameraSelector, DeclarativeVideoRuntime,
    ListVideoSourcesRequest, MemoryLogs, MemoryMetrics, MemoryTasks, PageRequest, VideoConfig,
    VideoInputFormat, VideoProvider, VideoRuntimeConfig, VideoRuntimeInjections, VideoSourceConfig,
    VideoSourceId,
};

struct EmptyEnumerator;

impl CameraEnumerator for EmptyEnumerator {
    fn discover(&self) -> Result<CameraDiscoverySnapshot, a2a_lab_dev_kit::A2aLabError> {
        Ok(CameraDiscoverySnapshot::from_parts(Vec::new(), Vec::new()))
    }
}

use a2a_lab_dev_kit::bind_local;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-declarative-runtime-{name}-{}",
        std::process::id()
    ))
}

const JPEG: &[u8] = &[0xFF, 0xD8, 0x01, 0x00, 0xFF, 0xD9];

fn mjpeg_bytes(marker: u8) -> Vec<u8> {
    let mut jpeg = JPEG.to_vec();
    jpeg[3] = marker;
    jpeg
}

struct SequenceCamera {
    frames: Vec<Vec<u8>>,
    index: AtomicU64,
}

impl SequenceCamera {
    fn new(frames: Vec<Vec<u8>>) -> Self {
        Self {
            frames,
            index: AtomicU64::new(0),
        }
    }
}

impl ColorCamera for SequenceCamera {
    fn advertised(
        &mut self,
    ) -> Result<Vec<AdvertisedCaptureProfile>, a2a_lab_dev_kit::A2aLabError> {
        Ok(vec![AdvertisedCaptureProfile::new(
            CaptureMode::new(CaptureFormat::Mjpeg, 4, 4).expect("mode"),
            CaptureRate::fps(30).expect("rate"),
        )])
    }

    fn open(
        &mut self,
        selected: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, a2a_lab_dev_kit::A2aLabError> {
        Ok(*selected)
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, a2a_lab_dev_kit::A2aLabError> {
        let index = usize::try_from(self.index.fetch_add(1, Ordering::SeqCst)).unwrap_or(0);
        let bytes = self.frames.get(index).cloned().unwrap_or_else(|| {
            self.frames
                .last()
                .cloned()
                .unwrap_or_else(|| mjpeg_bytes(0))
        });
        Ok(RawCameraSample::new(CaptureFormat::Mjpeg, 4, 4, 0, bytes))
    }

    fn stop(&mut self) -> Result<(), a2a_lab_dev_kit::A2aLabError> {
        Ok(())
    }
}

fn sample_config() -> VideoConfig {
    VideoConfig {
        sources: vec![VideoSourceConfig {
            id: VideoSourceId::new("cam").expect("id"),
            name: Some("Injected".to_owned()),
            description: None,
            format: VideoInputFormat::Mjpeg,
            camera_id: Some(CameraSelector::Index(0)),
            width: Some(4),
            height: Some(4),
            capture_rate: Some(30),
            semantic_indexing: None,
        }],
        fps: Some(5),
        live_retention: None,
        dvr: None,
    }
}

#[tokio::test]
async fn runtime_composes_injected_capture_provider_and_still_cache() {
    let dir = temp_root("live");
    let _ = std::fs::remove_dir_all(&dir);
    let frames: Vec<_> = (0..60).map(mjpeg_bytes).collect();
    let injections =
        VideoRuntimeInjections::none().with_camera("cam", Box::new(SequenceCamera::new(frames)));
    let runtime =
        DeclarativeVideoRuntime::start(VideoRuntimeConfig::new(sample_config(), &dir), injections)
            .await
            .expect("start");
    let provider = runtime.provider();
    let source = VideoSourceId::new("cam").expect("id");
    let mut ready = false;
    for _ in 0..80 {
        let store_registered = provider.live_h264().get(&source).is_some();
        let warm_still = runtime.still_cache().latest("cam").is_some();
        if store_registered && warm_still {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready, "live store and warm still should register");
    runtime.shutdown_handle().stop();
    drop(runtime);
}

#[tokio::test]
#[cfg(feature = "web-ui")]
async fn server_with_capture_settings_serves_settings_route() {
    let dir = temp_root("settings");
    let _ = std::fs::remove_dir_all(&dir);
    let runtime = DeclarativeVideoRuntime::start(
        VideoRuntimeConfig::new(sample_config(), &dir),
        VideoRuntimeInjections::none()
            .with_camera("cam", Box::new(SequenceCamera::new(vec![mjpeg_bytes(1)]))),
    )
    .await
    .expect("start");
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(runtime.provider())
        .share();
    let (listener, address) = bind_local().await.expect("bind");
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_capture_settings(runtime.settings_handle());
    tokio::spawn(async move { server.listen(Some(listener)).await.expect("listen") });
    let response = reqwest::get(format!("http://{address}{CAPTURE_SETTINGS_PATH}"))
        .await
        .expect("get");
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn missing_camera_still_starts_bounded_runtime() {
    let dir = temp_root("missing");
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = sample_config();
    config.sources[0].camera_id = Some(CameraSelector::Index(99_999));
    let runtime = DeclarativeVideoRuntime::start(
        VideoRuntimeConfig::new(config, &dir),
        VideoRuntimeInjections::none().with_enumerator(Box::new(EmptyEnumerator)),
    )
    .await
    .expect("start without camera");
    let listed = runtime
        .provider()
        .list_video_sources(ListVideoSourcesRequest::new(PageRequest::default()).unwrap())
        .await
        .expect("list");
    assert_eq!(listed.items().len(), 1);
    runtime.shutdown_handle().stop();
}
