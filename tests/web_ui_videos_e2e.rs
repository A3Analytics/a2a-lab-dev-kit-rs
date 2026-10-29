//! End-to-end Videos checks for expiry recovery, stable failure codes, and details.

#![cfg(feature = "web-ui")]

use std::sync::{Arc, Mutex};

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, FollowVideoRequest, GetVideoClipRequest,
    GetVideoFrameAtRequest, GetVideoFrameAtResult, GetVideoTimelineRequest,
    ListVideoSourcesRequest, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, Page, UtcTimestamp, VideoClip, VideoClipId, VideoClipStream, VideoFollow,
    VideoFollowId, VideoLiveStream, VideoProvider, VideoSource, VideoSourceId, VideoTimeline,
    bind_local,
};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{Browser, request_url};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

fn stamp(second: u64) -> String {
    let minutes = second / 60;
    let secs = second % 60;
    format!("2024-01-01T00:{minutes:02}:{secs:02}Z")
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

fn expect_case(scenario: &str, pass: bool, expected: &str, observed: &str) {
    assert!(pass, "{scenario}: expected {expected}, observed {observed}");
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

fn pause_player(page: &mut Browser) {
    page.eval("(() => { const el = document.getElementById('video-player'); if (el instanceof HTMLVideoElement) el.pause(); if (typeof window.__a2aNativePause === 'function') window.__a2aNativePause(); else el?.dispatchEvent(new Event('pause')); })()");
}

#[tokio::test]
async fn browser_videos_details_and_recoverable_errors() {
    let gated = GatedVideos::new(detail_catalog().await);
    let base = serve_gated(gated.clone()).await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || details_flow(&base, &gated, &handle))
        .await
        .unwrap();
}

fn details_flow(base: &str, videos: &GatedVideos, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(&body_prelude());
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#videos");
    assert_disclosure(&mut page);
    assert_playback_details(&mut page, base);
    assert_range_failures(&mut page, videos);
    assert_expired_retry(&mut page, videos, base);
    assert_generic_codes(&mut page, videos);
    page.click("video-follow-newest");
    expect_coded(
        &mut page,
        "live_unavailable",
        LIVE_UNAVAILABLE,
        "not available",
        true,
        true,
    );
    reselect(&mut page);
    page.click("video-follow-newest");
    expect_coded(
        &mut page,
        "live_unavailable",
        LIVE_UNAVAILABLE,
        "not available",
        true,
        true,
    );
    handle
        .block_on(
            videos
                .inner
                .set_live_available(&VideoSourceId::new("cam-a").unwrap(), true),
        )
        .unwrap();
    page.click("video-retry");
    page.wait_until(
        "document.getElementById('video-source')?.value === 'cam-a' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'connecting' || document.getElementById('video-status')?.dataset.state === 'stopped') && document.getElementById('video-alert')?.dataset.failure === ''",
    );
    assert_no_saved_stills(&mut page);
}

fn assert_range_failures(page: &mut Browser, videos: &GatedVideos) {
    videos.set_gate(ClipGate::NoRecording);
    play_coverage(page);
    expect_coded(
        page,
        "no_recording",
        NO_RECORDING,
        NOISE_RECORDING,
        false,
        true,
    );
    nudge_timeline(page);
    reselect(page);
    videos.set_gate(ClipGate::Gap);
    play_coverage(page);
    expect_coded(page, "recording_gap", RECORDING_GAP, NOISE_GAP, false, true);
    reselect(page);
}

fn assert_expired_retry(page: &mut Browser, videos: &GatedVideos, base: &str) {
    videos.set_gate(ClipGate::Expire);
    play_coverage(page);
    expect_coded(
        page,
        "clip_expired",
        CLIP_EXPIRED,
        "has expired",
        true,
        false,
    );
    let expired = media_paths(page, base, "/media/video-clips/");
    videos.set_gate(ClipGate::Pass);
    let before = clip_requests(page, base);
    let fetches = page.eval("window.__clipFetches || 0").as_u64().unwrap_or(0);
    page.click("video-retry");
    page.wait_until(&format!(
        "(window.__clipFetches || 0) > {fetches} && document.getElementById('video-alert')?.dataset.failure === '' && !(document.getElementById('video-alert-text')?.textContent.trim()) && document.getElementById('video-status')?.dataset.state === 'playing' && document.getElementById('video-player')?.hasAttribute('controls')"
    ));
    let after = clip_requests(page, base);
    let renewed = media_paths(page, base, "/media/video-clips/");
    expect_case(
        "clip retry after expiry stops",
        after > before && renewed.iter().any(|path| !expired.contains(path)),
        "a new clip handle is obtained and played",
        &format!(
            "clips {before} -> {after} expired {expired:?} renewed {renewed:?} status {}",
            text_attr(page, "video-status", "data-state")
        ),
    );
    pause_player(page);
}

fn assert_generic_codes(page: &mut Browser, videos: &GatedVideos) {
    assert_media_code(
        page,
        videos,
        ClipGate::MediaInvalid,
        "invalid",
        INVALID,
        false,
    );
    assert_media_code(
        page,
        videos,
        ClipGate::MediaNotFound,
        "not_found",
        NOT_FOUND,
        false,
    );
    assert_media_code(
        page,
        videos,
        ClipGate::MediaUnavailable,
        "unavailable",
        UNAVAILABLE,
        true,
    );
    assert_task_code_boundary(page, videos, ClipGate::TaskInvalid, "invalid", INVALID);
    assert_task_code_boundary(page, videos, ClipGate::TaskNotFound, "not_found", NOT_FOUND);
    assert_task_code_boundary(
        page,
        videos,
        ClipGate::TaskUnavailable,
        "unavailable",
        UNAVAILABLE,
    );
    videos.set_gate(ClipGate::Pass);
    recover_playback(page);
}

fn assert_disclosure(page: &mut Browser) {
    page.wait_until(
        "document.getElementById('video-interval-0') && !document.getElementById('video-frame-at')?.disabled",
    );
    assert_eq!(pressed_source(page), "cam-a");
    assert_regions(page);
    assert_details_visible(page);
    assert_idle_facts(page);
    assert_focus_order(page);
}

fn assert_playback_details(page: &mut Browser, base: &str) {
    page.eval(
        "window.__videoBusy = false; window.__videoBusyWatch = setInterval(() => { if (document.getElementById('video-view')?.getAttribute('aria-busy') === 'true') window.__videoBusy = true; }, 10);",
    );
    play_coverage(page);
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped'",
    );
    assert_eq!(
        page.eval("window.__videoBusy === true"),
        serde_json::json!(true),
        "loading did not expose aria-busy"
    );
    page.eval("clearInterval(window.__videoBusyWatch)");
    pause_player(page);
    page.wait_until("document.getElementById('video-details')");
    assert_playback_facts(page);
    page.wait_until("!document.getElementById('video-frame-at')?.disabled");
    page.click("video-frame-at");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.frameTime?.includes('2024-01-01') && document.getElementById('video-view')?.getAttribute('aria-busy') === 'false'",
    );
    let frame = page
        .eval("document.getElementById('video-status')?.dataset.frameTime ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(frame.contains("2024-01-01"), "{frame}");
    assert_no_saved_stills(page);
    let requests = page.requests(base);
    let bodies = request_bodies(&requests);
    assert!(bodies.contains("get_video_frame_at"), "{bodies}");
    assert!(!bodies.contains("list_images"), "{bodies}");
}

