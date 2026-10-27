//! Playback paints successive bench frames, holds one while paused, and stops at the typed range.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, UtcTimestamp, VideoSource, VideoSourceId, bind_local,
};

#[path = "../examples/bench_jpeg.rs"]
mod bench_jpeg;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const PAINTED: &str = "(() => { const canvas = document.getElementById('video-canvas'); if (!canvas || canvas.width !== 160 || canvas.height !== 96) return ''; const ctx = canvas.getContext('2d'); const hits = []; for (let block = 0; block < 10; block += 1) { const data = ctx.getImageData(block * 8 + 4, 48, 1, 1).data; if (data[0] > 200 && data[1] > 140 && data[1] < 220 && data[2] < 90) hits.push(block); } return hits.join(','); })()";

#[tokio::test]
async fn browser_video_playback_paints_moves_and_pauses() {
    let base = bench_lab().await;
    tokio::task::spawn_blocking(move || play(&base))
        .await
        .unwrap();
}

fn play(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-interval-0') && document.getElementById('video-load-range')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-available-start')?.textContent.trim() ?? ''"),
        serde_json::json!("2024-01-01T00:00:00Z")
    );
    assert_eq!(
        page.eval("document.getElementById('video-available-end')?.textContent.trim() ?? ''"),
        serde_json::json!("2024-01-01T00:05:00Z")
    );
    assert_eq!(
        page.eval(
            "document.getElementById('video-timeline')?.getAttribute('aria-valuetext') ?? ''"
        ),
        serde_json::json!("Available from 2024-01-01T00:00:00Z to 2024-01-01T00:05:00Z")
    );
    let coverage = page
        .eval("document.getElementById('video-interval-0')?.getBoundingClientRect().width ?? 0")
        .as_f64()
        .unwrap_or(0.0);
    let track = page
        .eval("document.querySelector('.video-track')?.getBoundingClientRect().width ?? 0")
        .as_f64()
        .unwrap_or(0.0);
    assert!(
        track > 200.0 && coverage > track * 0.9,
        "coverage {coverage} does not span the track {track}"
    );
    assert_eq!(
        page.eval("document.getElementById('video-player')?.hasAttribute('hidden')"),
        serde_json::json!(true),
        "player is visible before a clip"
    );
    load_range(&mut page, "2024-01-01T00:00:00Z", "2024-01-01T00:00:04Z");
    page.wait_until(&painted(
        "PAINTED !== '' && document.getElementById('video-player')?.hasAttribute('controls') && !document.getElementById('video-player')?.hasAttribute('hidden')",
    ));
    pause_when_painted(&mut page);
    page.eval("new Promise((resolve) => setTimeout(resolve, 400))");
    let held = painted_block(&mut page);
    let held_time = frame_time(&mut page);
    page.eval("new Promise((resolve) => setTimeout(resolve, 500))");
    assert_eq!(
        page.eval("document.getElementById('video-status')?.dataset.state"),
        serde_json::json!("paused"),
        "pause did not hold"
    );
    assert_eq!(painted_block(&mut page), held, "paused frame changed");
    assert_eq!(frame_time(&mut page), held_time, "paused timestamp changed");
    resume(&mut page);
    page.wait_until(&painted(&format!(
        "PAINTED !== '' && PAINTED !== '{held}' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped')"
    )));
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'stopped'");
    assert!(
        !painted_block(&mut page).is_empty(),
        "last frame was cleared"
    );
    assert_eq!(
        page.eval("document.getElementById('video-canvas')?.width"),
        serde_json::json!(160)
    );
    assert_eq!(
        page.eval("document.getElementById('video-canvas')?.height"),
        serde_json::json!(96)
    );
    assert_eq!(
        page.eval("document.getElementById('video-alert-text')?.textContent.trim() ?? ''"),
        serde_json::json!("")
    );
    assert_eq!(
        page.eval("document.getElementById('video-timeline')?.dataset.start ?? ''"),
        serde_json::json!("2024-01-01T00:00:00Z")
    );
    assert_eq!(
        page.eval("document.getElementById('video-timeline')?.dataset.end ?? ''"),
        serde_json::json!("2024-01-01T00:00:04Z")
    );
}

fn pause_when_painted(page: &mut Browser) {
    page.wait_until(&painted(
        "(() => { const block = PAINTED; if (!block) return false; const player = document.getElementById('video-player'); if (document.getElementById('video-status')?.dataset.state !== 'paused') { if (player instanceof HTMLVideoElement) player.pause(); if (typeof window.__a2aNativePause === 'function') window.__a2aNativePause(); return false; } return true; })()",
    ));
}

fn painted(expression: &str) -> String {
    expression.replace("PAINTED", &format!("({script})", script = PAINTED))
}

fn resume(page: &mut Browser) {
    let started = page.eval(
        "(() => { const el = document.getElementById('video-player'); if (!(el instanceof HTMLVideoElement)) return false; const playing = el.play(); if (playing && playing.catch) playing.catch(() => {}); if (typeof window.__a2aNativePlay === 'function') window.__a2aNativePlay(); else el.dispatchEvent(new Event('play')); return true; })()",
    );
    assert_eq!(started, serde_json::json!(true), "missing video player");
}

fn load_range(page: &mut Browser, start: &str, end: &str) {
    let start = serde_json::to_string(start).unwrap_or_default();
    let end = serde_json::to_string(end).unwrap_or_default();
    let entered = page.eval(&format!(
        "(() => {{ const start = document.getElementById('video-range-start'); const end = document.getElementById('video-range-end'); if (!(start instanceof HTMLInputElement) || !(end instanceof HTMLInputElement)) return false; start.value = {start}; end.value = {end}; start.dispatchEvent(new Event('input', {{ bubbles: true }})); end.dispatchEvent(new Event('input', {{ bubbles: true }})); return true; }})()"
    ));
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
    page.click("video-load-range");
}

fn painted_block(page: &mut Browser) -> String {
    page.eval(&format!("({script})", script = PAINTED))
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn frame_time(page: &mut Browser) -> String {
    if page.eval("!!document.getElementById('video-details')") != serde_json::json!(true) {
        page.click("video-details-toggle");
        page.wait_until("document.getElementById('video-details')");
    }
    page.eval(
        "document.querySelector('#video-details dd[data-label=\"Frame at time\"]')?.textContent.trim() ?? ''",
    )
    .as_str()
    .unwrap_or("")
    .to_owned()
}

async fn bench_lab() -> String {
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
