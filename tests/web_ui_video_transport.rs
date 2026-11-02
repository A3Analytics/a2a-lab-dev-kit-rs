//! Pause control beside the seek bar, and the live-edge speed lock.

#![cfg(feature = "web-ui")]

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/video_playback_bench.rs"]
mod video_playback_bench;

use browser::Browser;
use video_playback_bench::{
    bench_lab, bench_newest_stamp, bench_stamp_before_newest, load_range_js,
};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";

#[tokio::test]
async fn browser_video_pause_sits_by_the_seek_bar_and_live_edge_locks_speed() {
    let base = bench_lab().await;
    tokio::task::spawn_blocking(move || transport_flow(&base))
        .await
        .unwrap();
}

fn transport_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-pause')?.getAttribute('aria-label') === 'Pause' && document.querySelector('#video-pause rect') && document.getElementById('video-status')?.dataset.state === 'playing'",
    );
    assert_eq!(
        page.eval(
            "(() => { const button = document.getElementById('video-pause'); const track = document.querySelector('.video-seek .video-track'); if (!button || !track || button.parentElement !== track.parentElement) return false; const buttonBox = button.getBoundingClientRect(); const trackBox = track.getBoundingClientRect(); const buttonMid = (buttonBox.top + buttonBox.bottom) / 2; const trackMid = (trackBox.top + trackBox.bottom) / 2; return buttonBox.right <= trackBox.left + 1 && Math.abs(buttonMid - trackMid) < 12; })()"
        ),
        serde_json::json!(true),
        "pause should sit on the seek row, to the left of the bar"
    );
    assert_eq!(
        page.eval("!document.getElementById('video-newest')"),
        serde_json::json!(true)
    );

    let start = page.eval("document.getElementById('video-timeline')?.dataset.start ?? ''");
    page.click("video-pause");
    page.wait_until(
        "document.getElementById('video-pause')?.getAttribute('aria-label') === 'Play' && document.querySelector('#video-pause polygon') && document.getElementById('video-pause')?.getAttribute('aria-pressed') === 'true' && document.getElementById('video-status')?.dataset.state === 'paused'",
    );
    assert_eq!(
        page.eval(
            "(async () => { const frame = document.getElementById('video-status')?.dataset.frameTime ?? ''; await new Promise((resolve) => setTimeout(resolve, 700)); const status = document.getElementById('video-status'); return status?.dataset.state === 'paused' && status?.dataset.frameTime === frame; })()"
        ),
        serde_json::json!(true),
        "a paused frame should stay put"
    );
    assert_eq!(
        page.eval("document.getElementById('video-timeline')?.dataset.start ?? ''"),
        start,
        "the pause control should not seek"
    );
    page.click("video-pause");
    page.wait_until(
        "document.getElementById('video-pause')?.getAttribute('aria-label') === 'Pause' && document.querySelector('#video-pause rect') && document.getElementById('video-status')?.dataset.state === 'playing'",
    );

    load_range(&mut page, "2024-01-01T00:00:00Z", "2024-01-01T00:00:08Z");
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'playing'");
    set_speed(&mut page, "4x");
    page.wait_until("document.getElementById('video-speed')?.value === '4x'");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.frameTime?.includes('2024-01-01T00:00:0') && document.getElementById('video-playhead') && !document.getElementById('video-newest')",
    );
    assert_eq!(
        page.eval(
            "(() => { const select = document.getElementById('video-speed'); return select?.value === '4x' && [...select.options].every((option) => !option.disabled); })()"
        ),
        serde_json::json!(true),
        "a recording behind the live edge keeps every speed"
    );
    assert_eq!(
        page.eval(
            "(() => { const head = document.getElementById('video-playhead'); const track = document.querySelector('.video-track'); if (!head || !track || document.getElementById('video-newest')) return false; const headBox = head.getBoundingClientRect(); const trackBox = track.getBoundingClientRect(); return headBox.left < trackBox.left + trackBox.width * 0.5; })()"
        ),
        serde_json::json!(true),
        "the current-time mark follows the frame instead of staying at the end"
    );

    let edge = bench_newest_stamp();
    let approach = bench_stamp_before_newest(20);
    load_range(&mut page, &approach, &edge);
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'playing'");
    set_speed(&mut page, "8x");
    page.wait_until("document.getElementById('video-speed')?.value === '8x'");
    page.wait_until(
        "(() => { const select = document.getElementById('video-speed'); return select?.value === '1x' && [...select.options].filter((option) => option.value !== '1x').every((option) => option.disabled); })()",
    );
}

fn set_speed(page: &mut Browser, rate: &str) {
    page.eval(&format!(
        "(() => {{ const select = document.getElementById('video-speed'); select.value = '{rate}'; select.dispatchEvent(new Event('change', {{ bubbles: true }})); }})()"
    ));
}

fn load_range(page: &mut Browser, start: &str, end: &str) {
    let entered = page.eval(&load_range_js(start, end));
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
}
