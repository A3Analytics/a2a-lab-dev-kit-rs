//! Videos reader: source rail, UTC clip playback, and authenticated MJPEG.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics,
    MemoryTasks, MemoryVideos, SecurityScheme, UtcTimestamp, VideoSource, VideoSourceId,
    bind_local,
};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{Browser, header, request_url};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const TOKEN: &str = "video-bearer-token";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

#[tokio::test]
async fn browser_videos_selects_plays_and_recovers() {
    let (filled, videos) = filled_lab().await;
    let empty = empty_lab().await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || flow(&filled, &empty, &videos, &handle))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_timeline_follows_live_recorded_and_gaps() {
    let base = timeline_lab().await;
    tokio::task::spawn_blocking(move || timeline_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_sends_bearer_only_in_headers() {
    let base = secured_lab().await;
    tokio::task::spawn_blocking(move || bearer_flow(&base))
        .await
        .unwrap();
}

async fn filled_lab() -> (String, MemoryVideos) {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    insert_source(&videos, "cam-b", "Camera B").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    insert_frame(&videos, "cam-b", "2024-01-01T00:00:00Z").await;
    (serve(videos.clone(), false).await, videos)
}

async fn empty_lab() -> String {
    serve(MemoryVideos::new(), false).await
}

async fn timeline_lab() -> String {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    for second in 0..=60 {
        insert_frame(&videos, "cam-a", &stamp(second)).await;
    }
    insert_source(&videos, "cam-b", "Camera B").await;
    insert_frame(&videos, "cam-b", "2024-01-01T00:00:00Z").await;
    insert_frame(&videos, "cam-b", "2024-01-01T00:00:10Z").await;
    insert_source(&videos, "cam-c", "Camera C").await;
    videos
        .set_live_available(&VideoSourceId::new("cam-a").unwrap(), true)
        .await
        .unwrap();
    serve(videos, false).await
}

fn stamp(second: u64) -> String {
    let minutes = second / 60;
    let secs = second % 60;
    format!("2024-01-01T00:{minutes:02}:{secs:02}Z")
}

async fn secured_lab() -> String {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    insert_frame(&videos, "cam-a", "2024-01-01T00:00:00Z").await;
    insert_frame(&videos, "cam-a", "2024-01-01T00:00:01Z").await;
    insert_frame(&videos, "cam-a", "2024-01-01T00:00:02Z").await;
    serve(videos, true).await
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
                1,
                1,
                1,
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

async fn serve(videos: MemoryVideos, bearer: bool) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service).with_web_ui();
    if bearer {
        let schemes = [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into();
        let requirements = vec![[("bearerAuth".to_owned(), Vec::new())].into()];
        server = server.with_security(schemes, requirements);
    }
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn flow(filled: &str, empty: &str, videos: &MemoryVideos, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{filled}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    assert_shell(&mut page);
    assert_live_regions(&mut page);
    page.wait_until(
        "document.getElementById('video-interval-0') && document.getElementById('video-source') instanceof HTMLSelectElement && typeof window.__a2aNativePlay === 'function'",
    );
    assert_auto_playback_started(&mut page);
    assert_typed_range(&mut page);
    play_coverage(&mut page);
    page.wait_until("document.getElementById('video-status')?.dataset.state === 'playing'");
    assert_playback_started(&mut page);
    pause_player(&mut page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'paused' || document.getElementById('video-status')?.dataset.state === 'stopped'",
    );
    play_player(&mut page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped'",
    );
    pause_player(&mut page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'paused' || document.getElementById('video-status')?.dataset.state === 'stopped'",
    );
    assert_route_aborts(&mut page);
    assert_bounds(&mut page);
    assert_keyboard(&mut page);
    assert_still(&mut page);
    assert_reduced_motion(&mut page);
    assert_themes(&mut page);
    assert_failure(&mut page, videos, handle);
    assert_empty(empty);
}

fn bearer_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.wait_until("document.getElementById('bearer-token')");
    page.set_value("bearer-token", TOKEN);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-interval-0') && document.getElementById('video-source') instanceof HTMLSelectElement && typeof window.__a2aNativePlay === 'function'",
    );
    play_coverage(&mut page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-alert')?.textContent",
    );
    let requests = page.requests(base);
    let clip = requests
        .iter()
        .find(|request| request_url(request).contains("/media/video-clips/"));
    let clip = clip.expect("clip download");
    let expected = format!("Bearer {TOKEN}");
    assert_eq!(header(clip, "authorization"), Some(expected.as_str()));
    assert!(!request_url(clip).contains(TOKEN));
    assert!(!request_url(clip).contains('?'));
    let send = requests.iter().any(|request| {
        request_url(request).contains("/message:send")
            && header(request, "authorization").is_some_and(|value| value.contains(TOKEN))
    });
    assert!(send, "clip handle was not requested with a bearer header");
    let view = page
        .eval("document.getElementById('view-videos')?.innerText ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(!view.contains(TOKEN), "{view}");
}

fn assert_shell(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('nav-videos')?.getAttribute('aria-current')"),
        serde_json::json!("page")
    );
    assert_eq!(
        page.eval("document.getElementById('inventory-videos')?.hasAttribute('hidden')"),
        serde_json::json!(true)
    );
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        serde_json::json!(true)
    );
    let width = page
        .eval("document.getElementById('video-filters').getBoundingClientRect().width")
        .as_f64()
        .unwrap_or(0.0);
    assert!((width - 240.0).abs() < 8.0, "{width}");
    assert_eq!(
        page.eval("document.querySelectorAll('#video-view [role=tab]').length"),
        serde_json::json!(0)
    );
    assert_eq!(
        page.eval("document.querySelectorAll('#video-player').length"),
        serde_json::json!(1)
    );
}

