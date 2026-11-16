//! Videos semantic search UI: capability gates, metadata/decision separation, and fixtures.

#![cfg(feature = "web-ui")]

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabResult, A2aLabService, A2aServer, DecisionEvaluationCoverage,
    DecisionEvidenceItem, DecisionIdempotencyKey, DecisionOutcome, DecisionSubject,
    DvrStoreNamespace, DvrTrackId, EmbeddingMetric, FakeFrameDecisionLedgerProvider,
    FakeSemanticHit, FakeSemanticVideoSearchProvider, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionRecord, FrameDescriptionProvenance,
    FrameRoiTransform, FrameVectorIndexAttachment, GetSemanticSearchCapabilitiesRequest,
    GetVideoTimelineRequest, JevEvaluationId, JevEvaluationProvenance, JevProducerIdentity,
    JevStructuredOutput, LabPredicate, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, ModelSpaceIdentity, PipetteChannelSubject, ProvenanceGeneration, SavedFrameRef,
    SemanticFrameHit, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticIndexingCoverage, SemanticIndexingFreshness,
    SemanticIndexingStateCounts, SemanticProvenanceSelector, SemanticRetrievalScore,
    SemanticSearchScope, SemanticVideoSearchProvider, Sha256Digest, TextEmbeddingProvenance,
    TimeRange, UtcTimestamp, VideoProvider, VideoSource, VideoSourceId, bind_local,
    fake_semantic_hit,
};
use a2a_lab_dev_kit::{FrameDecisionLedgerProvider, FrameEmbeddingVector, VectorNormalization};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const LONG_SEMANTIC_DESCRIPTION: &str = "Supercalifragilisticexpialidocious-pipette-alignment-artifact-observed-on-channel-one-with-continuous-unbroken-token-stream-and-retrieval-metadata-that-must-wrap-inside-the-results-pane-without-horizontal-overflow";
const SEMANTIC_OVERFLOW_CHECK: &str = r"(() => {
  const scroll = document.querySelector('#video-search-results .log-scroll');
  const detail = document.getElementById('video-semantic-detail');
  const hits = [...document.querySelectorAll('#video-semantic-hits .video-semantic-hit')];
  const controlPx = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--density-control-height')) * (parseFloat(getComputedStyle(document.documentElement).fontSize) || 16);
  const inside = (panel, el) => {
    if (!panel || !el) return true;
    const port = panel.getBoundingClientRect();
    const box = el.getBoundingClientRect();
    if (box.width < 1) return true;
    return box.left >= port.left - 1 && box.right <= port.right + 1 && box.right <= window.innerWidth + 1;
  };
  const overlaps = (a, b) => {
    if (!a || !b || a.height < 2 || b.height < 2) return false;
    const x = Math.min(a.right, b.right) - Math.max(a.left, b.left);
    const y = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    return x > 2 && y > 2;
  };
  const docOverflow = document.documentElement.scrollWidth <= window.innerWidth + 1;
  const scrollLeft = scroll ? scroll.scrollLeft : 0;
  const scrollContained = scroll && scrollLeft < 2;
  const detailContained = !detail || [...detail.querySelectorAll('dd, dt, p, time, span')].every((el) => inside(scroll, el));
  const hitsContained = hits.every((hit) => inside(scroll, hit));
  const scrollY = scroll ? getComputedStyle(scroll).overflowY : '';
  const stacked = getComputedStyle(document.getElementById('video-search-browser')).flexDirection === 'column';
  const hitLayout = hits.map((hit) => {
    const style = getComputedStyle(hit);
    const box = hit.getBoundingClientRect();
    const desc = hit.querySelector('.video-semantic-description');
    const descBox = desc?.getBoundingClientRect();
    return {
      whiteSpace: style.whiteSpace,
      boxHeight: box.height,
      fits: hit.scrollHeight <= hit.clientHeight + 1,
      descInside: !descBox || (descBox.bottom <= box.bottom + 1 && descBox.top >= box.top - 1),
      tallEnough: !desc || desc.scrollHeight <= 24 || box.height > controlPx + 4,
    };
  });
  const hitsStacked = hits.length < 2 || hits.every((hit, index) => {
    if (index === 0) return true;
    return hit.getBoundingClientRect().top >= hits[index - 1].getBoundingClientRect().bottom - 1;
  });
  const detailBox = detail?.getBoundingClientRect() || null;
  const detailSeparated = !detailBox || hits.every((hit) => !overlaps(hit.getBoundingClientRect(), detailBox));
  const layoutPass = hitLayout.every((m) => m.fits && m.descInside && m.tallEnough) && hitsStacked && detailSeparated;
  return {
    pass: docOverflow && scrollContained && detailContained && hitsContained && layoutPass,
    docOverflow,
    scrollContained,
    detailContained,
    hitsContained,
    layoutPass,
    hitsStacked,
    detailSeparated,
    hitLayout,
    scrollY,
    stacked,
    innerWidth: window.innerWidth,
    scrollWidth: document.documentElement.scrollWidth
  };
})()";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

