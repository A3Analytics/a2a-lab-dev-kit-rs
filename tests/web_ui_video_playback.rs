//! Playback paints successive bench frames, holds one while paused, and stops at the typed range.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/video_playback_bench.rs"]
mod video_playback_bench;

use browser::Browser;
use video_playback_bench::{bench_catalog, bench_lab, load_range_js, serve_bench};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const PAINTED: &str = "(() => { const canvas = document.getElementById('video-canvas'); if (!canvas || canvas.width !== 320 || canvas.height !== 192) return ''; const ctx = canvas.getContext('2d'); const hits = []; for (let block = 0; block < 10; block += 1) { const data = ctx.getImageData(block * 8 + 4, 48, 1, 1).data; if (data[0] > 200 && data[1] > 140 && data[1] < 220 && data[2] < 90) hits.push(block); } return hits.join(','); })()";

#[tokio::test]
async fn browser_video_playback_paints_moves_and_pauses() {
    let base = bench_lab().await;
    tokio::task::spawn_blocking(move || play(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_video_playback_trims_leading_retention() {
    let base = trimmed_bench_lab().await;
    tokio::task::spawn_blocking(move || assert_leading_retention(&base))
        .await
        .unwrap();
}

#[allow(clippy::cognitive_complexity)]
fn play(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-interval-0') && document.getElementById('video-range-end')",
    );
    assert_eq!(
        page.eval(&local_clock(
            "document.getElementById('video-available-start')",
            "2024-01-01T00:00:00Z",
        )),
        serde_json::json!(true)
    );
    assert_eq!(
        page.eval(&local_clock(
            "document.getElementById('video-available-end')",
            "2024-01-01T00:05:00Z",
        )),
        serde_json::json!(true)
    );
    assert_eq!(
        page.eval(
            "(() => { const utcStamp = (iso) => { const d = new Date(iso); if (Number.isNaN(d.getTime())) return iso; const pad = (n) => String(n).padStart(2, '0'); return `${d.getUTCFullYear()}-${pad(d.getUTCMonth()+1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())} UTC`; }; const el = document.getElementById('video-timeline'); const text = el?.getAttribute('aria-valuetext') ?? ''; const start = el?.dataset.start || ''; const end = el?.dataset.end || ''; return start.length > 0 && end.length > 0 && text === `${utcStamp(start)} to ${utcStamp(end)}`; })()"
        ),
        serde_json::json!(true)
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
        serde_json::json!(false),
        "player auto-starts on the newest retained window"
    );
    load_range(&mut page, "2024-01-01T00:00:00Z", "2024-01-01T00:00:04Z");
    page.wait_until(&painted(
        "PAINTED !== '' && !document.getElementById('video-player')?.hasAttribute('hidden')",
    ));
    pause_when_painted(&mut page);
    page.eval("new Promise((resolve) => setTimeout(resolve, 400))");
    let held = painted_block(&mut page);
    let held_time = frame_time(&mut page);
    assert!(
        held_time.contains("T00:00:00"),
        "playback started at {held_time}, after the beginning of the range"
    );
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
        serde_json::json!(320)
    );
    assert_eq!(
        page.eval("document.getElementById('video-canvas')?.height"),
        serde_json::json!(192)
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

fn local_clock(element: &str, utc: &str) -> String {
    format!(
        "(() => {{ const utcStamp = (iso) => {{ const d = new Date(iso); if (Number.isNaN(d.getTime())) return iso; const pad = (n) => String(n).padStart(2, '0'); return `${{d.getUTCFullYear()}}-${{pad(d.getUTCMonth()+1)}}-${{pad(d.getUTCDate())}} ${{pad(d.getUTCHours())}}:${{pad(d.getUTCMinutes())}}:${{pad(d.getUTCSeconds())}} UTC`; }}; const el = {element}; const utc = el?.dateTime ?? ''; return el?.textContent.trim() === utcStamp(utc) && utc === '{utc}'; }})()"
    )
}

fn pause_when_painted(page: &mut Browser) {
    page.wait_until(&painted(
        "(() => { const block = PAINTED; if (!block) return false; const player = document.getElementById('video-player'); if (document.getElementById('video-status')?.dataset.state !== 'paused') { if (player instanceof HTMLVideoElement) player.pause(); if (typeof window.__a2aNativePause === 'function') window.__a2aNativePause(); return false; } return true; })()",
    ));
}

fn painted(expression: &str) -> String {
    expression.replace("PAINTED", &format!("({PAINTED})"))
}

fn resume(page: &mut Browser) {
    let started = page.eval(
        "(() => { const el = document.getElementById('video-player'); if (!(el instanceof HTMLVideoElement)) return false; const playing = el.play(); if (playing && playing.catch) playing.catch(() => {}); if (typeof window.__a2aNativePlay === 'function') window.__a2aNativePlay(); else el.dispatchEvent(new Event('play')); return true; })()",
    );
    assert_eq!(started, serde_json::json!(true), "missing video player");
}

fn load_range(page: &mut Browser, start: &str, end: &str) {
    let entered = page.eval(&load_range_js(start, end));
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
}

fn painted_block(page: &mut Browser) -> String {
    page.eval(&format!("({PAINTED})"))
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn frame_time(page: &mut Browser) -> String {
    page.wait_until(
        "(() => { const value = document.getElementById('video-status')?.dataset.frameTime ?? ''; return value !== '' && value !== 'none'; })()",
    );
    page.eval("document.getElementById('video-status')?.dataset.frameTime ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn detail_utc(page: &mut Browser, label: &str) -> String {
    let label = serde_json::to_string(label).unwrap_or_default();
    page.eval(&format!(
        "document.querySelector('#video-details [data-label={label}]')?.dataset.utc ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn assert_leading_retention(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until("document.getElementById('video-interval-0')");
    assert_eq!(
        page.eval(&local_clock(
            "document.getElementById('video-available-start')",
            "2024-01-01T00:03:00Z",
        )),
        serde_json::json!(true)
    );
    load_range(&mut page, "2024-01-01T00:02:30Z", "2024-01-01T00:03:30Z");
    page.wait_until(
        "document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped') && !document.getElementById('video-player')?.hasAttribute('hidden')",
    );
    let start = detail_utc(&mut page, "Start");
    let end = detail_utc(&mut page, "End");
    assert!(
        page.eval("document.getElementById('video-timeline')?.dataset.start ?? ''")
            .as_str()
            .unwrap_or("")
            .contains("2024-01-01T00:02:30"),
        "requested range was not kept on the timeline"
    );
    assert!(
        start.contains("2024-01-01T00:03:00"),
        "effective start {start}"
    );
    assert!(
        !start.contains("2024-01-01T00:02:30"),
        "effective start must not include trimmed retention {start}"
    );
    assert!(end.contains("2024-01-01T00:03"), "effective end {end}");
}

async fn trimmed_bench_lab() -> String {
    let videos = bench_catalog().await;
    videos
        .forget_before(
            &VideoSourceId::new("bench").unwrap(),
            UtcTimestamp::parse("2024-01-01T00:03:00Z").unwrap(),
        )
        .await
        .unwrap();
    serve_bench(videos).await
}
