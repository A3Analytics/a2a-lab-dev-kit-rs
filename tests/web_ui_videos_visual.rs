//! Light and dark layout checks for the Videos reader.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, UtcTimestamp, VideoSource, VideoSourceId, bind_local,
};
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];
const READY: &str = "document.getElementById('video-source')?.value === 'cam-a'";
const POPULATED: &str = "document.getElementById('video-source')?.value === 'cam-a' && (document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'stopped' || document.getElementById('video-canvas')?.width === 1)";
const EMPTY: &str =
    "document.getElementById('video-source-status')?.textContent.includes('No video sources')";

#[tokio::test]
async fn browser_video_visual_matrix_covers_themes_and_widths() {
    let (populated, videos) = populated_lab().await;
    let empty = empty_lab().await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || flow(&populated, &empty, &videos, &handle))
        .await
        .unwrap();
}

fn flow(
    populated: &str,
    empty: &str,
    videos: &a2a_lab_dev_kit::MemoryVideos,
    handle: &tokio::runtime::Handle,
) {
    let mut page = Browser::open(&format!("{populated}/ui/"));
    page.wait_until(CONNECTED);
    let failures = visual_matrix(&mut page, empty, videos, handle);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn visual_matrix(
    page: &mut Browser,
    empty: &str,
    videos: &a2a_lab_dev_kit::MemoryVideos,
    handle: &tokio::runtime::Handle,
) -> Vec<String> {
    let mut failures = Vec::new();
    for scheme in ["light", "dark"] {
        apply_scheme(page, scheme);
        page.viewport(1843, 900);
        page.show("#videos");
        page.wait_until(READY);
        judge_focus(page, scheme, &mut failures);
        for width in [767, 768, 1843] {
            page.viewport(width, 900);
            show_populated(page);
            close_details(page);
            failures.extend(visual_case(page, scheme, width, "populated"));
            open_details(page);
            failures.extend(visual_case(page, scheme, width, "details"));
            close_details(page);
        }
    }
    failures.extend(error_matrix(page, videos, handle));
    page.goto(&format!("{empty}/ui/"));
    page.wait_until(CONNECTED);
    for scheme in ["light", "dark"] {
        apply_scheme(page, scheme);
        for width in [767, 768, 1843] {
            page.viewport(width, 900);
            page.show("#videos");
            page.wait_until(EMPTY);
            failures.extend(visual_case(page, scheme, width, "empty"));
        }
    }
    failures
}

fn judge_focus(page: &mut Browser, scheme: &str, failures: &mut Vec<String>) {
    page.wait_until("document.getElementById('video-range-apply')");
    page.click_box("video-range-apply");
    let mouse = page.computed("#video-range-apply")["outlineStyle"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    judge(
        failures,
        &format!("{scheme} focus"),
        "focus",
        mouse == "none" && ring.contains("3px"),
        "mouse outline none and keyboard outline 3px",
        &format!("mouse {mouse} keyboard {ring}"),
    );
}

fn close_details(_page: &mut Browser) {}

fn open_details(page: &mut Browser) {
    page.wait_until("document.getElementById('video-details')");
}

fn error_matrix(
    page: &mut Browser,
    videos: &a2a_lab_dev_kit::MemoryVideos,
    handle: &tokio::runtime::Handle,
) -> Vec<String> {
    let mut failures = Vec::new();
    for scheme in ["light", "dark"] {
        handle.block_on(videos.clear_unavailable());
        apply_scheme(page, scheme);
        page.show("#videos");
        page.wait_until(READY);
        page.wait_until("document.getElementById('video-interval-0')");
        handle.block_on(videos.set_unavailable("camera offline"));
        page.click("video-interval-0");
        page.wait_until(
            "(document.getElementById('video-alert-text')?.textContent.trim().length || 0) > 0",
        );
        for width in [767, 768, 1843] {
            page.viewport(width, 900);
            failures.extend(visual_case(page, scheme, width, "error"));
        }
    }
    handle.block_on(videos.clear_unavailable());
    failures
}

fn show_populated(page: &mut Browser) {
    page.show("#videos");
    page.wait_until(READY);
    page.wait_until("document.getElementById('video-interval-0')");
    page.click("video-interval-0");
    page.wait_until(POPULATED);
}

fn visual_case(page: &mut Browser, scheme: &str, width: u32, state: &str) -> Vec<String> {
    let report = page.eval(VISUAL);
    let case = format!("{scheme} {width}px {state}");
    let mut failures = Vec::new();
    judge_frame(&mut failures, &case, width, &report);
    judge_theme(&mut failures, &case, &report);
    if state == "populated" {
        judge_populated(&mut failures, &case, &report);
    } else if state == "details" {
        judge_details(&mut failures, &case, &report);
    } else if state == "error" {
        judge_error(&mut failures, &case, &report);
    } else {
        judge_empty(&mut failures, &case, &report);
    }
    failures
}

fn judge_frame(failures: &mut Vec<String>, case: &str, width: u32, report: &Value) {
    judge(
        failures,
        case,
        "title",
        report["titles"] == json!(["Videos"]) || flag(report, "heading"),
        "one visible Videos heading or labeled view",
        &report["titles"].to_string(),
    );
    judge(
        failures,
        case,
        "overflow",
        flag(report, "overflow"),
        "no horizontal page overflow",
        &format!("scrollWidth within viewport {}", flag(report, "overflow")),
    );
    judge(
        failures,
        case,
        "parts",
        num(report, "filterHeight") > 20.0 && num(report, "resultHeight") > 20.0,
        "source and playback are visible",
        &format!(
            "heights {} {}",
            num(report, "filterHeight"),
            num(report, "resultHeight")
        ),
    );
    if width < 768 {
        judge(
            failures,
            case,
            "stack",
            flag(report, "stacked") && !flag(report, "row") && num(report, "filterWidth") > 300.0,
            "stacked rails wider than 300px",
            &format!(
                "stacked {} row {} width {}",
                flag(report, "stacked"),
                flag(report, "row"),
                num(report, "rail")
            ),
        );
    } else {
        judge(
            failures,
            case,
            "rail",
            !flag(report, "stacked") && flag(report, "row") && near(num(report, "rail"), 240.0),
            "15rem source rail beside playback",
            &format!(
                "stacked {} row {} width {}",
                flag(report, "stacked"),
                flag(report, "row"),
                num(report, "rail")
            ),
        );
    }
}

fn judge_theme(failures: &mut Vec<String>, case: &str, report: &Value) {
    let tokens = text_value(report, "bodyBg") == text_value(report, "tokenBg")
        && text_value(report, "bodyFg") == text_value(report, "tokenFg")
        && !text_value(report, "bodyBg").is_empty();
    judge(
        failures,
        case,
        "tokens",
        tokens,
        "body colors match theme tokens",
        &format!(
            "bg {}/{} fg {}/{}",
            text_value(report, "bodyBg"),
            text_value(report, "tokenBg"),
            text_value(report, "bodyFg"),
            text_value(report, "tokenFg")
        ),
    );
    judge(
        failures,
        case,
        "motion",
        text_value(report, "motion") == "0ms" && text_value(report, "wave") == "none",
        "reduced motion duration 0ms and strand animation none",
        &format!(
            "motion {} wave {}",
            text_value(report, "motion"),
            text_value(report, "wave")
        ),
    );
    judge(
        failures,
        case,
        "shell",
        flag(report, "inventory") && flag(report, "runner"),
        "hidden inventory and command runner",
        &format!(
            "inventory {} runner {}",
            flag(report, "inventory"),
            flag(report, "runner")
        ),
    );
}

fn judge_populated(failures: &mut Vec<String>, case: &str, report: &Value) {
    judge(
        failures,
        case,
        "player",
        flag(report, "player") && flag(report, "controls"),
        "playback video and controls visible",
        &format!(
            "player {} controls {}",
            flag(report, "player"),
            flag(report, "controls")
        ),
    );
    judge(
        failures,
        case,
        "names",
        num(report, "frame") == 0.0
            && num(report, "timeline") > 2.0
            && text_value(report, "timelineName").contains("Recording")
            && text_value(report, "frameName").is_empty(),
        "named timeline without a seek control",
        &format!(
            "follow {} frame {} timeline {} names {} / {}",
            num(report, "follow"),
            num(report, "frame"),
            num(report, "timeline"),
            text_value(report, "timelineName"),
            text_value(report, "frameName")
        ),
    );
    let status = text_value(report, "status");
    judge(
        failures,
        case,
        "copy",
        !status.contains("could not be read") && !status.contains("Connecting"),
        "ready playback without loading or error copy",
        &format!("status {status} alert {}", text_value(report, "alert")),
    );
}

fn judge_details(failures: &mut Vec<String>, case: &str, report: &Value) {
    judge(
        failures,
        case,
        "details",
        flag(report, "detailsOpen")
            && flag(report, "detailsInside")
            && !flag(report, "detailsOverlap")
            && !flag(report, "detailsClip")
            && text_value(report, "detailsText")
                .to_ascii_lowercase()
                .contains("time range")
            && text_value(report, "detailsText").contains("Start")
            && text_value(report, "detailsText").contains("End")
            && !text_value(report, "detailsText").contains("Connection")
            && !text_value(report, "detailsText").contains("Playback"),
        "open details stay inside the scrollport without overlap or clipped facts",
        &format!(
            "open {} inside {} overlap {} clip {} text {}",
            flag(report, "detailsOpen"),
            flag(report, "detailsInside"),
            flag(report, "detailsOverlap"),
            flag(report, "detailsClip"),
            text_value(report, "detailsText")
        ),
    );
}

fn judge_error(failures: &mut Vec<String>, case: &str, report: &Value) {
    judge(
        failures,
        case,
        "error",
        !text_value(report, "alertText").is_empty()
            && flag(report, "retry")
            && flag(report, "alertInside")
            && !flag(report, "alertOverlap")
            && num(report, "timeline") > 2.0
            && num(report, "follow") > 2.0,
        "error alert and timeline stay visible without overlap",
        &format!(
            "alert {} retry {} inside {} overlap {} timeline {}",
            text_value(report, "alertText"),
            flag(report, "retry"),
            flag(report, "alertInside"),
            flag(report, "alertOverlap"),
            num(report, "timeline")
        ),
    );
}

fn judge_empty(failures: &mut Vec<String>, case: &str, report: &Value) {
    let status = text_value(report, "sourceStatus");
    judge(
        failures,
        case,
        "copy",
        status.contains("No video sources")
            && text_value(report, "alert").is_empty()
            && text_value(report, "emptyHint").contains("Select a source and a time")
            && !flag(report, "controls"),
        "empty sources and viewer without media controls",
        &format!(
            "status {status} hint {} controls {}",
            text_value(report, "emptyHint"),
            flag(report, "controls")
        ),
    );
    judge(
        failures,
        case,
        "names",
        num(report, "follow") == 0.0
            && num(report, "frame") == 0.0
            && num(report, "timeline") > 2.0,
        "timeline stays available without a live or seek button",
        &format!(
            "follow {} frame {} timeline {}",
            num(report, "follow"),
            num(report, "frame"),
            num(report, "timeline")
        ),
    );
}

fn judge(
    failures: &mut Vec<String>,
    case: &str,
    name: &str,
    pass: bool,
    expected: &str,
    actual: &str,
) {
    if !pass {
        failures.push(format!(
            "{case}: {name} expected {expected}, observed {actual}"
        ));
    }
}

fn apply_scheme(page: &mut Browser, scheme: &str) {
    let quoted = serde_json::to_string(scheme).unwrap_or_default();
    page.eval(&format!("localStorage.setItem('theme', {quoted})"));
    page.reload();
    let class = if scheme == "dark" {
        "document.documentElement.classList.contains('dark')"
    } else {
        "!document.documentElement.classList.contains('dark')"
    };
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === {quoted} && {class} && {CONNECTED}"
    ));
    page.set_media(scheme, true);
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn num(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn text_value(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 2.0
}

const VISUAL: &str = r"( () => {
  const paint = (name) => {
    const probe = document.createElement('div');
    probe.style.backgroundColor = 'var(' + name + ')';
    document.body.appendChild(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  };
  const box = (id) => {
    const el = document.getElementById(id);
    if (!el) return { width: 0, height: 0, top: 0, left: 0, right: 0 };
    const rect = el.getBoundingClientRect();
    return { width: rect.width, height: rect.height, top: rect.top, left: rect.left, right: rect.right };
  };
  const filters = box('video-filters');
  const results = box('video-results');
  const player = document.getElementById('video-player');
  const titles = [...document.querySelectorAll('h1, h2')].filter((node) => {
    if (node.closest('[hidden]')) return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 2 && rect.height > 2;
  }).map((node) => node.textContent.trim()).filter((title) => ['Agent', 'Logs', 'Metrics', 'Tasks', 'Images', 'Videos', 'Chat'].includes(title));
  const heading = document.getElementById('video-view-heading')?.textContent.trim() === 'Videos';
  const body = getComputedStyle(document.body);
  const browser = document.getElementById('video-browser');
  const details = document.getElementById('video-details');
  if (details) details.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  const alert = document.getElementById('video-alert');
  if (alert && (alert.textContent || '').trim()) alert.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  const overlaps = (panel, other) => {
    if (!panel || !other || panel.height < 2 || other.height < 2) return false;
    const x = Math.min(panel.right, other.right) - Math.max(panel.left, other.left);
    const y = Math.min(panel.bottom, other.bottom) - Math.max(panel.top, other.top);
    return x > 2 && y > 2;
  };
  const inside = (panel) => {
    const scroll = document.querySelector('#video-results .log-scroll');
    if (!panel || !scroll || panel.height < 8) return false;
    const port = scroll.getBoundingClientRect();
    return panel.top >= port.top - 1 && panel.bottom <= port.bottom + 1 && panel.left >= -1 && panel.right <= window.innerWidth + 1;
  };
  const insideFilters = (panel) => {
    const rail = document.getElementById('video-filters');
    if (!panel || !rail || panel.height < 8) return false;
    const port = rail.getBoundingClientRect();
    return panel.top >= port.top - 1 && panel.bottom <= port.bottom + 1 && panel.left >= port.left - 1 && panel.right <= port.right + 1;
  };
  const detailsBox = details ? details.getBoundingClientRect() : null;
  const alertBox = alert ? alert.getBoundingClientRect() : null;
  const timelineBox = document.getElementById('video-timeline')?.getBoundingClientRect() || null;
  return {
    titles,
    heading,
    rail: filters.width,
    stacked: browser ? getComputedStyle(browser).flexDirection === 'column' : false,
    row: Math.abs(filters.top - results.top) < 4 && filters.right <= results.left + 2,
    overflow: document.documentElement.scrollWidth <= window.innerWidth + 1,
    filterWidth: filters.width,
    filterHeight: filters.height,
    resultWidth: results.width,
    resultHeight: results.height,
    innerWidth: window.innerWidth,
    bodyBg: body.backgroundColor,
    tokenBg: paint('--background'),
    bodyFg: body.color,
    tokenFg: paint('--foreground'),
    motion: getComputedStyle(document.documentElement).getPropertyValue('--motion-normal').trim(),
    wave: getComputedStyle(document.querySelector('.strand-wave')).animationName,
    player: !!(player && !player.hidden && player.getBoundingClientRect().width > 2),
    controls: !!(player && !player.hidden && getComputedStyle(document.getElementById('video-timeline')).opacity === '1'),
    follow: box('video-follow-newest').width,
    frame: box('video-frame-at').width,
    timeline: box('video-timeline').width,
    timelineName: document.getElementById('video-timeline')?.getAttribute('aria-label') || '',
    frameName: document.getElementById('video-frame-at')?.textContent.trim() || '',
    emptyHint: document.getElementById('video-empty')?.textContent.trim() || '',
    status: document.getElementById('video-status')?.textContent.trim() || '',
    sourceStatus: document.getElementById('video-source-status')?.textContent.trim() || '',
    alert: document.getElementById('video-alert')?.textContent.trim() || '',
    inventory: document.getElementById('inventory-videos')?.hasAttribute('hidden') === true,
    runner: document.getElementById('workbench')?.hasAttribute('hidden') === true,
    detailsOpen: !!details,
    detailsInside: insideFilters(detailsBox),
    detailsOverlap: overlaps(detailsBox, timelineBox),
    detailsClip: details ? [...details.querySelectorAll('label, input')].some((el) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) : false,
    detailsText: details?.innerText.trim() || '',
    alertText: document.getElementById('video-alert-text')?.textContent.trim() || '',
    retry: !!document.getElementById('video-retry'),
    alertInside: inside(alertBox),
    alertOverlap: overlaps(alertBox, timelineBox)
  };
})()";

async fn populated_lab() -> (String, MemoryVideos) {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    (serve(videos.clone()).await, videos)
}

async fn empty_lab() -> String {
    serve(MemoryVideos::new()).await
}

async fn serve(videos: MemoryVideos) -> String {
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