#[tokio::test]
async fn browser_videos_semantic_absent_keeps_legacy_browsing() {
    let base = legacy_lab().await;
    tokio::task::spawn_blocking(move || legacy_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_semantic_hits_navigate_and_separate_metadata() {
    let base = semantic_lab().await;
    tokio::task::spawn_blocking(move || semantic_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_semantic_long_descriptions_stay_contained() {
    let base = semantic_lab_long_descriptions().await;
    tokio::task::spawn_blocking(move || semantic_overflow_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_semantic_search_result_states() {
    let base = semantic_lab().await;
    tokio::task::spawn_blocking(move || semantic_result_states_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_semantic_hits_open_browse_deep_links() {
    let base = linked_hit_lab().await;
    tokio::task::spawn_blocking(move || linked_hit_flow(&base))
        .await
        .unwrap();
}

const SEMANTIC_SEARCH_LOADING_HELD: &str = r"document.getElementById('video-search-result-state')?.dataset.state === 'loading'
  && document.getElementById('video-semantic-search-loading')
  && document.getElementById('video-semantic-status')?.dataset.loading === 'true'
  && typeof window.__releaseSemanticSearch === 'function'";

const HOLD_SEMANTIC_SEARCH: &str = r"( () => {
  const orig = window.fetch.bind(window);
  let pending = null;
  let holdNext = false;
  let failRemaining = 0;
  window.__holdNextSemanticSearch = () => { holdNext = true; };
  window.__failNextSemanticSearch = (count) => {
    failRemaining = Number.isFinite(count) ? Math.max(1, count) : 2;
  };
  window.__releaseSemanticSearch = () => {
    if (pending) {
      const resolve = pending;
      pending = null;
      resolve();
    }
  };
  window.fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    let body = '';
    if (String(url).includes('/message:send')) {
      try {
        const copy = input instanceof Request ? input.clone() : new Request(input, init);
        body = await copy.text();
      } catch (_) {}
    }
    const search = body.includes('search_semantic_frames');
    if (search && failRemaining > 0) {
      failRemaining -= 1;
      return new Response('{}', { status: 500, statusText: 'Server Error' });
    }
    if (search && holdNext) {
      holdNext = false;
      await new Promise((resolve) => { pending = resolve; });
    }
    return orig(input, init);
  };
})();";

async fn legacy_lab() -> String {
    let videos = filled_videos().await;
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .share(),
    )
    .await
}

async fn semantic_lab_long_descriptions() -> String {
    let videos = filled_videos().await;
    let semantic = long_description_semantic_provider();
    let saved = frame_payload();
    let decisions = decision_provider(&saved).unwrap();
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .with_semantic_search(semantic)
            .with_frame_decisions(decisions)
            .share(),
    )
    .await
}

async fn semantic_lab() -> String {
    let videos = filled_videos().await;
    insert_source(&videos, "cam-b", "Camera B").await;
    for at in [
        "2024-01-01T00:00:03Z",
        "2024-01-01T00:00:04Z",
        "2024-01-01T00:00:05Z",
        "2024-01-01T00:00:06Z",
        "2024-01-01T00:00:07Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-b", at).await;
    }
    let semantic = paged_semantic_provider();
    let decisions = decision_provider(&frame_payload()).unwrap();
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .with_semantic_search(semantic)
            .with_frame_decisions(decisions)
            .share(),
    )
    .await
}

fn legacy_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until("document.getElementById('video-interval-0')");
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'absent' || document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    let state = page
        .eval("(() => document.getElementById('video-semantic-capability')?.dataset.state ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(state, "absent", "legacy browsing should report absent APIs");
    assert_eq!(
        page.eval("document.getElementById('video-browse-tab')?.getAttribute('aria-selected')"),
        serde_json::json!("false"),
    );
    page.click("video-browse-tab");
    page.wait_until("!document.getElementById('video-browse-panel')?.hidden");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-player')",
    );
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
fn semantic_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until(
        "document.querySelector('#video-search-panel:not([hidden]) #video-search-browser[data-source-id=\"cam-a\"]')",
    );
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-context')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-browser')?.dataset.sourceId"),
        serde_json::json!("cam-a"),
    );
    assert_eq!(
        page.eval(
            "document.getElementById('video-search-source-all')?.getAttribute('aria-pressed')"
        ),
        serde_json::json!("true"),
    );
    page.wait_until("document.getElementById('video-search-range-start')?.value === ''");
    assert_eq!(
        page.eval("document.getElementById('video-search-range-start')?.type"),
        serde_json::json!("datetime-local"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-range-end')?.type"),
        serde_json::json!("datetime-local"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-range-start')?.step"),
        serde_json::json!("1"),
    );
    let player_source = page.eval("document.getElementById('video-source')?.value");
    page.click("video-search-source-0");
    page.click("video-search-source-1");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.searchSources === 'cam-a,cam-b'",
    );
    assert_eq!(
        page.eval(
            "document.getElementById('video-search-source-all')?.getAttribute('aria-pressed')"
        ),
        serde_json::json!("false"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-source')?.value"),
        player_source,
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
    );
    page.click("video-search-source-all");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.searchSources === 'all'",
    );
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    page.click("video-search-clear");
    page.wait_until("document.getElementById('video-search-browser')?.dataset.start === ''");
    page.click("video-search-local-time");
    page.wait_until(
        "document.getElementById('video-search-local-time')?.getAttribute('aria-pressed') === 'true'",
    );
    page.click("video-search-local-time");
    page.wait_until(
        "document.getElementById('video-search-local-time')?.getAttribute('aria-pressed') === 'false' && document.getElementById('video-search-range-start')?.value === ''",
    );
    let generation_before = page.eval("document.getElementById('video-view')?.dataset.gen ?? ''");
    assert_eq!(
        page.eval(
            r"(() => {
                const start = document.getElementById('video-search-range-start');
                const end = document.getElementById('video-search-range-end');
                start.value = '2024-01-01T00:00:01';
                end.value = '2024-01-01T00:00:05';
                start.dispatchEvent(new Event('input', { bubbles: true }));
                end.dispatchEvent(new Event('input', { bubbles: true }));
                return start.value === '2024-01-01T00:00:01' && end.value === '2024-01-01T00:00:05';
            })()",
        ),
        serde_json::json!(true),
    );
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.start?.startsWith('2024-01-01T00:00:01')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-view')?.dataset.gen ?? ''"),
        generation_before.clone(),
    );
    page.click("video-search-clear");
    page.wait_until("document.getElementById('video-search-browser')?.dataset.start === ''");
    assert_eq!(
        page.eval("document.getElementById('video-search-range-start')?.value"),
        serde_json::json!(""),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-range-end')?.value"),
        serde_json::json!(""),
    );
    page.eval(
        r"(() => {
            const start = document.getElementById('video-search-range-start');
            const end = document.getElementById('video-search-range-end');
            start.value = '2024-01-01T00:00:05';
            end.value = '2024-01-01T00:00:01';
            start.dispatchEvent(new Event('input', { bubbles: true }));
            end.dispatchEvent(new Event('input', { bubbles: true }));
            return end.value === '2024-01-01T00:00:01';
        })()",
    );
    page.wait_until(
        "document.getElementById('video-search-range-error')?.textContent.trim().length > 0",
    );
    page.click("video-search-clear");
    page.wait_until(
        "document.getElementById('video-search-range-error')?.textContent.trim() === ''",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-query')?.value"),
        serde_json::json!(""),
    );
    assert_eq!(
        page.eval("document.getElementById('video-view')?.dataset.gen ?? ''"),
        generation_before,
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-label')?.textContent.trim()"),
        serde_json::json!("Search"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-filters')?.getAttribute('aria-label')"),
        serde_json::json!("Search"),
    );
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until("document.querySelector('#video-semantic-hits .video-semantic-hit')");
    page.wait_until("document.querySelector('#video-semantic-hits [data-source=\"cam-b\"]')");
    page.wait_until("document.getElementById('video-semantic-pagination')");
    page.click("video-semantic-next");
    page.wait_until(
        "document.getElementById('video-semantic-page-status')?.textContent.includes('Page 2')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-0')"),
        serde_json::json!(null),
    );
    page.click("video-semantic-previous");
    page.wait_until("document.getElementById('video-semantic-hit-0')");
    page.wait_until("document.getElementById('video-semantic-detail')");
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-0')?.tagName"),
        serde_json::json!("A"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-0')?.getAttribute('href')"),
        serde_json::json!("/ui/videos?source_id=cam-a&at=2024-01-01T00%3A00%3A01Z&autoplay=false"),
    );
    assert_eq!(
        page.eval(
            r#"document.querySelector('#video-semantic-hits [data-source="cam-b"]')?.getAttribute('href')"#,
        ),
        serde_json::json!(
            "/ui/videos?source_id=cam-b&at=2024-01-01T00%3A00%3A01Z&autoplay=false"
        ),
    );
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time')?.tagName"),
        serde_json::json!("TIME"),
    );
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time a')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time')?.closest('a')?.id"),
        serde_json::json!("video-semantic-hit-0"),
    );
    page.wait_until("document.querySelector('[data-chip=\"search\"]')");
    page.eval(
        r"(() => {
            const start = document.getElementById('video-search-range-start');
            const end = document.getElementById('video-search-range-end');
            start.value = '2024-01-01T00:00:00';
            end.value = '2024-01-01T00:00:10';
            start.dispatchEvent(new Event('input', { bubbles: true }));
            end.dispatchEvent(new Event('input', { bubbles: true }));
            return start.value === '2024-01-01T00:00:00' && end.value === '2024-01-01T00:00:10';
        })()",
    );
    let html = page
        .eval("(() => document.getElementById('video-semantic-detail')?.innerHTML ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(html.contains("Search metadata (Luna)"));
    assert!(html.contains("Frame decisions (Jev)"));
    assert!(html.contains("not a verified lab decision"));
    assert!(!html.contains("chupacabra.dvr"));
    assert!(!html.contains("Bearer "));
    let index_state = page
        .eval(
            "(() => document.getElementById('video-semantic-index-state')?.dataset.state ?? '')()",
        )
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(index_state, "ready");
    page.wait_until(
        "document.querySelector('#video-decision-row-0')?.dataset.outcome === 'present'",
    );
    assert_eq!(
        page.eval("document.querySelector('#video-decision-row-0 a')"),
        serde_json::json!(null),
    );
    let decision_outcome = page
        .eval("(() => document.querySelector('#video-decision-row-0')?.dataset.outcome ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(decision_outcome, "present");
}

fn semantic_overflow_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-range-start')?.type"),
        serde_json::json!("datetime-local"),
    );
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until("document.getElementById('video-semantic-detail')");
    let wide = page.eval(SEMANTIC_OVERFLOW_CHECK);
    assert_eq!(
        wide.get("pass"),
        Some(&serde_json::json!(true)),
        "wide viewport overflow report: {wide}"
    );
    assert!(
        wide.get("scrollY")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "auto" || v == "scroll"),
        "results pane keeps vertical scroll: {wide}"
    );
    page.viewport(767, 900);
    page.wait_until(
        "getComputedStyle(document.getElementById('video-search-browser')).flexDirection === 'column'",
    );
    let narrow = page.eval(SEMANTIC_OVERFLOW_CHECK);
    assert_eq!(
        narrow.get("stacked"),
        Some(&serde_json::json!(true)),
        "narrow search browser stacks filters: {narrow}"
    );
    assert_eq!(
        narrow.get("pass"),
        Some(&serde_json::json!(true)),
        "narrow viewport overflow report: {narrow}"
    );
    let selected = page.eval(SEMANTIC_OVERFLOW_CHECK);
    assert_eq!(
        selected.get("pass"),
        Some(&serde_json::json!(true)),
        "narrow viewport with selected detail: {selected}"
    );
}