fn assert_live_regions(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('video-status')?.getAttribute('role')"),
        serde_json::json!("status")
    );
    assert_eq!(
        page.eval("document.getElementById('video-status')?.getAttribute('aria-live')"),
        serde_json::json!("polite")
    );
    assert_eq!(
        page.eval("document.getElementById('video-alert')?.getAttribute('role')"),
        serde_json::json!("alert")
    );
    assert_eq!(
        page.eval("document.getElementById('video-alert')?.getAttribute('aria-live')"),
        serde_json::json!("assertive")
    );
}

fn assert_playback_started(page: &mut Browser) {
    let state = text(page, "video-status");
    let canvas = page.eval("document.getElementById('video-canvas')?.width ?? 0");
    assert!(
        state.contains("Playing") || state.contains("Stopped") || canvas.as_u64().unwrap_or(0) == 1,
        "{state} {canvas}"
    );
}

fn assert_route_aborts(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'connecting' || document.getElementById('video-status')?.dataset.state === 'playing'",
    );
    let before = page
        .eval("document.getElementById('video-view')?.dataset.gen ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    page.show("#logs");
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{before}' && document.getElementById('video-status')?.dataset.state === 'stopped'"
    ));
    page.show("#videos");
    let after = page
        .eval("document.getElementById('video-view')?.dataset.gen ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_ne!(before, after);
}

