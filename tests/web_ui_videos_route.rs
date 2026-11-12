//! Videos browse/search routes and playback lifecycle outside Browse.

#![cfg(feature = "web-ui")]

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/video_playback_bench.rs"]
mod video_playback_bench;

use browser::Browser;
use video_playback_bench::bench_lab;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";

const TRACK_MEDIA: &str = r"( () => {
  const orig = window.fetch.bind(window);
  window.__mediaFetches = 0;
  window.__mediaInFlight = 0;
  window.fetch = async (input, init) => {
    const url = String(typeof input === 'string' ? input : (input && input.url) || '');
    if (url.includes('/media/video-live/') || url.includes('/media/video-clips/')) {
      window.__mediaFetches = (window.__mediaFetches || 0) + 1;
      window.__mediaInFlight = (window.__mediaInFlight || 0) + 1;
    }
    try {
      return await orig(input, init);
    } finally {
      if (url.includes('/media/video-live/') || url.includes('/media/video-clips/')) {
        window.__mediaInFlight = Math.max(0, (window.__mediaInFlight || 0) - 1);
      }
    }
  };
})();";

#[tokio::test]
async fn browser_videos_routes_stop_streaming_and_preserve_state() {
    let base = bench_lab().await;
    tokio::task::spawn_blocking(move || route_lifecycle_flow(&base))
        .await
        .unwrap();
}

#[allow(clippy::cognitive_complexity)]
fn route_lifecycle_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/videos"));
    page.wait_until(CONNECTED);
    page.eval(TRACK_MEDIA);
    page.wait_until("document.getElementById('video-interval-0')");
    page.wait_until("!document.getElementById('video-player')?.hasAttribute('hidden')");
    let playing_on_browse = page.eval("document.getElementById('video-status')?.dataset.state");
    let playing_on_browse = playing_on_browse.as_str().unwrap_or("");
    assert!(
        playing_on_browse == "playing" || playing_on_browse == "connecting",
        "browse should start playback, observed {playing_on_browse}"
    );
    page.click("video-search-tab");
    page.wait_until("location.pathname.endsWith('/videos/search')");
    page.wait_until("!document.getElementById('video-search-panel')?.hidden");
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'stopped'");
    page.eval("new Promise((resolve) => setTimeout(resolve, 400))");
    assert_eq!(
        page.eval("window.__mediaInFlight ?? 0"),
        serde_json::json!(0),
        "search must not keep browse media requests in flight"
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
        "browse panel should hide on search"
    );
    let search_query = page.eval(
        "(() => { const el = document.getElementById('video-semantic-query'); if (!(el instanceof HTMLInputElement)) return ''; el.value = 'bench motion'; el.dispatchEvent(new Event('input', {bubbles:true})); return el.value; })()",
    );
    page.click("video-browse-tab");
    page.wait_until(
        "location.pathname.endsWith('/videos') && !location.pathname.includes('/search')",
    );
    page.wait_until("!document.getElementById('video-browse-panel')?.hidden");
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'stopped'");
    page.eval("new Promise((resolve) => setTimeout(resolve, 400))");
    assert_eq!(
        page.eval("document.getElementById('video-status')?.dataset.state"),
        serde_json::json!("stopped"),
        "returning to browse must not auto-start streaming"
    );
    assert_eq!(
        page.eval("window.__mediaInFlight ?? 0"),
        serde_json::json!(0),
        "browse return must not keep media requests active"
    );
    page.show("#logs");
    page.wait_until("document.getElementById('view-title')?.textContent?.includes('Logs')");
    page.eval("new Promise((resolve) => setTimeout(resolve, 400))");
    assert_eq!(
        page.eval("window.__mediaInFlight ?? 0"),
        serde_json::json!(0),
        "leaving Videos must stop media requests"
    );
    page.show("#videos");
    page.wait_until("!document.getElementById('video-browse-panel')?.hidden");
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'stopped'");
    page.show("videos/search");
    page.wait_until("location.pathname.endsWith('/videos/search')");
    assert_eq!(
        page.eval("document.getElementById('video-semantic-query')?.value ?? ''"),
        search_query,
        "search query state should survive leaving and returning to Videos"
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-tab')?.getAttribute('aria-selected')"),
        serde_json::json!("false"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-tab')?.getAttribute('aria-selected')"),
        serde_json::json!("true"),
    );
}