fn semantic_result_states_flow(base: &str) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(HOLD_SEMANTIC_SEARCH);
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until("document.querySelector('#video-semantic-hits .video-semantic-hit')");
    page.wait_until("document.getElementById('video-semantic-pagination')");
    page.wait_until("document.getElementById('video-semantic-detail')");

    page.eval("window.__holdNextSemanticSearch()");
    page.set_value("video-semantic-query", "pipette tip");
    page.click_selector("#video-semantic-submit");
    page.wait_until(SEMANTIC_SEARCH_LOADING_HELD);
    assert_eq!(
        page.eval("document.getElementById('video-search-result-state')?.dataset.state"),
        serde_json::json!("loading"),
    );
    assert_eq!(
        page.eval(
            "document.querySelectorAll('#video-semantic-search-loading .strand-loader').length"
        ),
        serde_json::json!(1),
    );
    assert_eq!(
        page.eval(
            "document.querySelectorAll('#video-search-result-state .loading-surface').length"
        ),
        serde_json::json!(1),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hits')?.hidden"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-detail')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-pagination')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-count')?.textContent.trim()"),
        serde_json::json!("Searching…"),
    );
    page.viewport(767, 900);
    page.wait_until(
        "getComputedStyle(document.getElementById('video-search-browser')).flexDirection === 'column'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-result-state')?.dataset.state"),
        serde_json::json!("loading"),
    );
    page.eval("window.__releaseSemanticSearch()");
    page.wait_until(
        "document.getElementById('video-search-result-state')?.dataset.state === 'ready'",
    );
    page.wait_until("document.querySelector('#video-semantic-hits .video-semantic-hit')");
    page.wait_until("document.getElementById('video-semantic-detail')");

    page.set_value("video-semantic-query", "no-such-semantic-term");
    page.click_selector("#video-semantic-submit");
    page.wait_until(
        "document.getElementById('video-search-result-state')?.dataset.state === 'empty'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hits')?.hidden"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-status')?.textContent.trim()"),
        serde_json::json!("No hits in this scope."),
    );

    page.eval("window.__failNextSemanticSearch()");
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until(
        "document.getElementById('video-search-result-state')?.dataset.state === 'fault'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-status')?.dataset.state"),
        serde_json::json!("fault"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hits')?.hidden"),
        serde_json::json!(true),
    );
}

