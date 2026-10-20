//! Light and dark layout checks for the Images reader.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, Image, ImageDescriptor, ImageId, ImageSource, ImageSourceId,
    JsonObject, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, UtcTimestamp, bind_local,
};
use base64::Engine;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const READY: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-history-status')?.dataset.state === 'ready'";
const PREVIEW_SOURCE: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-preview' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'png-ok'";
const PNG_READY: &str = "document.getElementById('image-modal')?.dataset.state === 'selected-ready' && document.getElementById('image-modal')?.dataset.image === 'png-ok' && document.getElementById('image-preview')?.complete && document.getElementById('image-preview')?.naturalWidth === 1 && document.getElementById('image-preview')?.clientWidth === 200 && document.getElementById('image-preview')?.clientHeight === 200";
const CAM_EMPTY: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-empty' && document.getElementById('image-history-status')?.dataset.state === 'empty'";

#[tokio::test]
async fn browser_image_visual_matrix_covers_themes_and_widths() {
    let base = lab().await;
    tokio::task::spawn_blocking(move || flow(&base))
        .await
        .unwrap();
}

fn flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    let failures = visual_matrix(&mut page);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn visual_matrix(page: &mut Browser) -> Vec<String> {
    let mut failures = Vec::new();
    for scheme in ["light", "dark"] {
        apply_scheme(page, scheme);
        page.viewport(1843, 900);
        page.show("#images");
        show_every_image(page);
        page.wait_until(READY);
        judge_focus(page, scheme, &mut failures);
        for width in [767, 768, 1843] {
            page.viewport(width, 900);
            show_populated(page);
            failures.extend(visual_case(page, scheme, width, "populated"));
            show_empty(page);
            failures.extend(visual_case(page, scheme, width, "empty"));
        }
    }
    failures
}

fn judge_focus(page: &mut Browser, scheme: &str, failures: &mut Vec<String>) {
    page.click_box("image-source-0");
    let mouse = page.computed("#image-source-0")["outlineStyle"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    let case = format!("{scheme} focus");
    judge(
        failures,
        &case,
        "focus",
        mouse == "none" && ring.contains("3px"),
        "mouse outline none and keyboard outline 3px",
        &format!("mouse {mouse} keyboard {ring}"),
    );
    page.wait_until(READY);
}

fn show_every_image(page: &mut Browser) {
    page.wait_until(
        "document.getElementById('image-range-start')?.value && document.getElementById('image-range-end')?.value",
    );
    page.eval("['image-range-start','image-range-end'].forEach((id) => { const el = document.getElementById(id); el.value = ''; el.dispatchEvent(new Event('input', {bubbles:true})); })");
}

fn show_populated(page: &mut Browser) {
    choose_source(page, "cam-preview");
    page.wait_until(PREVIEW_SOURCE);
    page.click("image-result-0");
    page.wait_until(PNG_READY);
}

fn show_empty(page: &mut Browser) {
    choose_source(page, "cam-empty");
    page.wait_until(CAM_EMPTY);
}

fn choose_source(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let clicked = page.eval(&format!(
        "(() => {{ const target = document.querySelector('#image-sources [data-source=' + {id} + ']'); if (!target) return false; const selected = () => [...document.querySelectorAll('#image-sources [aria-pressed=\"true\"]')].filter((el) => el.dataset.source); const all = document.getElementById('image-source-all'); if (all && all.getAttribute('aria-pressed') === 'true') {{ target.click(); return true; }} const id = target.dataset.source; if (!selected().some((el) => el.dataset.source === id)) target.click(); for (const el of selected()) if (el.dataset.source !== id) el.click(); return true; }})()"
    ));
    assert_eq!(clicked, Value::Bool(true), "missing source");
}

fn visual_case(page: &mut Browser, scheme: &str, width: u32, state: &str) -> Vec<String> {
    let report = page.eval(VISUAL);
    let case = format!("{scheme} {width}px {state}");
    let mut failures = Vec::new();
    judge_frame(&mut failures, &case, width, &report);
    judge_theme(&mut failures, &case, &report);
    if state == "populated" {
        judge_populated(&mut failures, &case, &report);
    } else {
        judge_empty(&mut failures, &case, &report);
    }
    failures
}

fn judge_frame(failures: &mut Vec<String>, case: &str, width: u32, report: &Value) {
    let titles = report["titles"].to_string();
    judge(
        failures,
        case,
        "title",
        report["titles"] == json!(["Images"]),
        "one visible route title Images",
        &titles,
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
        "source and history are visible",
        &format!(
            "heights {} {}",
            num(report, "filterHeight"),
            num(report, "resultHeight"),
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
            "15rem source rail beside history",
            &format!(
                "stacked {} row {} width {}",
                flag(report, "stacked"),
                flag(report, "row"),
                num(report, "rail")
            ),
        );
    }
    let right = num(report, "filterRight").max(num(report, "resultRight"));
    judge(
        failures,
        case,
        "clip",
        right <= num(report, "innerWidth") + 1.0 && num(report, "filterWidth") > 40.0,
        "parts stay inside the viewport",
        &format!("right {right} viewport {}", num(report, "innerWidth")),
    );
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
        flag(report, "inventory") && flag(report, "runner") && flag(report, "capture"),
        "hidden inventory and command runner with current-image capture",
        &format!(
            "inventory {} runner {} capture {}",
            flag(report, "inventory"),
            flag(report, "runner"),
            flag(report, "capture")
        ),
    );
}

