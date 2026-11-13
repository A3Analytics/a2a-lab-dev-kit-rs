//! Hardware-free checks for `/config/settings`.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::videos::SENSOR_30FPS_PERIOD_NS;
use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, CameraSelector, ConfigDuration, MemoryLogs, MemoryMetrics,
    MemoryTasks, SaveIntervalDuration, VideoCaptureSettings, VideoCaptureSettingsHandle,
    VideoConfig, VideoDvrConfig, VideoInputFormat, VideoSourceConfig, VideoSourceId, bind_local,
};
use serde_json::json;

#[tokio::test]
async fn settings_shell_route_is_public() {
    let handle = settings_handle();
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_video_capture_settings(handle);
    tokio::spawn(async move { server.listen(Some(listener)).await.unwrap() });
    let base = format!("http://{address}");
    let response = reqwest::get(format!("{base}/ui/config/settings"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn settings_route_serves_sources_and_applies_live_fps_only() {
    let handle = settings_handle();
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_video_capture_settings(handle.clone());
    tokio::spawn(async move { server.listen(Some(listener)).await.unwrap() });

    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    let read = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let body: serde_json::Value = read.json().await.unwrap();
    assert_eq!(body["sources"][0]["id"], "bench");
    assert_eq!(body["sources"][0]["camera_id"], 4);
    assert_eq!(body["configured_videos_fps"], 5);
    assert_eq!(body["live_output_fps"], 5);
    assert_eq!(body["dvr_retention_secs"], 3_600);
    assert_eq!(body["dvr_save_interval"], "1s");

    let ok = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "videos": { "fps": 12 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(handle.snapshot().live_output_fps(), 12);
    assert_eq!(handle.snapshot().configured_videos_fps(), Some(5));
    assert_eq!(handle.snapshot().view().dvr_retention_secs, Some(3_600));
    assert_eq!(
        handle.snapshot().view().dvr_save_interval.as_deref(),
        Some("1s")
    );

    let ok_interval = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "videos": { "dvr": { "save_interval": "500ms" } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok_interval.status(), 200);
    assert_eq!(
        handle.snapshot().view().dvr_save_interval.as_deref(),
        Some("500ms")
    );

    let rejected = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "videos": { "fps": 0 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    assert_eq!(handle.snapshot().live_output_fps(), 12);

    let err = rejected.json::<serde_json::Value>().await.unwrap();
    assert_eq!(err["error"]["code"], "invalid");
}

#[tokio::test]
async fn settings_route_rejects_too_fast_save_interval() {
    let handle = settings_handle();
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_video_capture_settings(handle.clone());
    tokio::spawn(async move { server.listen(Some(listener)).await.unwrap() });

    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    let rejected = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "videos": { "dvr": { "save_interval": "10ms" } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    assert_eq!(
        handle.snapshot().view().dvr_save_interval.as_deref(),
        Some("1s")
    );
}

fn settings_handle() -> VideoCaptureSettingsHandle {
    let config = VideoConfig {
        sources: vec![VideoSourceConfig {
            id: VideoSourceId::new("bench").unwrap(),
            name: Some("Bench".to_owned()),
            description: None,
            format: VideoInputFormat::Mjpeg,
            camera_id: Some(CameraSelector::Index(4)),
            width: Some(640),
            height: Some(480),
            capture_rate: Some(30),
            semantic_indexing: None,
        }],
        fps: Some(5),
        live_retention: ConfigDuration::parse("videos.live_retention", "30s").ok(),
        dvr: Some(VideoDvrConfig {
            retention: ConfigDuration::parse("videos.dvr.retention", "1h").ok(),
            save_interval: SaveIntervalDuration::parse("videos.dvr.save_interval", "1s").ok(),
            storage_path: Some("./dvr".to_owned()),
        }),
    };
    let settings = VideoCaptureSettings::from_video_config(Some(&config))
        .with_negotiated_sensor_period_ns(SENSOR_30FPS_PERIOD_NS);
    VideoCaptureSettingsHandle::new(settings)
}