const BROWSE_CAM_A: &str = "!document.getElementById('video-browse-panel')?.hidden && document.getElementById('video-source')?.value === 'cam-a' && decodeURIComponent(location.search).includes('source_id=cam-a') && decodeURIComponent(location.search).includes('autoplay=false')";
const BROWSE_CAM_B: &str = "!document.getElementById('video-browse-panel')?.hidden && document.getElementById('video-source')?.value === 'cam-b' && decodeURIComponent(location.search).includes('source_id=cam-b') && decodeURIComponent(location.search).includes('autoplay=false')";
const SEARCH_HITS: &str = "document.querySelector('#video-search-panel:not([hidden]) #video-semantic-hits .video-semantic-hit')";

fn linked_hit_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until(SEARCH_HITS);
    page.wait_until("document.getElementById('video-semantic-detail')");
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-0')?.getAttribute('href')"),
        serde_json::json!("/ui/videos?source_id=cam-a&at=2024-01-01T00%3A00%3A01Z&autoplay=false"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-1')?.getAttribute('href')"),
        serde_json::json!("/ui/videos?source_id=cam-b&at=2024-01-01T00%3A00%3A01Z&autoplay=false"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-2')?.tagName"),
        serde_json::json!("BUTTON"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-2')?.getAttribute('href')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-2')?.disabled"),
        serde_json::json!(true),
    );
    assert_hit_timestamps_share_row_href(&mut page);
    let html = page
        .eval("(() => document.getElementById('video-semantic-detail')?.innerHTML ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(html.contains("Search metadata (Luna)"));
    assert!(html.contains("Frame decisions (Jev)"));

    page.click("video-semantic-hit-2");
    assert_eq!(
        page.eval("location.pathname.endsWith('/videos/search')"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-source')?.value"),
        serde_json::json!("cam-a"),
    );

    page.click("video-semantic-hit-0");
    page.wait_until(BROWSE_CAM_A);
    let same_source_search = page
        .eval("decodeURIComponent(location.search)")
        .as_str()
        .unwrap_or("")
        .to_owned();

    page.click("video-search-tab");
    page.wait_until(SEARCH_HITS);
    page.click("video-semantic-hit-1");
    page.wait_until(BROWSE_CAM_B);

    let mut direct = Browser::open(&format!(
        "{base}/ui/videos?source_id=cam-a&at=2024-01-01T00%3A00%3A01Z&autoplay=false"
    ));
    direct.wait_until(CONNECTED);
    direct.wait_until(BROWSE_CAM_A);
    assert_eq!(
        direct.eval("decodeURIComponent(location.search)"),
        serde_json::json!(same_source_search),
    );
}

fn assert_hit_timestamps_share_row_href(page: &mut Browser) {
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time')?.tagName"),
        serde_json::json!("TIME"),
    );
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time a')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.querySelector('#video-semantic-hit-0 time')?.closest('a')?.id"),
        serde_json::json!("video-semantic-hit-0"),
    );
    page.wait_until("document.querySelector('#video-decision-row-0')");
    assert_eq!(
        page.eval("document.querySelector('#video-decision-row-0 a')"),
        serde_json::json!(null),
    );
}