fn assert_regions(page: &mut Browser) {
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
    assert_eq!(
        page.eval("document.getElementById('video-view')?.hasAttribute('aria-busy')"),
        serde_json::json!(true)
    );
}

fn assert_details_visible(page: &mut Browser) {
    page.wait_until("document.getElementById('video-details')");
    assert_eq!(
        page.eval("document.getElementById('video-filters')?.contains(document.getElementById('video-details'))"),
        serde_json::json!(true)
    );
    assert_eq!(
        page.eval("!!document.getElementById('video-details-toggle')"),
        serde_json::json!(false)
    );
}

fn assert_focus_order(page: &mut Browser) {
    assert_eq!(
        page.eval(
            "(() => [...document.querySelectorAll('#video-browser button, #video-browser select, #video-browser input, #video-browser [tabindex=\"0\"]')].filter((el) => !el.disabled && el.getClientRects().length > 0).map((el) => el.id))()"
        ),
        serde_json::json!([
            "video-source",
            "video-timeline",
            "video-range-start",
            "video-range-end",
            "video-follow-newest",
            "video-frame-at"
        ])
    );
    assert_eq!(
        page.eval(
            "(() => [...document.querySelectorAll('#video-browser button, #video-browser select, #video-browser input, #video-browser [role=slider]')].every((el) => ((el.getAttribute('aria-label') || el.labels?.[0]?.textContent || el.textContent || '').trim().length > 0)))()"
        ),
        serde_json::json!(true)
    );
    page.focus("video-follow-newest");
    let live = page
        .eval("document.getElementById('video-follow-newest')?.textContent.trim()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(live, "LIVE");
}

fn assert_idle_facts(page: &mut Browser) {
    assert_range_facts(page);
}

fn assert_playback_facts(page: &mut Browser) {
    assert_range_facts(page);
}

fn assert_range_facts(page: &mut Browser) {
    let labels = detail_labels(page);
    assert_eq!(labels, ["Start time", "End time"], "{labels:?}");
    let text = page
        .eval("document.getElementById('video-details')?.innerText ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(!text.contains("Connection"), "{text}");
    assert!(!text.contains("Playback"), "{text}");
    assert!(!text.contains("Bench"), "{text}");
    assert!(!text.contains("Camera"), "{text}");
    let start = detail_utc(page, "Start time");
    let end = detail_utc(page, "End time");
    assert!(start.contains("2024-01-01"), "{start}");
    assert!(end.contains("2024-01-01"), "{end}");
}

fn detail_utc(page: &mut Browser, label: &str) -> String {
    let label = serde_json::to_string(label).unwrap_or_default();
    page.eval(&format!(
        "document.querySelector('#video-details dd[data-label={label}]')?.dataset.utc ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn assert_media_code(
    page: &mut Browser,
    videos: &GatedVideos,
    gate: ClipGate,
    code: &str,
    sentence: &str,
    retry: bool,
) {
    page.eval("window.__bodies = []");
    videos.set_gate(gate);
    play_coverage(page);
    let noise = match gate {
        ClipGate::MediaInvalid => "not a stable sentence",
        ClipGate::MediaNotFound => "missing-handle",
        _ => "camera offline",
    };
    expect_coded(page, code, sentence, noise, retry, false);
    let bodies = captured_bodies(page);
    expect_case(
        &format!("media {code}"),
        stable_codes(&bodies).iter().any(|item| item == code),
        &format!("media error.code {code}"),
        &bodies,
    );
    videos.set_gate(ClipGate::Pass);
    if retry {
        page.click("video-retry");
    } else {
        play_coverage(page);
    }
    page.wait_until(
        "document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-status')?.dataset.state === 'connecting')",
    );
    pause_player(page);
}

fn assert_task_code_boundary(
    page: &mut Browser,
    videos: &GatedVideos,
    gate: ClipGate,
    code: &str,
    sentence: &str,
) {
    page.eval("window.__bodies = []");
    videos.set_gate(gate);
    play_coverage(page);
    page.wait_until(
        "(document.getElementById('video-alert-text')?.textContent.trim().length || 0) > 0",
    );
    let bodies = captured_bodies(page);
    let present = stable_codes(&bodies).iter().any(|item| item == code);
    let failure = text_attr(page, "video-alert", "data-failure");
    let alert = text(page, "video-alert-text");
    if present {
        expect_case(
            &format!("task {code}"),
            failure == code && alert.trim() == sentence,
            &format!("{sentence} because the stable code is present"),
            &format!("failure {failure} alert {alert} body {bodies}"),
        );
    } else {
        // A2A task failures for invalid, not_found, and unavailable are not
        // attached as ErrorInfo metadata.code. The sentence is asserted only
        // when that code, or media error.code, is present.
        expect_case(
            &format!("task {code} without stable code"),
            failure != code && alert.trim() != sentence,
            "coded sentence withheld until ErrorInfo metadata.code or media error.code is present",
            &format!("failure {failure} alert {alert} body {bodies}"),
        );
    }
    videos.set_gate(ClipGate::Pass);
    recover_playback(page);
}

fn recover_playback(page: &mut Browser) {
    let retry = page.eval("!!document.getElementById('video-retry')") == serde_json::json!(true);
    if retry {
        page.click("video-retry");
    } else {
        play_coverage(page);
    }
    page.wait_until(
        "document.getElementById('video-alert')?.dataset.failure === '' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-status')?.dataset.state === 'connecting')",
    );
    pause_player(page);
}

fn captured_bodies(page: &mut Browser) -> String {
    page.eval("JSON.stringify(window.__bodies || [])")
        .as_str()
        .unwrap_or("[]")
        .to_owned()
}

fn stable_codes(raw: &str) -> Vec<String> {
    let Ok(bodies) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    let mut codes = Vec::new();
    for body in bodies {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        collect_codes(&value, &mut codes);
    }
    codes
}

fn collect_codes(value: &serde_json::Value, codes: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(code) = map
                .get("metadata")
                .and_then(|metadata| metadata.get("code"))
                .and_then(|code| code.as_str())
            {
                codes.push(code.to_owned());
            }
            if map.get("message").is_some()
                && let Some(code) = map.get("code").and_then(|code| code.as_str())
            {
                codes.push(code.to_owned());
            }
            for child in map.values() {
                collect_codes(child, codes);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_codes(item, codes);
            }
        }
        _ => {}
    }
}

fn body_prelude() -> String {
    r"( () => {
  const orig = window.fetch.bind(window);
  window.__bodies = [];
  window.fetch = async (input, init) => {
    const response = await orig(input, init);
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    const href = String(url);
    if (href.includes('/media/video-clips/')) window.__clipFetches = (window.__clipFetches || 0) + 1;
    if (href.includes('/message:send') || (href.includes('/media/') && response.status !== 200)) {
      try {
        const text = await response.clone().text();
        window.__bodies.push(text.slice(0, 4000));
      } catch (error) {}
    }
    return response;
  };
})();"
        .to_owned()
}

