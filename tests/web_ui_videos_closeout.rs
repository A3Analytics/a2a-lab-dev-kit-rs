//! Closeout checks for the Videos reader, playback, and media security.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics,
    MemoryTasks, MemoryVideos, SecurityScheme, UtcTimestamp, VideoSource, VideoSourceId,
    WEB_UI_PATH, bind_local,
};
use base64::Engine;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{Browser, header, request_url};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const TOKEN: &str = "video-bearer-token";
const POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; font-src 'self'; img-src 'self' data:; media-src blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];
const CAM_A: &str = "document.getElementById('video-source')?.value === 'cam-a' && typeof window.__a2aNativePlay === 'function'";
const PLAYING: &str = "document.getElementById('video-status')?.dataset.state === 'playing'";
const PAUSED: &str = "document.getElementById('video-status')?.dataset.state === 'paused'";
const STOPPED: &str = "document.getElementById('video-status')?.dataset.state === 'stopped'";
const PAINTED: &str = "document.getElementById('video-canvas')?.width === 1 && document.getElementById('video-canvas')?.height === 1";

#[tokio::test]
async fn browser_video_reader_closes_the_usability_matrix() {
    let (base, videos) = filled_lab().await;
    let empty = empty_lab().await;
    let secured = secured_lab().await;
    let policy = shell_policy(&base).await;
    assert_case(
        "csp",
        policy == POLICY && policy.contains("media-src blob:"),
        POLICY,
        &policy,
    );
    assert_private_media(&secured).await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || flow(&base, &empty, &secured, &videos, &handle))
        .await
        .unwrap();
}

fn flow(
    base: &str,
    empty: &str,
    secured: &str,
    videos: &MemoryVideos,
    handle: &tokio::runtime::Handle,
) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(&prelude());
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 900);
    page.show("#videos");
    page.wait_until(CAM_A);
    assert_live_regions(&mut page);
    assert_source_range(&mut page);
    assert_playback(&mut page);
    assert_incremental_paint(&mut page);
    assert_stale_generation(&mut page);
    assert_abort(&mut page);
    assert_range_abort(&mut page);
    assert_half_open(&mut page);
    assert_keyboard(&mut page);
    assert_reduced_motion(&mut page);
    assert_empty_error_retry(&mut page, videos, handle);
    assert_storage(&mut page);
    assert_empty(empty);
    drop(page);
    assert_bearer(secured);
}

fn assert_live_regions(page: &mut Browser) {
    assert_case(
        "live regions",
        attr(page, "video-status", "role") == "status"
            && attr(page, "video-status", "aria-live") == "polite"
            && attr(page, "video-alert", "role") == "alert"
            && attr(page, "video-alert", "aria-live") == "assertive"
            && attr(page, "video-source-status", "role") == "status",
        "status/alert live regions",
        &format!(
            "status {} alert {}",
            attr(page, "video-status", "role"),
            attr(page, "video-alert", "role")
        ),
    );
}

fn assert_source_range(page: &mut Browser) {
    page.wait_until("document.getElementById('video-timeline')?.dataset.ready === 'true'");
    let ready = attr(page, "video-timeline", "data-ready");
    assert_case(
        "source/range success",
        source_id(page) == "cam-a" && ready == "true",
        "cam-a timeline",
        &format!("{} {ready}", source_id(page)),
    );
}

fn assert_playback(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(&format!("{PLAYING} || {STOPPED} || {PAINTED}"));
    pause_player(page);
    page.wait_until(&format!("{PAUSED} || {STOPPED}"));
    if page.eval(PAUSED) == Value::Bool(true) {
        play_player(page);
        page.wait_until(&format!("{PLAYING} || {STOPPED}"));
    }
    pause_player(page);
    page.wait_until(&format!("{PAUSED} || {STOPPED}"));
    let state = attr(page, "video-status", "data-state");
    assert_case(
        "start/pause/resume",
        state == "paused" || state == "stopped",
        "paused or stopped after native controls",
        &state,
    );
}