async fn linked_hit_lab() -> String {
    let videos = filled_videos().await;
    insert_source(&videos, "cam-b", "Camera B").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-b", at).await;
    }
    let semantic = linked_semantic_provider();
    let decisions = decision_provider(&frame_payload()).unwrap();
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .with_semantic_search(semantic)
            .with_frame_decisions(decisions)
            .share(),
    )
    .await
}

fn linked_semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let cam_a = frame_at("2024-01-01T00:00:01Z");
    provider.insert_hit(
        fake_semantic_hit(
            cam_a.clone(),
            attachment.clone(),
            "A pipette tip is visible on channel one.",
            0.92,
            neighbors(&cam_a),
        )
        .unwrap(),
    );
    let cam_b = frame_for("cam-b", "2024-01-01T00:00:01Z");
    provider.insert_hit(
        fake_semantic_hit(
            cam_b.clone(),
            attachment.clone(),
            "A pipette tip is visible on the external camera.",
            0.91,
            neighbors(&cam_b),
        )
        .unwrap(),
    );
    let blocked = frame_at("2024-01-01T00:00:02Z");
    provider.insert_hit(blocked_semantic_hit(
        blocked,
        attachment,
        "A pipette tip is expired from retained playback.",
    ));
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 2, 0, 1),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    provider.set_active_provenance(selector(1));
    Arc::new(provider)
}