fn expect_coded(
    page: &mut Browser,
    code: &str,
    sentence: &str,
    noise: &str,
    retry: bool,
    reselect_range: bool,
) {
    let code_json = serde_json::to_string(code).unwrap_or_default();
    let sentence_json = serde_json::to_string(sentence).unwrap_or_default();
    page.wait_until(&format!(
        "document.getElementById('video-alert')?.dataset.failure === {code_json} && document.getElementById('video-alert-text')?.textContent.trim() === {sentence_json}"
    ));
    let alert = text(page, "video-alert");
    assert!(!alert.contains(noise), "{alert}");
    assert_eq!(pressed_source(page), "cam-a");
    assert!(sentence_once(page, sentence), "{alert}");
    assert_eq!(
        page.eval("!!document.getElementById('video-retry')"),
        serde_json::json!(retry)
    );
    assert_eq!(
        page.eval("!!document.getElementById('video-reselect')"),
        serde_json::json!(reselect_range)
    );
    assert_eq!(
        page.eval("document.getElementById('video-timeline')?.dataset.ready"),
        serde_json::json!("true")
    );
    assert_eq!(
        page.eval("document.getElementById('video-view')?.getAttribute('aria-busy')"),
        serde_json::json!("false")
    );
    assert_eq!(
        page.eval("document.getElementById('video-status')?.getAttribute('aria-live')"),
        serde_json::json!("polite")
    );
}