fn assert_bounds(page: &mut Browser) {
    let max_us = page
        .eval("document.getElementById('video-timeline')?.dataset.maxUs ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    let duration = page
        .eval("Number(document.getElementById('video-timeline')?.dataset.durationUs || '0')")
        .as_f64()
        .unwrap_or(0.0);
    assert_eq!(max_us, "60000000");
    assert!(
        duration > 0.0 && duration <= 60_000_000.0,
        "selection {duration} exceeds 60 seconds"
    );
}

fn assert_keyboard(page: &mut Browser) {
    page.focus("video-timeline");
    let before = text_attr(page, "video-timeline", "data-end");
    key_down(page, "video-timeline", "ArrowLeft", true);
    page.wait_until(&format!(
        "document.getElementById('video-timeline')?.dataset.end !== '{}'",
        before.replace('\'', "")
    ));
    let after = text_attr(page, "video-timeline", "data-duration-us");
    assert!(after.parse::<i64>().unwrap_or(i64::MAX) <= 60_000_000);
    page.click_box("video-follow-newest");
    let mouse = page.computed("#video-follow-newest")["outlineStyle"].clone();
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(mouse, serde_json::json!("none"), "{mouse}");
    assert!(ring.contains("3px"), "{ring}");
}

fn assert_still(page: &mut Browser) {
    pause_player(page);
    page.wait_until("!document.getElementById('video-frame-at')?.disabled");
    assert_eq!(
        page.eval("document.getElementById('video-frame-at')?.textContent.trim()"),
        serde_json::json!("Frame at time")
    );
    let before = page
        .eval("document.getElementById('video-view')?.dataset.gen ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    page.click("video-frame-at");
    page.wait_until(&format!(
        "document.getElementById('video-alert')?.textContent.length > 0 || (document.getElementById('video-view')?.dataset.gen !== '{before}' && document.getElementById('video-view')?.getAttribute('aria-busy') === 'false' && document.getElementById('video-status')?.dataset.state === 'idle')"
    ));
    let view = page
        .eval("document.getElementById('view-videos')?.innerText ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(!view.contains("Saved images"), "{view}");
    assert_eq!(
        page.eval("document.getElementById('image-view')?.closest('[hidden]') !== null || document.getElementById('view-images')?.hasAttribute('hidden')"),
        serde_json::json!(true)
    );
}

fn assert_reduced_motion(page: &mut Browser) {
    page.reduced_motion(true);
    page.focus("video-timeline");
    let before = text_attr(page, "video-timeline", "data-duration-us");
    key_down(page, "video-timeline", "ArrowLeft", true);
    page.wait_until(&format!(
        "document.getElementById('video-timeline')?.dataset.durationUs !== '{before}'"
    ));
    play_coverage(page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-status')?.dataset.state === 'connecting'",
    );
    assert_ne!(text(page, "video-status"), "");
    pause_player(page);
    page.reduced_motion(false);
}

fn assert_themes(page: &mut Browser) {
    for scheme in ["light", "dark"] {
        page.color_scheme(scheme);
        page.show("#videos");
        let player = page.computed("#video-player");
        assert_eq!(player["hidden"], serde_json::json!(false), "{scheme}");
        let stage = page.computed("#video-stage");
        assert!(!stage["backgroundColor"].as_str().unwrap_or("").is_empty());
    }
}

fn assert_failure(page: &mut Browser, videos: &MemoryVideos, handle: &tokio::runtime::Handle) {
    handle.block_on(videos.set_unavailable("camera offline"));
    play_coverage(page);
    page.wait_until("document.getElementById('video-retry')");
    handle.block_on(videos.clear_unavailable());
    page.click("video-retry");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-status')?.dataset.state === 'connecting'",
    );
    pause_player(page);
}

fn assert_empty(empty: &str) {
    let mut page = Browser::open(&format!("{empty}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-source-status')?.textContent.includes('No video sources')",
    );
}

fn timeline_flow(base: &str) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(&timeline_prelude());
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-interval-0') && window.__timelineSplit === true",
    );
    assert_eq!(
        page.eval("document.querySelectorAll('#video-timeline .video-interval').length"),
        serde_json::json!(1)
    );
    assert_eq!(
        page.eval("document.querySelectorAll('#video-timeline .video-gap').length"),
        serde_json::json!(0)
    );
    assert_auto_playback_started(&mut page);
    assert_live_hidden_boundary(&mut page, base);
    assert_playback_abort(&mut page);
    page.show("#videos");
    page.wait_until("document.getElementById('video-interval-0')");
    page.click("video-follow-newest");
    page.wait_until("(window.__liveFetches || 0) >= 2");
    assert_genuine_gap(&mut page);
    page.focus("video-timeline");
    key_down(&mut page, "video-timeline", "ArrowRight", false);
    key_down(&mut page, "video-timeline", "ArrowRight", false);
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.start?.includes('2024-01-01T00:00:10')",
    );
    let duration = page
        .eval("Number(document.getElementById('video-timeline')?.dataset.durationUs || '0')")
        .as_f64()
        .unwrap_or(0.0);
    assert!(duration > 0.0 && duration <= 1_000_000.0, "{duration}");
    page.reduced_motion(true);
    let before = text_attr(&mut page, "video-timeline", "data-start");
    key_down(&mut page, "video-timeline", "ArrowLeft", false);
    page.wait_until(&format!(
        "document.getElementById('video-timeline')?.dataset.start !== '{before}'"
    ));
    assert_ne!(
        page.eval("document.getElementById('video-status')"),
        serde_json::json!(null)
    );
    page.reduced_motion(false);
    page.wait_until("!document.getElementById('video-frame-at')?.disabled");
    page.click("video-frame-at");
    page.wait_until("document.getElementById('video-view')?.getAttribute('aria-busy') === 'false'");
    let requests = page.requests(base);
    let bodies = requests
        .iter()
        .filter_map(|request| request["params"]["request"]["postData"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(bodies.contains("get_video_timeline"), "{bodies}");
    assert!(bodies.contains("follow_video"), "{bodies}");
    assert!(bodies.contains("get_video_clip"), "{bodies}");
    assert!(bodies.contains("get_video_frame_at"), "{bodies}");
    assert!(!bodies.contains("list_images"), "{bodies}");
    assert!(!bodies.contains("search_images"), "{bodies}");
    let live = requests
        .iter()
        .filter(|request| request_url(request).contains("/media/video-live/"))
        .count();
    let clips = requests
        .iter()
        .filter(|request| request_url(request).contains("/media/video-clips/"))
        .count();
    assert!(live >= 1, "live follow was not opened");
    assert!(clips >= 1, "bounded clip was not opened");
    assert_gap_does_not_span(&bodies);
    assert_empty_timeline(&mut page);
}

fn assert_live_hidden_boundary(page: &mut Browser, base: &str) {
    page.click("video-follow-newest");
    page.wait_until("(window.__liveFetches || 0) >= 1");
    let live_urls = media_paths(page, base, "/media/video-live/");
    expect_case(
        "live follow",
        live_urls
            .iter()
            .all(|url| !url.contains('?') && !url.contains('#')),
        "authenticated relative live path",
        &live_urls.join(" "),
    );
    play_coverage(page);
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.durationUs === '60000000' && (window.__clipFetches || 0) >= 1 && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped') && document.getElementById('video-alert')?.dataset.failure === '' && document.getElementById('video-player')?.hasAttribute('controls')",
    );
    let crosses = page.eval(
        "(() => { const el = document.getElementById('video-timeline'); const start = el?.dataset.start || ''; const end = el?.dataset.end || ''; return start < '2024-01-01T00:00:30Z' && end > '2024-01-01T00:00:30Z'; })()",
    );
    expect_case(
        "hidden chunk boundary",
        crosses == serde_json::json!(true),
        "60s selection crosses the abutting boundary and plays",
        &format!(
            "start {} end {} status {}",
            text_attr(page, "video-timeline", "data-start"),
            text_attr(page, "video-timeline", "data-end"),
            text_attr(page, "video-status", "data-state")
        ),
    );
    assert_eq!(
        page.eval("document.getElementById('video-follow-newest')?.textContent.trim()"),
        serde_json::json!("LIVE")
    );
}

fn assert_playback_abort(page: &mut Browser) {
    let playing = generation(page);
    key_down(page, "video-timeline", "ArrowRight", false);
    page.press_enter();
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{playing}'"
    ));
    expect_case(
        "abort on range change",
        generation(page) != playing,
        "committing a new range bumps generation",
        &format!("{playing} -> {}", generation(page)),
    );
    let ranged = generation(page);
    page.show("#logs");
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{ranged}' && document.getElementById('video-status')?.dataset.state === 'stopped'"
    ));
    expect_case(
        "abort on stop/route/unmount",
        generation(page) != ranged && text_attr(page, "video-status", "data-state") == "stopped",
        "leaving the viewer stops playback",
        &format!(
            "{ranged} -> {} {}",
            generation(page),
            text_attr(page, "video-status", "data-state")
        ),
    );
}