fn blocked_semantic_hit(
    frame: SavedFrameRef,
    attachment: FrameVectorIndexAttachment,
    description: &str,
) -> FakeSemanticHit {
    let playback = neighbors(&frame);
    let retrieval = SemanticRetrievalScore::new(0.5, attachment.model_space().metric()).unwrap();
    let hit = SemanticFrameHit::new(
        frame,
        description,
        &attachment,
        retrieval,
        SemanticFrameIndexState::Expired,
        SemanticFrameMediaAvailability::Unavailable {
            reason: "retention_expired".to_owned(),
        },
        playback,
        None,
    )
    .unwrap();
    FakeSemanticHit {
        hit,
        attachment,
        description_text: description.to_owned(),
    }
}

async fn filled_videos() -> MemoryVideos {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    videos
}

async fn insert_source(videos: &MemoryVideos, id: &str, name: &str) {
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new(id).unwrap(),
                name,
                format!("{name} recording"),
                None,
                None,
                640,
                360,
                30,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

async fn insert_frame(videos: &MemoryVideos, source: &str, at: &str) {
    videos
        .insert_frame(
            VideoSourceId::new(source).unwrap(),
            UtcTimestamp::parse(at).unwrap(),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
}

async fn serve(service: Arc<dyn a2a_lab_dev_kit::A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&service)
            .with_web_ui()
            .listen(listener)
            .await
            .unwrap();
    });
    format!("http://{address}")
}

fn long_description_semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let saved = frame_payload();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let vector_attachment = attachment(1, LONG_SEMANTIC_DESCRIPTION.as_bytes());
    let playback = neighbors(&saved);
    let prepared = fake_semantic_hit(
        saved,
        vector_attachment,
        LONG_SEMANTIC_DESCRIPTION,
        0.92,
        playback,
    )
    .unwrap();
    provider.insert_hit(prepared);
    let short = fake_semantic_hit(
        frame_at("2024-01-01T00:00:02Z"),
        attachment(2, b"Short pipette note."),
        "Short pipette note.",
        0.88,
        neighbors(&frame_at("2024-01-01T00:00:02Z")),
    )
    .unwrap();
    provider.insert_hit(short);
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    provider.set_active_provenance(selector(1));
    Arc::new(provider)
}

fn semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let saved = frame_payload();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let playback = neighbors(&saved);
    let prepared = fake_semantic_hit(
        saved,
        attachment,
        "A pipette tip is visible on channel one.",
        0.92,
        playback,
    )
    .unwrap();
    provider.insert_hit(prepared);
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    provider.set_active_provenance(selector(1));
    Arc::new(provider)
}