fn sentence_once(page: &mut Browser, sentence: &str) -> bool {
    let sentence = serde_json::to_string(sentence).unwrap_or_default();
    page.eval(&format!(
        "(() => {{ const view = document.getElementById('video-view'); const text = view?.innerText || ''; const status = document.getElementById('video-status')?.innerText || ''; const alerts = [...view.querySelectorAll('[role=alert]')].filter((el) => el.innerText.trim()); return text.split({sentence}).length - 1 === 1 && !status.includes({sentence}) && alerts.length === 1; }})()"
    )) == serde_json::json!(true)
}

fn reselect(page: &mut Browser) {
    page.click("video-reselect");
    page.wait_until(
        "!document.getElementById('video-reselect') && !(document.getElementById('video-alert-text')?.textContent.trim()) && document.getElementById('video-source')?.value === 'cam-a' && document.getElementById('video-timeline')?.dataset.ready === 'true' && document.getElementById('video-timeline')?.dataset.selection === 'empty'",
    );
}

fn nudge_timeline(page: &mut Browser) {
    page.focus("video-timeline");
    let before = text_attr(page, "video-timeline", "data-start");
    key_down(page, "video-timeline", "ArrowRight", false);
    page.wait_until(&format!(
        "document.getElementById('video-timeline')?.dataset.start !== '{}'",
        before.replace('\'', "")
    ));
}

fn assert_no_saved_stills(page: &mut Browser) {
    let view = page
        .eval("document.getElementById('view-videos')?.innerText ?? ''")
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(!view.contains("saved image"), "{view}");
    assert!(!view.contains("still image"), "{view}");
    assert!(!view.contains("saved still"), "{view}");
    assert_eq!(
        page.eval("document.getElementById('video-frame-at')?.textContent.trim()"),
        serde_json::json!("Frame at time")
    );
    assert_eq!(
        page.eval("document.getElementById('view-images')?.hasAttribute('hidden')"),
        serde_json::json!(true)
    );
}