fn assert_genuine_gap(page: &mut Browser) {
    choose_video_source(page, "cam-b");
    page.wait_until(
        "document.getElementById('video-gap-0') && document.getElementById('video-interval-1')",
    );
    let intervals =
        page.eval("document.querySelectorAll('#video-timeline .video-interval').length");
    let gaps = page.eval("document.querySelectorAll('#video-timeline .video-gap').length");
    expect_case(
        "genuine gap",
        intervals.as_u64() >= Some(2) && gaps.as_u64() >= Some(1),
        "separate retained intervals with a gap",
        &format!("intervals {intervals} gaps {gaps}"),
    );
    page.click("video-interval-0");
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.start?.includes('2024-01-01T00:00:00') && !document.getElementById('video-timeline')?.dataset.end?.includes('2024-01-01T00:00:10') && Number(document.getElementById('video-timeline')?.dataset.durationUs || '0') <= 1000000 && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped') && document.getElementById('video-alert')?.dataset.failure === ''",
    );
}

fn assert_gap_does_not_span(bodies: &str) {
    let crossing = bodies.lines().any(|body| {
        body.contains("get_video_clip")
            && body.contains("\"source_id\":\"cam-b\"")
            && body.contains("2024-01-01T00:00:00")
            && body.contains("2024-01-01T00:00:10")
    });
    expect_case(
        "genuine gap playback",
        !crossing,
        "no clip request spans the cam-b gap",
        bodies,
    );
}

