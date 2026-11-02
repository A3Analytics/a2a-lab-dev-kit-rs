//! Shared bench catalog for browser video playback tests.

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, UtcTimestamp, VideoSource, VideoSourceId, bind_local,
};

#[path = "../../examples/web_ui/bench_jpeg.rs"]
mod bench_jpeg;

pub async fn bench_catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new("bench").unwrap(),
                "Bench",
                "Bench camera recording",
                None,
                None,
                bench_jpeg::FRAME_WIDTH,
                bench_jpeg::FRAME_HEIGHT,
                4,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    for index in 0..bench_jpeg::FRAME_COUNT {
        videos
            .insert_frame(
                VideoSourceId::new("bench").unwrap(),
                UtcTimestamp::parse(&bench_jpeg::captured_at(index)).unwrap(),
                bench_jpeg::frame(index),
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&VideoSourceId::new("bench").unwrap(), true)
        .await
        .unwrap();
    videos
}

pub async fn serve_bench(videos: MemoryVideos) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

pub async fn bench_lab() -> String {
    serve_bench(bench_catalog().await).await
}

pub fn bench_newest_stamp() -> String {
    bench_jpeg::captured_at(bench_jpeg::FRAME_COUNT - 1)
}

pub fn bench_stamp_before_newest(whole_seconds: u32) -> String {
    let frames = whole_seconds.saturating_mul(4);
    let index = bench_jpeg::FRAME_COUNT
        .saturating_sub(1)
        .saturating_sub(frames);
    bench_jpeg::captured_at(index)
}

pub fn load_range_js(start: &str, end: &str) -> String {
    let start = serde_json::to_string(start).unwrap_or_default();
    let end = serde_json::to_string(end).unwrap_or_default();
    format!(
        "(() => {{ const start = document.getElementById('video-range-start'); const end = document.getElementById('video-range-end'); const apply = document.getElementById('video-range-apply'); if (!(start instanceof HTMLInputElement) || !(end instanceof HTMLInputElement) || !(apply instanceof HTMLButtonElement)) return false; start.value = {start}; end.value = {end}; start.dispatchEvent(new Event('input', {{ bubbles: true }})); end.dispatchEvent(new Event('input', {{ bubbles: true }})); apply.click(); return true; }})()"
    )
}
