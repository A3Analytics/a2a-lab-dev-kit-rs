//! Videos playback speed control in the browser shell.

#![cfg(feature = "web-ui")]

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/video_playback_bench.rs"]
mod video_playback_bench;

use browser::Browser;
use video_playback_bench::{bench_lab, load_range_js};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";

#[tokio::test]
async fn browser_video_speed_control_preserves_playback_state() {
    let base = bench_lab_url().await;
    tokio::task::spawn_blocking(move || speed_flow(&base))
        .await
        .unwrap();
}

fn speed_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until("document.getElementById('video-speed')");
    assert_eq!(
        page.eval(
            "(() => { const select = document.getElementById('video-speed'); return select?.value === '1x' && select?.options?.length === 5; })()"
        ),
        serde_json::json!(true)
    );
    page.wait_until("document.getElementById('video-interval-0')");
    load_range(&mut page, "2024-01-01T00:00:00Z", "2024-01-01T00:01:00Z");
    page.wait_until(
        "(() => { const state = document.getElementById('video-status')?.dataset.state; if (state === 'paused') return true; if (state !== 'playing') return false; const player = document.getElementById('video-player'); if (player instanceof HTMLVideoElement) player.pause(); if (typeof window.__a2aNativePause === 'function') window.__a2aNativePause(); return false; })()",
    );
    let before = page.eval(
        "(() => ({ source: document.getElementById('video-source')?.value ?? '', start: document.getElementById('video-timeline')?.dataset.start ?? '', end: document.getElementById('video-timeline')?.dataset.end ?? '', frame: document.getElementById('video-status')?.dataset.frameTime ?? '' }))()",
    );
    for rate in ["0.5x", "2x", "4x", "8x", "1x"] {
        page.eval(&format!(
            "(() => {{ const select = document.getElementById('video-speed'); select.value = '{rate}'; select.dispatchEvent(new Event('change', {{ bubbles: true }})); }})()"
        ));
        assert_eq!(
            page.eval("document.getElementById('video-speed')?.value"),
            serde_json::json!(rate)
        );
        assert_eq!(
            page.eval("document.getElementById('video-status')?.dataset.state"),
            serde_json::json!("paused"),
            "pause state should survive a {rate} change"
        );
    }
    let after = page.eval(
        "(() => ({ source: document.getElementById('video-source')?.value ?? '', start: document.getElementById('video-timeline')?.dataset.start ?? '', end: document.getElementById('video-timeline')?.dataset.end ?? '', frame: document.getElementById('video-status')?.dataset.frameTime ?? '' }))()",
    );
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["start"], before["start"]);
    assert_eq!(after["end"], before["end"]);
    assert_eq!(after["frame"], before["frame"]);
    page.eval("(() => { const select = document.getElementById('video-speed'); select.value = '0.5x'; select.dispatchEvent(new Event('change', { bubbles: true })); })()");
    page.wait_until("document.getElementById('video-speed')?.value === '0.5x'");
    page.click("video-follow-newest");
    page.wait_until(
        "document.getElementById('video-follow-newest')?.classList.contains('video-live-active')",
    );
    page.wait_until("document.getElementById('video-speed')?.value === '1x'");
    assert_eq!(
        page.eval(
            "(() => { const select = document.getElementById('video-speed'); return select && [...select.options].filter((option) => option.value !== '1x').every((option) => option.disabled); })()"
        ),
        serde_json::json!(true)
    );
}

fn load_range(page: &mut Browser, start: &str, end: &str) {
    let entered = page.eval(&load_range_js(start, end));
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
}

async fn bench_lab_url() -> String {
    bench_lab().await
}