fn assert_empty_timeline(page: &mut Browser) {
    choose_video_source(page, "cam-c");
    page.wait_until(
        "document.getElementById('video-source')?.value === 'cam-c' && document.getElementById('video-timeline')?.dataset.ready === 'true' && document.getElementById('video-timeline')?.dataset.empty === 'true' && !(document.getElementById('video-alert-text')?.textContent.trim()) && !document.getElementById('video-retry')",
    );
    let empty_timeline = text(page, "video-timeline-empty");
    expect_case(
        "empty timeline",
        empty_timeline.contains("No retained video"),
        "empty coverage is success",
        &empty_timeline,
    );
}

fn choose_video_source(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let changed = page.eval(&format!(
        "(() => {{ const el = document.getElementById('video-source'); if (!(el instanceof HTMLSelectElement)) return false; el.value = {id}; el.dispatchEvent(new Event('change', {{bubbles:true}})); return el.value === {id}; }})()"
    ));
    assert_eq!(
        changed,
        serde_json::json!(true),
        "video source was not selected"
    );
}

fn expect_case(scenario: &str, pass: bool, expected: &str, observed: &str) {
    assert!(pass, "{scenario}: expected {expected}, observed {observed}");
}

fn generation(page: &mut Browser) -> String {
    page.eval("document.getElementById('video-view')?.dataset.gen ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn media_paths(page: &Browser, base: &str, needle: &str) -> Vec<String> {
    page.requests(base)
        .iter()
        .filter_map(|request| {
            let url = request_url(request);
            url.contains(needle).then(|| url.to_owned())
        })
        .collect()
}

fn timeline_prelude() -> String {
    r#"( () => {
  const orig = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    if (String(url).includes('/media/video-live/')) window.__liveFetches = (window.__liveFetches || 0) + 1;
    if (String(url).includes('/media/video-clips/')) window.__clipFetches = (window.__clipFetches || 0) + 1;
    const response = await orig(input, init);
    if (!String(url).includes('/message:send')) return response;
    const text = await response.clone().text();
    if (!text.includes('get_video_timeline') || !text.includes('"source_id":"cam-a"') || window.__timelineSplit) {
      return response;
    }
    const pattern = /"intervals":\[\{[^\]]+\}\]/;
    const found = text.match(pattern);
    const start = text.match(/"start":"(2024-01-01T00:00:00Z)"/);
    const end = text.match(/"end":"(2024-01-01T00:01:01Z)"/);
    if (!found || !start || !end) {
      window.__timelineUnsplit = text.slice(Math.max(0, text.indexOf('intervals') - 20), text.indexOf('intervals') + 180);
      return response;
    }
    const mid = '2024-01-01T00:00:30Z';
    const body = text.replace(found[0], `"intervals":[{"start":"${start[1]}","end":"${mid}"},{"start":"${mid}","end":"${end[1]}"}]`);
    window.__timelineSplit = true;
    const headers = new Headers(response.headers);
    headers.delete('content-length');
    return new Response(body, { status: response.status, statusText: response.statusText, headers });
  };
})();"#
        .to_owned()
}

