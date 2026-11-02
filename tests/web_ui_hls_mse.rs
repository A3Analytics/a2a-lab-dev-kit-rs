//! Browser MSE checks for authenticated gateway HLS playback.

#![cfg(feature = "web-ui")]

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/hls_playback_bench.rs"]
mod hls_playback_bench;

use browser::{Browser, header, request_method, request_url};
use hls_playback_bench::{BEARER_TOKEN, bench_lab};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";

#[tokio::test]
async fn browser_hls_mse_uses_gateway_fetch_and_cleanup() {
    let base = bench_lab().await;
    tokio::task::spawn_blocking(move || hls_flow(&base))
        .await
        .unwrap();
}

fn hls_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.wait_until("document.getElementById('bearer-token')");
    page.set_value("bearer-token", BEARER_TOKEN);
    page.eval("(() => { const el = document.getElementById('bearer-token'); el?.dispatchEvent(new Event('input', { bubbles: true })); })()");
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-source-status')?.dataset.state === 'ready' && document.getElementById('video-source')?.value === 'enc-live'",
    );
    page.wait_until("document.getElementById('video-timeline')?.dataset.ready === 'true'");
    page.click("video-follow-newest");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'connecting'",
    );
    page.wait_until("(() => { const trace = window.__a2aHlsTrace; return trace && trace.appendOrder && trace.appendOrder.length >= 2; })()");
    page.wait_until(
        "(() => (window.__a2aHlsTrace?.appendOrder ?? []).some((item) => item.endsWith('.m4s')))()",
    );
    let player_src = page
        .eval("document.getElementById('video-player')?.getAttribute('src') ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(
        !player_src.contains("index.m3u8"),
        "playlist must not be assigned directly to the video element"
    );
    assert!(
        player_src.starts_with("blob:") || player_src.is_empty(),
        "expected blob MediaSource URL, got {player_src}"
    );
    let media_requests: Vec<_> = page
        .requests(base)
        .into_iter()
        .filter(|request| {
            let url = request_url(request);
            url.contains("/media/video-live/") || url.contains("/media/video-clips/")
        })
        .collect();
    assert!(
        media_requests.len() >= 3,
        "expected manifest, init, and segment fetches"
    );
    let bearer = format!("Bearer {BEARER_TOKEN}");
    for request in &media_requests {
        assert_eq!(request_method(request), "GET");
        assert_eq!(header(request, "Authorization"), Some(bearer.as_str()));
        let url = request_url(request);
        assert!(!url.contains("token="), "credential leaked into URL {url}");
        assert!(!url.contains("Bearer"), "credential leaked into URL {url}");
    }
    let trace = page.eval("window.__a2aHlsTrace");
    let order = trace["appendOrder"].as_array().expect("append order");
    assert_eq!(order.first().and_then(|v| v.as_str()), Some("index.m3u8"));
    let segment_in_trace = order
        .iter()
        .any(|item| item.as_str().is_some_and(hls_playback_bench::is_m4s_uri));
    let segment_fetched = page.requests(base).into_iter().any(|request| {
        request_url(&request).contains("/media/video-live/")
            && std::path::Path::new(request_url(&request))
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("m4s"))
    });
    assert!(
        segment_in_trace || segment_fetched,
        "segment gateway fetch or append missing"
    );
    page.eval("(() => { const select = document.getElementById('video-speed'); select.value = '2x'; select.dispatchEvent(new Event('change', { bubbles: true })); })()");
    page.wait_until("document.getElementById('video-speed')?.value === '2x'");
    let rate = page
        .eval("document.getElementById('video-player')?.playbackRate ?? 0")
        .as_f64()
        .unwrap_or(0.0);
    assert!((rate - 2.0).abs() < 0.01, "playbackRate at 2x was {rate}");
    page.eval("(() => { const select = document.getElementById('video-speed'); select.value = '8x'; select.dispatchEvent(new Event('change', { bubbles: true })); })()");
    page.wait_until("document.getElementById('video-speed')?.value === '8x'");
    page.wait_until("(() => { const order = window.__a2aHlsTrace?.appendOrder ?? []; const note = document.getElementById('video-status')?.textContent ?? ''; return order.includes('index-8x.m3u8') || note.includes('1x'); })()");
    page.wait_until("(() => { const order = window.__a2aHlsTrace?.appendOrder ?? []; return order.filter((item) => item.endsWith('.m3u8')).length >= 2; })()");
    page.click("video-follow-newest");
    page.wait_until("window.__a2aHlsTrace?.cleaned === true");
    let leaked = page.eval("(() => { const player = document.getElementById('video-player'); return (player?.src ?? '').startsWith('blob:'); })()");
    assert_eq!(
        leaked,
        serde_json::json!(false),
        "object URL leaked after teardown"
    );
}