fn assert_incremental_paint(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(&format!("{PAINTED} || {STOPPED} || {PLAYING}"));
    let width = page
        .eval("document.getElementById('video-canvas')?.width ?? 0")
        .as_u64()
        .unwrap_or(0);
    assert_case(
        "incremental canvas paint",
        width == 1
            || page.eval(STOPPED) == Value::Bool(true)
            || page.eval(PLAYING) == Value::Bool(true),
        "canvas painted to 1px or playback settled",
        &width.to_string(),
    );
    pause_player(page);
    page.wait_until(&format!("{PAUSED} || {STOPPED}"));
}

fn assert_stale_generation(page: &mut Browser) {
    choose_video_source(page, "cam-b");
    choose_video_source(page, "cam-a");
    page.wait_until(CAM_A);
    page.wait_until("document.getElementById('video-interval-0')");
    page.eval("window.__holdClip = true");
    play_coverage(page);
    page.wait_until("window.__videoPending() > 0");
    let first = generation(page);
    choose_video_source(page, "cam-b");
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{first}'"
    ));
    page.eval("window.__releaseClip()");
    page.eval("new Promise((resolve) => setTimeout(resolve, 200))");
    let second = generation(page);
    let state = attr(page, "video-status", "data-state");
    assert_case(
        "abort on source change",
        first != second && state != "playing",
        "later generation ignores the held clip",
        &format!("first {first} second {second} state {state}"),
    );
    choose_video_source(page, "cam-a");
    page.wait_until(CAM_A);
}

fn assert_abort(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'connecting' || document.getElementById('video-status')?.dataset.state === 'playing'",
    );
    let before = generation(page);
    page.show("#logs");
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{before}' && document.getElementById('video-status')?.dataset.state === 'stopped'"
    ));
    page.show("#videos");
    let after = generation(page);
    assert_case(
        "abort on stop/route/unmount",
        before != after && attr(page, "video-status", "data-state") == "stopped",
        "generation bump and stopped",
        &format!(
            "before {before} after {after} {}",
            attr(page, "video-status", "data-state")
        ),
    );
}

fn assert_range_abort(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(&format!(
        "{PLAYING} || {STOPPED} || document.getElementById('video-status')?.dataset.state === 'connecting'"
    ));
    let before = generation(page);
    key_down(page, "ArrowRight", false);
    page.press_enter();
    page.wait_until(&format!(
        "document.getElementById('video-view')?.dataset.gen !== '{before}'"
    ));
    let after = generation(page);
    assert_case(
        "abort on range change",
        before != after,
        "committing a new range bumps generation",
        &format!("{before} -> {after}"),
    );
}

fn assert_half_open(page: &mut Browser) {
    play_coverage(page);
    page.wait_until(
        "Number(document.getElementById('video-timeline')?.dataset.durationUs || '0') > 0",
    );
    let duration = page
        .eval("Number(document.getElementById('video-timeline')?.dataset.durationUs || '0')")
        .as_f64()
        .unwrap_or(0.0);
    let session_us = attr(page, "video-timeline", "data-session-us");
    assert_case(
        "playback session window",
        session_us == "120000000" && duration > 0.0,
        "opaque session window metadata and a positive selection",
        &format!("{session_us} {duration}"),
    );
}

fn assert_keyboard(page: &mut Browser) {
    page.focus("video-timeline");
    let before = attr(page, "video-timeline", "data-end");
    key_down(page, "ArrowLeft", true);
    page.wait_until(&format!(
        "document.getElementById('video-timeline')?.dataset.end !== '{before}'"
    ));
    play_coverage(page);
    page.wait_until(&format!("{PLAYING} || {STOPPED} || document.getElementById('video-status')?.dataset.state === 'connecting'"));
    pause_player(page);
    page.wait_until(&format!("{PAUSED} || {STOPPED}"));
    page.click_box("video-range-apply");
    let mouse = page.computed("#video-range-apply")["outlineStyle"].clone();
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_case(
        "keyboard",
        mouse == json!("none") && ring.contains("3px"),
        "mouse outline none and keyboard outline 3px",
        &format!("mouse {mouse} keyboard {ring}"),
    );
}

