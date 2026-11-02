//! Pointer, keyboard, and time-field interactions on the Videos timeline.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, UtcTimestamp, VideoSource, VideoSourceId, bind_local,
};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];
const READY: &str = "document.getElementById('video-interval-0') && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped')";

#[tokio::test]
async fn browser_videos_interactions_click_drag_and_type() {
    let base = interaction_lab().await;
    tokio::task::spawn_blocking(move || interactions(&base))
        .await
        .unwrap();
}

fn interactions(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until(READY);
    assert_eq!(
        page.eval("!!document.getElementById('video-load-range')"),
        serde_json::json!(false),
        "the time fields still wait on a Load clip button"
    );

    let origin = attr(&mut page, "video-timeline", "data-start");
    drag_interval(&mut page, 0.15, 0.7);
    page.wait_until(
        "document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped')",
    );
    let dragged = attr(&mut page, "video-timeline", "data-start");
    assert_ne!(
        dragged, origin,
        "dragging the retained bar left the selection at {origin}"
    );
    assert!(
        dragged.as_str().unwrap_or("").contains("2024-01-01T00:00:"),
        "dragged selection {dragged} left the recording"
    );
    assert_eq!(
        page.eval("document.getElementById('video-alert-text')?.textContent.trim() ?? ''"),
        serde_json::json!("")
    );
    page.focus("video-timeline");
    page.eval(
        "(() => { const el = document.getElementById('video-timeline'); el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true })); return true; })()",
    );
    let nudged = attr(&mut page, "video-timeline", "data-start");
    assert_ne!(nudged, dragged, "arrow key did not move the selection");

    page.click_box("video-interval-0");
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.start?.includes('2024-01-01T00:00:00') && document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped')",
    );

    type_range(&mut page, "2024-01-01T00:00:10Z", "2024-01-01T00:00:12Z");
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.start?.includes('2024-01-01T00:00:10') && document.getElementById('video-timeline')?.dataset.end?.includes('2024-01-01T00:00:12') && document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped')",
    );
}

fn drag_interval(page: &mut Browser, from_ratio: f64, to_ratio: f64) {
    page.focus("video-timeline");
    let bounds = page.eval(
        "(() => { const el = document.getElementById('video-interval-0'); const rect = el.getBoundingClientRect(); return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }; })()",
    );
    let x = bounds["x"].as_f64().unwrap();
    let y = bounds["y"].as_f64().unwrap() + bounds["height"].as_f64().unwrap() / 2.0;
    let width = bounds["width"].as_f64().unwrap();
    page.drag(x + width * from_ratio, y, x + width * to_ratio, y);
}

fn type_range(page: &mut Browser, start: &str, end: &str) {
    let start = serde_json::to_string(start).unwrap_or_default();
    let end = serde_json::to_string(end).unwrap_or_default();
    let entered = page.eval(&format!(
        "(() => {{ const start = document.getElementById('video-range-start'); const end = document.getElementById('video-range-end'); const apply = document.getElementById('video-range-apply'); if (!(start instanceof HTMLInputElement) || !(end instanceof HTMLInputElement) || !(apply instanceof HTMLButtonElement)) return false; start.value = {start}; end.value = {end}; start.dispatchEvent(new Event('input', {{ bubbles: true }})); end.dispatchEvent(new Event('input', {{ bubbles: true }})); apply.click(); return true; }})()"
    ));
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
}

fn attr(page: &mut Browser, id: &str, name: &str) -> serde_json::Value {
    let id = serde_json::to_string(id).unwrap_or_default();
    let name = serde_json::to_string(name).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.getAttribute({name}) ?? ''"
    ))
}

async fn interaction_lab() -> String {
    let videos = MemoryVideos::new();
    let source = VideoSourceId::new("cam").unwrap();
    videos
        .insert_source(
            VideoSource::new(
                source.clone(),
                "Camera",
                "Camera recording",
                None,
                None,
                1,
                1,
                1,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    for second in 0..=40 {
        videos
            .insert_frame(
                source.clone(),
                UtcTimestamp::parse(&format!("2024-01-01T00:00:{second:02}Z")).unwrap(),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
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