fn paged_semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let saved = frame_payload();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let prepared = fake_semantic_hit(
        saved.clone(),
        attachment.clone(),
        "A pipette tip is visible on channel one.",
        0.92,
        neighbors(&saved),
    )
    .unwrap();
    provider.insert_hit(prepared);
    let other = frame_for("cam-b", "2024-01-01T00:00:01Z");
    let other_hit = fake_semantic_hit(
        other.clone(),
        attachment.clone(),
        "A pipette tip is visible on the external camera.",
        0.91,
        neighbors(&other),
    )
    .unwrap();
    provider.insert_hit(other_hit);
    for (at, score) in [
        ("2024-01-01T00:00:00Z", 0.8_f32),
        ("2024-01-01T00:00:02Z", 0.75_f32),
        ("2024-01-01T00:00:03Z", 0.7_f32),
        ("2024-01-01T00:00:04Z", 0.65_f32),
        ("2024-01-01T00:00:05Z", 0.6_f32),
        ("2024-01-01T00:00:06Z", 0.55_f32),
    ] {
        let frame = frame_at(at);
        let extra = fake_semantic_hit(
            frame.clone(),
            attachment.clone(),
            "A pipette tip is visible on channel one.",
            score,
            neighbors(&frame),
        )
        .unwrap();
        provider.insert_hit(extra);
    }
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    provider.set_active_provenance(selector(1));
    Arc::new(provider)
}

fn decision_provider(
    saved: &SavedFrameRef,
) -> Result<Arc<dyn FrameDecisionLedgerProvider>, a2a_lab_dev_kit::A2aLabError> {
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"eval-config"),
        Sha256Digest::hash_bytes(b"threshold"),
    )?;
    let provider = Arc::new(FakeFrameDecisionLedgerProvider::read_only_consumer(
        &config,
        FrameDecisionEvaluatorLimits::pilot(),
    )?);
    provider.seed(tip_record(saved, DecisionOutcome::Present))?;
    Ok(provider)
}

fn frame_payload() -> SavedFrameRef {
    frame_at("2024-01-01T00:00:01Z")
}

fn frame_at(at: &str) -> SavedFrameRef {
    frame_for("cam-a", at)
}

fn frame_for(source: &str, at: &str) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        VideoSourceId::new(source).unwrap(),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse(at).unwrap(),
        JPEG,
        None,
    )
    .unwrap()
}