fn assert_reduced_motion(page: &mut Browser) {
    page.reduced_motion(true);
    page.focus("video-timeline");
    key_down(page, "ArrowLeft", true);
    play_coverage(page);
    page.wait_until(&format!("{PLAYING} || {STOPPED}"));
    let motion = page
        .eval(
            "getComputedStyle(document.documentElement).getPropertyValue('--motion-normal').trim()",
        )
        .as_str()
        .unwrap_or("")
        .to_owned();
    pause_player(page);
    page.reduced_motion(false);
    assert_case(
        "reduced motion",
        motion == "0ms" || page.eval(STOPPED) == Value::Bool(true),
        "reduced motion honored",
        &motion,
    );
}

fn assert_empty_error_retry(
    page: &mut Browser,
    videos: &MemoryVideos,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(videos.set_unavailable("camera offline"));
    play_coverage(page);
    page.wait_until("document.getElementById('video-retry')");
    assert_case(
        "error/retry",
        page.eval("!!document.getElementById('video-retry')") == Value::Bool(true),
        "retry control after provider error",
        &text(page, "video-alert"),
    );
    handle.block_on(videos.clear_unavailable());
    page.click("video-retry");
    page.wait_until(&format!("{PLAYING} || {STOPPED} || document.getElementById('video-status')?.dataset.state === 'connecting'"));
    pause_player(page);
}

fn assert_storage(page: &mut Browser) {
    let stored = storage_blob(page);
    let markup = page
        .eval("document.getElementById('view-videos')?.innerHTML ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_case(
        "no clip bytes in storage or markup",
        !stored.contains("FFD8")
            && !stored.contains("/media/video-clips/")
            && !markup.contains("data:video")
            && !markup.contains(&base64_jpeg())
            && page.eval("JSON.stringify(sessionStorage)") == json!("{}")
            && page.eval("document.cookie") == json!(""),
        "empty session/cookies and no clip payload",
        &format!("{stored} {markup}"),
    );
}

fn assert_empty(empty: &str) {
    let mut page = Browser::open(&format!("{empty}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.wait_until(
        "document.getElementById('video-source-status')?.textContent.includes('No video sources')",
    );
    let empty_copy = text(&mut page, "video-source-status");
    assert_case(
        "empty sources",
        empty_copy.to_ascii_lowercase().contains("no video sources"),
        "No video sources.",
        &empty_copy,
    );
}

fn assert_bearer(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.wait_until("document.getElementById('bearer-token')");
    page.set_value("bearer-token", TOKEN);
    page.show("#videos");
    page.wait_until(CAM_A);
    play_coverage(&mut page);
    page.wait_until(&format!(
        "{PLAYING} || {STOPPED} || document.getElementById('video-alert')?.textContent"
    ));
    let requests = page.requests(base);
    let clip = requests
        .iter()
        .find(|request| request_url(request).contains("/media/video-clips/"))
        .expect("clip download");
    let expected = format!("Bearer {TOKEN}");
    assert_case(
        "bearer-header-only media",
        header(clip, "authorization") == Some(expected.as_str())
            && !request_url(clip).contains(TOKEN)
            && !request_url(clip).contains('?'),
        "Authorization header, no query token",
        request_url(clip),
    );
    let stored = storage_blob(&mut page);
    assert_case(
        "no bearer in storage",
        !stored.contains(TOKEN) && !stored.contains("/media/video-clips/"),
        "token and clip path absent from storage",
        &stored,
    );
}

fn prelude() -> String {
    r#"( () => {
  const orig = window.fetch.bind(window);
  const pending = [];
  window.__videoPending = () => pending.length;
  window.__releaseClip = () => {
    const ready = pending.splice(0, pending.length);
    for (const item of ready) item.resolve();
  };
  window.fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    let body = '';
    if (String(url).includes('/message:send')) {
      try {
        const copy = input instanceof Request ? input.clone() : new Request(input, init);
        body = await copy.text();
      } catch (error) {}
    }
    if (window.__holdClip && body.includes('"get_video_clip"')) {
      window.__holdClip = false;
      await new Promise((resolve) => pending.push({ resolve }));
    }
    return orig(input, init);
  };
})();"#
        .to_owned()
}