fn assert_auto_playback_started(page: &mut Browser) {
    page.wait_until(
        "document.getElementById('video-player')?.hasAttribute('controls') && !document.getElementById('video-player')?.hasAttribute('hidden')",
    );
    assert_eq!(
        page.eval("!!document.getElementById('video-empty')"),
        serde_json::json!(false)
    );
}

fn assert_typed_range(page: &mut Browser) {
    let entered = page.eval(
        "(() => { const start = document.getElementById('video-range-start'); const end = document.getElementById('video-range-end'); if (!(start instanceof HTMLInputElement) || !(end instanceof HTMLInputElement)) return false; start.value = '2024-01-01T00:00:00Z'; end.value = '2024-01-01T00:00:02Z'; start.dispatchEvent(new Event('input', { bubbles: true })); end.dispatchEvent(new Event('input', { bubbles: true })); return true; })()",
    );
    assert_eq!(entered, serde_json::json!(true), "range fields missing");
    page.wait_until(
        "document.getElementById('video-timeline')?.dataset.start?.includes('2024-01-01T00:00:00') && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped') && document.getElementById('video-player')?.hasAttribute('controls')",
    );
}

fn play_coverage(page: &mut Browser) {
    page.wait_until("document.getElementById('video-interval-0')");
    page.click("video-interval-0");
}

fn key_down(page: &mut Browser, id: &str, key: &str, shift: bool) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let key = serde_json::to_string(key).unwrap_or_default();
    let shift = if shift { "true" } else { "false" };
    let sent = page.eval(&format!(
        "(() => {{ const el = document.getElementById({id}); if (!el) return false; el.focus(); el.dispatchEvent(new KeyboardEvent('keydown', {{ key: {key}, code: {key}, shiftKey: {shift}, bubbles: true, cancelable: true }})); return document.activeElement === el; }})()"
    ));
    assert_eq!(
        sent,
        serde_json::json!(true),
        "timeline did not receive {key}"
    );
}

fn text_attr(page: &mut Browser, id: &str, name: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    let name = serde_json::to_string(name).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.getAttribute({name}) ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn play_player(page: &mut Browser) {
    let started = page.eval(
        "(() => { const el = document.getElementById('video-player'); if (!(el instanceof HTMLVideoElement)) return false; el.muted = true; const playing = el.play(); if (playing && playing.catch) playing.catch(() => {}); if (typeof window.__a2aNativePlay === 'function') window.__a2aNativePlay(); else el.dispatchEvent(new Event('play')); return true; })()",
    );
    assert_eq!(started, serde_json::json!(true), "missing video player");
}

fn pause_player(page: &mut Browser) {
    page.eval("(() => { const el = document.getElementById('video-player'); if (el instanceof HTMLVideoElement) el.pause(); if (typeof window.__a2aNativePause === 'function') window.__a2aNativePause(); else el?.dispatchEvent(new Event('pause')); })()");
}

fn text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.innerText ?? document.getElementById({id})?.textContent ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}