fn judge_populated(failures: &mut Vec<String>, case: &str, report: &Value) {
    judge(
        failures,
        case,
        "preview",
        flag(report, "preview") && flag(report, "previewOk"),
        "bounded preview",
        &format!(
            "preview {} bounded {}",
            flag(report, "preview"),
            flag(report, "previewOk")
        ),
    );
    judge(
        failures,
        case,
        "metadata",
        readable(flag(report, "dark"), num(report, "metaLum")),
        "readable history text",
        &format!("lum {}", num(report, "metaLum")),
    );
    let status = text_value(report, "status");
    judge(
        failures,
        case,
        "copy",
        !status.contains("could not be read")
            && !status.contains("No images")
            && !status.contains("Loading")
            && text_value(report, "alert").is_empty(),
        "ready history without empty, loading, or error copy",
        &format!("status {status} alert {}", text_value(report, "alert")),
    );
}

fn judge_empty(failures: &mut Vec<String>, case: &str, report: &Value) {
    let status = text_value(report, "status");
    judge(
        failures,
        case,
        "copy",
        status == "No images for this source."
            && text_value(report, "alert").is_empty()
            && !flag(report, "preview"),
        "empty history without a preview, match, or error",
        &format!(
            "status {status} alert {} preview {}",
            text_value(report, "alert"),
            flag(report, "preview")
        ),
    );
    judge(
        failures,
        case,
        "metadata",
        text_value(report, "metaColor") != text_value(report, "metaBg")
            && !text_value(report, "metaColor").is_empty(),
        "empty-state text differs from its background",
        &format!(
            "{} on {}",
            text_value(report, "metaColor"),
            text_value(report, "metaBg")
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

fn readable(dark: bool, lum: f64) -> bool {
    if dark { lum > 180.0 } else { lum < 80.0 }
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
  const lum = (value) => {
    const oklch = String(value).match(/oklch\(\s*([0-9.]+)/);
    if (oklch) return Number(oklch[1]) * 255;
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = value;
    ctx.fillRect(0, 0, 1, 1);
    const data = ctx.getImageData(0, 0, 1, 1).data;
    return data[0] * 0.2126 + data[1] * 0.7152 + data[2] * 0.0722;
  };
  const box = (id) => {
    const el = document.getElementById(id);
    if (!el) return { width: 0, height: 0, top: 0, left: 0, right: 0 };
    const rect = el.getBoundingClientRect();
    return { width: rect.width, height: rect.height, top: rect.top, left: rect.left, right: rect.right };
  };
  const filters = box('image-filters');
  const results = box('image-results');
  const inspector = box('image-inspector');
  const preview = document.getElementById('image-preview');
  const title = document.querySelector('#image-history .log-card-title');
  const status = document.getElementById('image-history-status');
  const sample = title || status;
  const metaColor = sample ? getComputedStyle(sample).color : '';
  const metaBg = sample ? getComputedStyle(sample).backgroundColor : '';
  const titles = [...document.querySelectorAll('h1, h2')].filter((node) => {
    if (node.closest('[hidden]')) return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 2 && rect.height > 2;
  }).map((node) => node.textContent.trim()).filter((title) => ['Agent', 'Logs', 'Metrics', 'Tasks', 'Images', 'Chat'].includes(title));
  let previewOk = false;
  if (preview && preview.naturalWidth && preview.naturalHeight) {
    const parent = preview.parentElement.getBoundingClientRect();
    const rect = preview.getBoundingClientRect();
    const style = getComputedStyle(preview);
    const ratio = preview.clientWidth / preview.clientHeight;
    const natural = preview.naturalWidth / preview.naturalHeight;
    previewOk = style.objectFit === 'contain' && rect.width <= parent.width + 1 && rect.height <= window.innerHeight && Math.abs(ratio - natural) < 0.05;
  }
  const body = getComputedStyle(document.body);
  const browser = document.getElementById('image-browser');
  return {
    titles,
    rail: filters.width,
    stacked: getComputedStyle(browser).flexDirection === 'column',
    row: Math.abs(filters.top - results.top) < 4 && filters.right <= results.left + 2,
    overflow: document.documentElement.scrollWidth <= window.innerWidth + 1,
    filterWidth: filters.width,
    filterHeight: filters.height,
    resultWidth: results.width,
    resultHeight: results.height,
    inspectorWidth: inspector.width,
    inspectorHeight: inspector.height,
    filterRight: filters.right,
    resultRight: results.right,
    inspectorRight: inspector.right,
    innerWidth: window.innerWidth,
    bodyBg: body.backgroundColor,
    tokenBg: paint('--background'),
    bodyFg: body.color,
    tokenFg: paint('--foreground'),
    motion: getComputedStyle(document.documentElement).getPropertyValue('--motion-normal').trim(),
    wave: getComputedStyle(document.querySelector('.strand-wave')).animationName,
    preview: !!preview,
    previewOk,
    status: status ? status.textContent.trim() : '',
    alert: document.getElementById('image-history-alert')?.textContent.trim() || '',
    metaLum: lum(metaColor),
    metaColor,
    metaBg: metaBg === 'rgba(0, 0, 0, 0)' ? getComputedStyle(document.getElementById('image-modal') || document.body).backgroundColor : metaBg,
    dark: document.documentElement.classList.contains('dark'),
    inventory: document.getElementById('inventory-images')?.hasAttribute('hidden') === true,
    runner: document.getElementById('workbench')?.hasAttribute('hidden') === true,
    capture: !!document.getElementById('image-current') && !!document.getElementById('image-current-source') && document.getElementById('image-current-actions')?.parentElement?.id === 'image-view' && document.getElementById('image-current-actions')?.nextElementSibling?.id === 'image-browser' && !document.querySelector('#image-filters .log-actions') && !document.getElementById('image-refresh') && !document.getElementById('image-clear') && document.querySelector('#image-filters > .log-filter-group') === document.getElementById('image-sources') && [...(document.getElementById('image-sources')?.querySelectorAll('.log-facet') || [])].every((el) => el.getAttribute('aria-pressed') === 'true' || el.getAttribute('aria-pressed') === 'false') && (document.getElementById('image-sources')?.querySelectorAll('.log-facet').length || 0) > 0
  };
})()";

async fn lab() -> String {
    let images = MemoryImages::new();
    insert_source(&images, "cam-a", "Camera A").await;
    insert_source(&images, "cam-empty", "Empty").await;
    insert_source(&images, "cam-preview", "Preview").await;
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1)
        .unwrap();
    insert_frame(
        &images,
        "frame-000",
        "cam-a",
        "2024-01-01T00:00:00Z",
        "image/png",
        Some("plate well"),
        png.clone(),
    )
    .await;
    insert_frame(
        &images,
        "png-ok",
        "cam-preview",
        "2024-03-01T00:00:00Z",
        "image/png",
        Some("visible frame"),
        png,
    )
    .await;
    serve(images).await
}

async fn serve(images: MemoryImages) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn insert_source(images: &MemoryImages, id: &str, name: &str) {
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new(id).unwrap(),
            name: name.to_owned(),
            description: format!("{name} frame"),
            asset_id: None,
            semantic_id: None,
        })
        .await
        .unwrap();
}

async fn insert_frame(
    images: &MemoryImages,
    id: &str,
    source: &str,
    at: &str,
    media: &str,
    caption: Option<&str>,
    bytes: Vec<u8>,
) {
    let descriptor = ImageDescriptor::new(
        ImageId::new(id).unwrap(),
        ImageSourceId::new(source).unwrap(),
        UtcTimestamp::parse(at).unwrap(),
        media,
        1,
        1,
        caption.map(str::to_owned),
        JsonObject::empty(),
    )
    .unwrap();
    images
        .insert_image(Image::new(descriptor, bytes).unwrap())
        .await
        .unwrap();
}