fn detail_labels(page: &mut Browser) -> Vec<String> {
    let value = page.eval(
        "[...document.querySelectorAll('#video-details dt')].map((el) => el.textContent.trim())",
    );
    value
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn pressed_source(page: &mut Browser) -> String {
    page.eval("document.getElementById('video-source')?.value ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn clip_requests(page: &Browser, base: &str) -> usize {
    page.requests(base)
        .iter()
        .filter(|request| {
            request["params"]["request"]["postData"]
                .as_str()
                .is_some_and(|body| body.contains("get_video_clip"))
        })
        .count()
}

fn request_bodies(requests: &[serde_json::Value]) -> String {
    requests
        .iter()
        .filter_map(|request| request["params"]["request"]["postData"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
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

const NO_RECORDING: &str = "No recording covers the requested range.";
const RECORDING_GAP: &str = "The recording contains a gap.";
const CLIP_EXPIRED: &str = "The clip expired.";
const LIVE_UNAVAILABLE: &str = "Live video is unavailable.";
const INVALID: &str = "The video request is invalid.";
const NOT_FOUND: &str = "The requested video was not found.";
const UNAVAILABLE: &str = "Video is unavailable.";
const NOISE_RECORDING: &str = "no_recording: human text";
const NOISE_GAP: &str = "recording gap in the message";

#[derive(Clone, Copy)]
enum ClipGate {
    Pass,
    NoRecording,
    Gap,
    Expire,
    MediaInvalid,
    MediaNotFound,
    MediaUnavailable,
    TaskInvalid,
    TaskNotFound,
    TaskUnavailable,
}

#[derive(Clone)]
struct GatedVideos {
    inner: MemoryVideos,
    gate: Arc<Mutex<ClipGate>>,
}

impl GatedVideos {
    fn new(inner: MemoryVideos) -> Self {
        Self {
            inner,
            gate: Arc::new(Mutex::new(ClipGate::Pass)),
        }
    }

    fn set_gate(&self, gate: ClipGate) {
        *self.gate.lock().unwrap() = gate;
    }

    fn gate(&self) -> ClipGate {
        *self.gate.lock().unwrap()
    }
}

impl VideoProvider for GatedVideos {
    async fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> Result<Page<VideoSource>, A2aLabError> {
        self.inner.list_video_sources(request).await
    }

    async fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> Result<GetVideoFrameAtResult, A2aLabError> {
        self.inner.get_video_frame_at(request).await
    }

    async fn get_video_clip(&self, request: GetVideoClipRequest) -> Result<VideoClip, A2aLabError> {
        match self.gate() {
            ClipGate::Pass
            | ClipGate::Expire
            | ClipGate::MediaInvalid
            | ClipGate::MediaNotFound
            | ClipGate::MediaUnavailable => self.inner.get_video_clip(request).await,
            ClipGate::NoRecording => Err(A2aLabError::no_recording(NOISE_RECORDING)),
            ClipGate::Gap => Err(A2aLabError::recording_gap(NOISE_GAP)),
            ClipGate::TaskInvalid => Err(A2aLabError::invalid("range", "bad range text")),
            ClipGate::TaskNotFound => Err(A2aLabError::not_found("video source", "cam-a")),
            ClipGate::TaskUnavailable => Err(A2aLabError::unavailable("camera offline")),
        }
    }

    async fn open_video_clip(&self, id: &VideoClipId) -> Result<VideoClipStream, A2aLabError> {
        match self.gate() {
            ClipGate::MediaInvalid => Err(A2aLabError::invalid("id", "not a stable sentence")),
            ClipGate::MediaNotFound => Err(A2aLabError::not_found("video clip", "missing-handle")),
            ClipGate::MediaUnavailable => Err(A2aLabError::unavailable("camera offline")),
            ClipGate::Expire => {
                self.inner.expire_clips().await;
                self.inner.open_video_clip(id).await
            }
            _ => self.inner.open_video_clip(id).await,
        }
    }

    async fn get_video_timeline(
        &self,
        request: GetVideoTimelineRequest,
    ) -> Result<VideoTimeline, A2aLabError> {
        self.inner.get_video_timeline(request).await
    }

    async fn follow_video(&self, request: FollowVideoRequest) -> Result<VideoFollow, A2aLabError> {
        self.inner.follow_video(request).await
    }

    async fn open_video_follow(&self, id: &VideoFollowId) -> Result<VideoLiveStream, A2aLabError> {
        self.inner.open_video_follow(id).await
    }
}

async fn detail_catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new("cam-a").unwrap(),
                "Camera A",
                "Camera A recording",
                None,
                None,
                320,
                180,
                1,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    for second in 0..=70 {
        insert_frame(&videos, "cam-a", &stamp(second)).await;
    }
    videos
}

async fn serve_gated(videos: GatedVideos) -> String {
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