fn scope() -> SemanticSearchScope {
    SemanticSearchScope::new(
        VideoSourceId::new("cam-a").unwrap(),
        Some(
            TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-01-01T00:00:10Z").unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap()
}

fn selector(generation: u64) -> SemanticProvenanceSelector {
    SemanticProvenanceSelector::new(
        model_space(generation),
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap()
}

fn model_space(generation: u64) -> ModelSpaceIdentity {
    ModelSpaceIdentity::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap()
}

fn attachment(generation: u64, text: &[u8]) -> FrameVectorIndexAttachment {
    let desc = FrameDescriptionProvenance::new(
        "bedrock",
        "global.openai.gpt-5.6-luna",
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"preprocess-v1"),
        Some(FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap()),
        Sha256Digest::hash_bytes(text),
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap();
    let embed = TextEmbeddingProvenance::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
        Sha256Digest::hash_bytes(text),
    )
    .unwrap();
    let vector = FrameEmbeddingVector::new(vec![0.1, 0.2, 0.3, 0.4], 4).unwrap();
    FrameVectorIndexAttachment::new(model_space(generation), desc, embed, vector).unwrap()
}

fn neighbors(saved: &SavedFrameRef) -> SemanticFramePlaybackNeighbors {
    let start = saved.captured_at();
    let end = start.saturating_add_micros(5_000_000);
    SemanticFramePlaybackNeighbors::new(TimeRange::new(start, end).unwrap(), vec![saved.clone()], 5)
        .unwrap()
}

fn tip_record(saved: &SavedFrameRef, outcome: DecisionOutcome) -> FrameDecisionRecord {
    let key = DecisionIdempotencyKey::new(
        "jev-ui-fixture-001:lab.pipette_tip_attached:left-pipetter-channel-1",
    )
    .unwrap();
    let subject = DecisionSubject::PipetteChannel(
        PipetteChannelSubject::new(
            "left-pipetter",
            "channel-1",
            Some(FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap()),
        )
        .unwrap(),
    );
    let evidence = vec![
        DecisionEvidenceItem::saved_frame_revision(
            saved.frame_id().clone(),
            FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap(),
        )
        .unwrap(),
    ];
    let provenance = JevEvaluationProvenance::new(
        saved.captured_at(),
        UtcTimestamp::parse("2024-01-01T00:00:05Z").unwrap(),
        JevProducerIdentity::default_production(),
        JevEvaluationId::new("jev-ui-fixture-001").unwrap(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response"),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .unwrap();
    FrameDecisionRecord::new(
        key,
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        subject,
        evidence,
        outcome,
        JevStructuredOutput::new(Some(0.97), None, None, None, Some(0.88), None, None).unwrap(),
        provenance,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn http_semantic_capabilities_body_includes_active_provenance() {
    let base = semantic_lab().await;
    let response = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", a2a_lab_dev_kit::A2A_PROTOCOL_VERSION)
        .json(&serde_json::json!({
            "message": {
                "messageId": "semantic-cap-http",
                "role": "ROLE_USER",
                "parts": [{
                    "data": {
                        "operation": "get_semantic_search_capabilities",
                        "params": {}
                    }
                }]
            }
        }))
        .send()
        .await
        .expect("send");
    assert!(response.status().is_success());
    let body = response.text().await.expect("body");
    assert!(
        body.contains("active_provenance"),
        "missing active provenance in HTTP response: {body}"
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("json");
    let data = value
        .pointer("/task/artifacts/0/parts/0/data")
        .or_else(|| value.pointer("/artifacts/0/parts/0/data"))
        .cloned()
        .expect("artifact data");
    assert_eq!(
        data.get("operation").and_then(|v| v.as_str()),
        Some("get_semantic_search_capabilities")
    );
    assert!(
        data.pointer("/result/active_provenance/model_space")
            .is_some()
    );
    let artifacts = value
        .pointer("/task/artifacts")
        .and_then(|v| v.as_array())
        .expect("artifacts");
    assert_eq!(artifacts.len(), 1, "unexpected artifact count: {body}");
    let parts = artifacts[0]
        .get("parts")
        .and_then(|v| v.as_array())
        .expect("parts");
    let data_parts = parts
        .iter()
        .filter(|part| part.get("data").is_some())
        .count();
    assert_eq!(data_parts, 1, "expected one data part: {body}");
    let payload = serde_json::to_string_pretty(&data).expect("pretty");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("payload parse");
    assert_eq!(
        parsed.get("operation").and_then(|v| v.as_str()),
        Some("get_semantic_search_capabilities")
    );
    let desc_gen = parsed
        .pointer("/result/active_provenance/description_generation")
        .expect("description_generation");
    assert!(
        desc_gen.is_number(),
        "description_generation must be numeric on the wire: {desc_gen:?}"
    );
    assert!(
        body.contains("\"description_generation\":1.0")
            || body.contains("\"description_generation\":1"),
        "A2A wire must emit description_generation as a whole number: {}",
        &body[..body.len().min(2000)]
    );
}

#[tokio::test]
async fn semantic_capabilities_include_active_provenance_on_wire() {
    let videos = filled_videos().await;
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .with_semantic_search(paged_semantic_provider())
        .share();
    let outcome = lab
        .execute(A2aLabCommand::GetSemanticSearchCapabilities(
            GetSemanticSearchCapabilitiesRequest::new(),
        ))
        .await
        .expect("execute");
    let A2aLabResult::GetSemanticSearchCapabilities(caps) = outcome.task.result else {
        panic!("capabilities");
    };
    let active = caps.active_provenance().expect("active provenance");
    assert_eq!(active.model_space().model(), "text-embed-v1");
    assert_eq!(active.description_generation().get(), 1);
}

#[tokio::test]
async fn ui_shaped_search_command_returns_fixture_hit() {
    let videos = filled_videos().await;
    let timeline = videos
        .get_video_timeline(GetVideoTimelineRequest::new(
            VideoSourceId::new("cam-a").unwrap(),
        ))
        .await
        .unwrap();
    let interval = timeline.intervals().first().expect("retained interval");
    let last = timeline.intervals().last().expect("last retained interval");
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .with_semantic_search(semantic_provider())
        .share();
    let command: A2aLabCommand = serde_json::from_value(serde_json::json!({
        "operation": "search_semantic_frames",
        "params": {
            "scope": {
                "source_id": "cam-a",
                "range": {
                    "start": interval.start().to_rfc3339(),
                    "end": last.end().to_rfc3339()
                }
            },
            "query": { "kind": "text", "text": "pipette" },
            "provenance": {
                "model_space": {
                    "provider": "bedrock",
                    "model": "text-embed-v1",
                    "dimensions": 4,
                    "normalization": "l2_unit",
                    "metric": "cosine",
                    "generation": 1
                },
                "description_generation": 1
            },
            "limits": {
                "top_k": 10,
                "candidate_limit": 1000,
                "max_neighbor_frames": 5
            },
            "page": { "limit": 10 }
        }
    }))
    .expect("ui-shaped command");
    let outcome = lab.execute(command).await.expect("execute");
    let A2aLabResult::SearchSemanticFrames(result) = outcome.task.result else {
        panic!("search result");
    };
    assert_eq!(result.hits().items().len(), 1);
}