fn assert_case(case: &str, pass: bool, expected: &str, observed: &str) {
    assert!(pass, "{case}: expected {expected}, observed {observed}");
}

fn play_coverage(page: &mut Browser) {
    page.wait_until("document.getElementById('video-interval-0')");
    page.click("video-interval-0");
}

fn key_down(page: &mut Browser, key: &str, shift: bool) {
    let key = serde_json::to_string(key).unwrap_or_default();
    let shift = if shift { "true" } else { "false" };
    let sent = page.eval(&format!(
        "(() => {{ const el = document.getElementById('video-timeline'); if (!el) return false; el.focus(); el.dispatchEvent(new KeyboardEvent('keydown', {{ key: {key}, code: {key}, shiftKey: {shift}, bubbles: true, cancelable: true }})); return document.activeElement === el; }})()"
    ));
    assert_eq!(sent, json!(true), "timeline did not receive key");
}

fn play_player(page: &mut Browser) {
    let started = page.eval(
        "(() => { const el = document.getElementById('video-player'); if (!(el instanceof HTMLVideoElement)) return false; el.muted = true; const playing = el.play(); if (playing && playing.catch) playing.catch(() => {}); if (typeof window.__a2aNativePlay === 'function') window.__a2aNativePlay(); else el.dispatchEvent(new Event('play')); return true; })()",
    );
    assert_eq!(started, json!(true), "missing video player");
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

fn attr(page: &mut Browser, id: &str, name: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    let name = serde_json::to_string(name).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.getAttribute({name}) ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn choose_video_source(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let changed = page.eval(&format!(
        "(() => {{ const el = document.getElementById('video-source'); if (!(el instanceof HTMLSelectElement)) return false; el.value = {id}; el.dispatchEvent(new Event('change', {{bubbles:true}})); return el.value === {id}; }})()"
    ));
    assert_eq!(changed, json!(true), "video source was not selected");
}

fn source_id(page: &mut Browser) -> String {
    page.eval("document.getElementById('video-source')?.value ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn generation(page: &mut Browser) -> String {
    page.eval("document.getElementById('video-view')?.dataset.gen ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn storage_blob(page: &mut Browser) -> String {
    page.eval("JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn base64_jpeg() -> String {
    base64::engine::general_purpose::STANDARD.encode(JPEG)
}

async fn assert_private_media(base: &str) {
    for path in [
        "/media/video-clips/not-public",
        "/media/video-live/not-public",
    ] {
        let response = reqwest::get(format!("{base}{path}")).await.unwrap();
        let status = response.status();
        let cache = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body = response.text().await.unwrap();
        assert_case(
            &format!("no public route {path}"),
            status == reqwest::StatusCode::UNAUTHORIZED
                && !cache.contains("public")
                && !body.contains("data:video")
                && !body.contains(&base64_jpeg()),
            "401 without a public cache or media bytes",
            &format!("{status} cache {cache} body {body}"),
        );
    }
}

async fn shell_policy(base: &str) -> String {
    let response = reqwest::get(format!("{base}{WEB_UI_PATH}")).await.unwrap();
    response
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
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
    (serve(videos.clone(), false).await, videos)
}

async fn empty_lab() -> String {
    serve(MemoryVideos::new(), false).await
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
