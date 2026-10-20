//! Closeout checks for the Images history reader and preview inspector.

#![cfg(feature = "web-ui")]

use std::collections::HashMap;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, Image, ImageDescriptor, ImageId, ImageSource,
    ImageSourceId, JsonObject, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks,
    SecurityScheme, UtcTimestamp, WEB_UI_PATH, bind_local,
};
use a2a_types::SecurityRequirement;
use base64::Engine;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{Browser, header};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const TOKEN: &str = "image-bearer-token";
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const MARKUP: &str = r#"<svg id="image-xss" onload="window.__imageXss=1"></svg>"#;
const BAD_BYTES: &[u8] = b"closeout-bad-b64";
const CAM_A: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 100 && document.getElementById('image-result-0')?.dataset.image === 'frame-000'";
const CAPTURE_READY: &str = "!!document.getElementById('image-current') && !document.getElementById('image-current').disabled && document.getElementById('image-current-source')?.value === 'cam-a' && document.getElementById('image-current-actions')?.parentElement?.id === 'image-view' && document.getElementById('image-current-actions')?.nextElementSibling?.id === 'image-browser' && !document.querySelector('#image-filters .log-actions')";
const CAM_EMPTY: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-empty' && document.getElementById('image-history-status')?.dataset.state === 'empty'";
const PREVIEW_SOURCE: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-preview' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'png-ok'";
const PNG_READY: &str = "document.getElementById('image-modal')?.dataset.state === 'selected-ready' && document.getElementById('image-modal')?.dataset.image === 'png-ok' && document.getElementById('image-preview')?.complete && document.getElementById('image-preview')?.naturalWidth === 1 && document.getElementById('image-preview')?.clientWidth === 200 && document.getElementById('image-preview')?.clientHeight === 200 && document.getElementById('image-size')?.textContent === '1 × 1' && !document.getElementById('image-facts')";
const SIZE_CORNER: &str = r"( () => {
  const img = document.getElementById('image-preview')?.getBoundingClientRect();
  const size = document.getElementById('image-size')?.getBoundingClientRect();
  if (!img || !size) return false;
  return Math.abs(size.left - img.left) < 1 && Math.abs(size.bottom - img.bottom) < 1;
})()";
const LIMIT_READY: &str = "document.getElementById('image-modal')?.dataset.image === 'png-limit' && document.getElementById('image-preview')?.complete && document.getElementById('image-preview')?.naturalWidth === 1";
const OVER_READY: &str = "document.getElementById('image-modal')?.dataset.image === 'png-over' && !document.getElementById('image-preview')";
const SVG_READY: &str = "document.getElementById('image-modal')?.dataset.image === 'svg-mark' && !document.getElementById('image-preview') && !document.querySelector('#image-modal svg')";
const BAD_READY: &str = "document.getElementById('image-modal')?.dataset.image === 'bad-b64' && !document.getElementById('image-preview')";
const SELECTED_LOADING: &str =
    "document.getElementById('image-modal')?.dataset.state === 'selected'";

#[tokio::test]
async fn browser_image_reader_closes_the_usability_matrix() {
    let (base, images) = matrix_lab().await;
    let empty = serve(MemoryImages::new(), false).await;
    let secured = secured_lab().await;
    let policy = shell_policy(&base).await;
    assert!(policy.contains("default-src 'none'"), "{policy}");
    assert!(policy.contains("img-src 'self' data:"), "{policy}");
    assert!(
        policy.contains("script-src 'self' 'wasm-unsafe-eval'"),
        "{policy}"
    );
    assert!(!policy.contains("unsafe-inline"), "{policy}");
    let handle = tokio::runtime::Handle::current();
    let bad = bad_b64();
    tokio::task::spawn_blocking(move || flow(&base, &empty, &secured, &images, &bad, &handle))
        .await
        .unwrap();
}

fn flow(
    base: &str,
    empty: &str,
    secured: &str,
    images: &MemoryImages,
    bad: &str,
    handle: &tokio::runtime::Handle,
) {
    let mut page = Browser::open("about:blank");
    let _script = page.prelude(&prelude(bad));
    page.goto(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 900);
    page.show("#images");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'loading' && document.getElementById('image-history-status')?.textContent.includes('Loading images.') && document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a'",
    );
    assert_loading(&mut page);
    show_every_image(&mut page);
    page.eval("window.__releaseImage('first-list')");
    page.wait_until(CAM_A);
    assert_desktop(&mut page);
    assert_queries(&mut page);
    assert_current_capture(&mut page);
    assert_keyboard(&mut page);
    page.click("image-more");
    page.wait_until(
        "document.querySelectorAll('#image-history [data-id]').length === 101 && document.querySelector('#image-history [data-id]:last-child')?.dataset.image === 'frame-100'",
    );
    assert_eq!(page.eval(UNIQUE_IDS), Value::Bool(true));
    assert_eq!(
        page.eval("document.getElementById('image-count')"),
        Value::Null
    );
    assert_focus_survives(&mut page);
    assert_stale_list(&mut page);
    assert_preview(&mut page);
    assert_empty_source(&mut page);
    assert_failures(&mut page, images, handle);
    assert_transport(&mut page);
    assert_console(&mut page, "matrix");
    page.goto(&format!("{empty}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#images");
    page.wait_until(
        "document.getElementById('image-source-status')?.dataset.state === 'sources' && document.getElementById('image-source-status')?.textContent.includes('No image sources.') && document.querySelector('#image-sources .log-facet')?.disabled === true && document.querySelector('#image-sources .log-facet')?.textContent.includes('None loaded')",
    );
    assert_no_sources(&mut page);
    drop(page);
    assert_bearer(secured);
}

fn assert_loading(page: &mut Browser) {
    assert_eq!(attr(page, "image-history-status", "role"), "status");
    assert_eq!(attr(page, "image-history-status", "aria-live"), "polite");
    assert_eq!(attr(page, "image-history-alert", "role"), "alert");
    assert_eq!(text(page, "image-history-alert"), "");
    assert!(!text(page, "image-history-status").contains("No images"));
    assert!(!text(page, "image-history-status").contains("could not be read"));
    assert_eq!(page.eval(CAPTURE_READY), Value::Bool(true));
    assert_eq!(source_value(page), "cam-a");
    assert_eq!(text(page, "image-source-status"), "");
    assert_hidden_shell(page);
}

fn assert_desktop(page: &mut Browser) {
    let report = page.eval(DESKTOP);
    assert_eq!(report["pass"], Value::Bool(true), "{report}");
    page.click_box("image-source-0");
    let mouse = page.computed("#image-source-0")["outlineStyle"].clone();
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(mouse, json!("none"), "mouse outline {mouse}");
    assert!(ring.contains("3px"), "keyboard outline {ring}");
    page.wait_until(CAM_A);
}

fn assert_queries(page: &mut Browser) {
    let searches = sends(page, "search_images");
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 1 && document.getElementById('image-result-0')?.dataset.image === 'frame-000'",
    );
    let body = last_send(page, "search_images");
    assert!(body.contains("cam-a") && body.contains("plate"), "{body}");
    assert!(sends(page, "search_images") > searches);
    clear_criteria(page);
    page.wait_until(CAM_A);
    type_input(page, "image-range-start", "2024-01-01T00:00");
    type_input(page, "image-range-end", "2024-01-01T00:02");
    page.wait_until(
        "document.querySelectorAll('#image-history [data-id]').length === 2 && document.getElementById('image-result-0')?.dataset.image === 'frame-000' && document.getElementById('image-result-1')?.dataset.image === 'frame-001'",
    );
    clear_criteria(page);
    page.wait_until(CAM_A);
    type_input(page, "image-search", "no-such-image");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'nomatch' && document.getElementById('image-history-status')?.textContent.includes('No images match this search.')",
    );
    assert_eq!(text(page, "image-history-alert"), "");
    assert!(!text(page, "image-history-status").contains("No images for this source."));
    assert_eq!(source_value(page), "cam-a");
    clear_criteria(page);
    page.wait_until(CAM_A);
    assert_invalid(page);
}

fn assert_current_capture(page: &mut Browser) {
    let captures = sends(page, "get_current_image");
    let lists = sends(page, "list_images");
    page.eval("(() => { const el = document.getElementById('image-current-source'); el.value = 'cam-late'; el.dispatchEvent(new Event('change', {bubbles:true})); })()");
    page.wait_until("document.getElementById('image-current-source')?.value === 'cam-late'");
    page.eval("window.__holdCurrent = true");
    page.click("image-current");
    page.wait_until(
        "document.getElementById('image-current')?.disabled && document.getElementById('image-current-status')?.textContent.includes('Capturing current image.') && window.__imagePending() > 0",
    );
    assert_eq!(attr(page, "image-current-status", "role"), "status");
    assert_eq!(attr(page, "image-current-status", "aria-live"), "polite");
    assert_eq!(attr(page, "image-current-alert", "role"), "alert");
    assert_eq!(attr(page, "image-current-alert", "aria-live"), "assertive");
    page.eval("window.__releaseImage('paused-current')");
    page.wait_until(
        "document.getElementById('image-current-status')?.textContent === '' && !document.getElementById('image-current')?.disabled && document.getElementById('image-modal')?.dataset.origin === 'current' && document.getElementById('image-modal')?.dataset.image === 'late-only' && document.getElementById('image-preview')?.alt === 'late frame' && document.activeElement?.id === 'image-modal' && document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-history-status')?.dataset.state === 'ready'",
    );
    let body = last_send(page, "get_current_image");
    assert!(body.contains("cam-late"), "{body}");
    assert!(sends(page, "get_current_image") > captures);
    assert!(sends(page, "list_images") > lists);
    assert!(last_send(page, "list_images").contains("cam-a"));
    assert_eq!(text(page, "image-current-status"), "");
    assert_eq!(text(page, "image-current-alert"), "");
    assert_eq!(source_value(page), "cam-a");
    page.click("image-modal");
    page.wait_until("!document.getElementById('image-modal')");
    page.eval("(() => { const el = document.getElementById('image-current-source'); el.value = 'cam-preview'; el.dispatchEvent(new Event('change', {bubbles:true})); })()");
    page.wait_until("document.getElementById('image-current-source')?.value === 'cam-preview'");
}

fn assert_invalid(page: &mut Browser) {
    let searches = sends(page, "search_images");
    let lists = sends(page, "list_images");
    type_input(page, "image-search", "   ");
    page.wait_until(
        "document.getElementById('image-history-alert')?.textContent.includes('Enter a search, or clear the field.')",
    );
    assert_eq!(sends(page, "search_images"), searches);
    assert_eq!(sends(page, "list_images"), lists);
    assert!(text(page, "image-history").contains("frame-000"));
    assert!(!text(page, "image-history-status").contains("Enter a search"));
    clear_criteria(page);
    page.wait_until(CAM_A);
    type_input(page, "image-range-start", "2024-01-02T00:00");
    page.wait_until(
        "document.getElementById('image-history-alert')?.textContent.includes('Enter a start and an end.')",
    );
    assert_eq!(sends(page, "search_images"), searches);
    clear_criteria(page);
    page.wait_until(CAM_A);
    type_input(page, "image-range-start", "2024-01-02T00:00");
    type_input(page, "image-range-end", "2024-01-01T00:00");
    page.wait_until(
        "document.getElementById('image-history-alert')?.textContent.includes('The start must be before the end.')",
    );
    assert_eq!(sends(page, "search_images"), searches);
    assert_eq!(attr(page, "image-history-alert", "role"), "alert");
    assert_eq!(attr(page, "image-history-alert", "aria-live"), "assertive");
    clear_criteria(page);
    page.wait_until(CAM_A);
}

fn assert_keyboard(page: &mut Browser) {
    assert_eq!(page.eval(RESULT_STOPS), json!(["image-result-0"]));
    let mut expected = facet_ids(page);
    expected.extend(
        [
            "image-search",
            "image-range-start",
            "image-range-end",
            "image-result-0",
            "image-more",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let refs = expected.iter().map(String::as_str).collect::<Vec<_>>();
    let order = tab_controls(page, refs[0], &refs);
    assert_eq!(order, expected);
    choose_source(page, "cam-empty");
    page.wait_until(CAM_EMPTY);
    assert_eq!(source_value(page), "cam-empty");
    choose_source(page, "cam-a");
    page.wait_until(CAM_A);
    press_result(page, "ArrowDown");
    page.wait_until(
        "document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true'",
    );
    press_result(page, "ArrowDown");
    page.wait_until(
        "document.getElementById('image-result-1')?.getAttribute('aria-checked') === 'true' && document.getElementById('image-result-1')?.dataset.image === 'frame-001'",
    );
    assert_eq!(page.eval(RESULT_STOPS), json!(["image-result-1"]));
    press_result(page, "ArrowUp");
    page.wait_until(
        "document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true'",
    );
}

fn assert_focus_survives(page: &mut Browser) {
    page.wait_until("document.getElementById('image-modal')?.dataset.image === 'frame-000'");
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'frame-000'",
    );
    page.focus("image-result-0");
    page.eval("window.__holdNext = true");
    clear_criteria(page);
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'loading' && document.getElementById('image-modal')?.dataset.image === 'frame-000'",
    );
    assert_focus_valid(page, "history refresh");
    page.eval("window.__releaseImage('paused-list')");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true' && document.getElementById('image-result-0')?.dataset.image === 'frame-000'",
    );
    assert_focus_valid(page, "history ready");
    assert_eq!(page.eval(RESULT_STOPS), json!(["image-result-0"]));
    page.focus("image-result-0");
    type_input(page, "image-search", "no-such-image");
    page.wait_until("document.getElementById('image-history-status')?.dataset.state === 'nomatch'");
    assert_focus_valid(page, "results disappeared");
    assert_eq!(source_value(page), "cam-a");
    clear_criteria(page);
    page.wait_until(CAM_A);
}

fn assert_stale_list(page: &mut Browser) {
    choose_source(page, "cam-late");
    page.wait_until(
        "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-late' && document.getElementById('image-history-status')?.dataset.state === 'loading' && window.__imagePending() > 0",
    );
    choose_source(page, "cam-a");
    page.wait_until(CAM_A);
    let lists = sends(page, "list_images");
    page.eval("window.__releaseImage('cam-late')");
    wait_for_send(page, "list_images", lists);
    page.eval("new Promise((resolve) => setTimeout(resolve, 300))");
    assert_eq!(page.eval(CAM_A_EXPR), Value::Bool(true));
    assert!(!text(page, "image-history").contains("late-only"));
}

fn assert_preview(page: &mut Browser) {
    choose_source(page, "cam-preview");
    page.wait_until(PREVIEW_SOURCE);
    assert_named_preview(page);
    assert_preview_boundary(page);
    assert_withheld_media(page);
    assert_stale_frame(page);
    assert_private_storage(page);
}

fn assert_named_preview(page: &mut Browser) {
    page.eval("window.__imageXss = 0");
    page.click("image-result-0");
    page.wait_until(PNG_READY);
    assert_preview_facts(page);
    assert_accessible_names(page);
}

fn assert_preview_facts(page: &mut Browser) {
    assert_eq!(attr(page, "image-modal", "data-image"), "png-ok");
    assert_eq!(attr(page, "image-preview", "alt"), "visible frame");
    assert_eq!(text(page, "image-size"), "1 × 1");
    assert_eq!(page.eval(SIZE_CORNER), Value::Bool(true), "size corner");
    let src = attr(page, "image-preview", "src");
    assert!(src.starts_with("data:image/png;base64,"), "{src}");
    assert_eq!(
        page.eval("document.getElementById('image-facts')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('image-origin')"),
        Value::Null
    );
}

fn assert_accessible_names(page: &mut Browser) {
    let names = page.eval(NAMES);
    assert_eq!(names["sources"], json!("Source"));
    assert_eq!(names["history"], json!("Image history"));
    assert_eq!(names["filters"], json!("Image filters"));
    assert_eq!(names["search"], json!("Search"));
    assert_eq!(names["start"], json!("Start"));
    assert_eq!(names["end"], json!("End"));
    assert_eq!(names["inspector"], json!("Image preview"));
    let result = names["result"].as_str().unwrap_or("");
    assert!(result.contains("png-ok"), "{result}");
    assert!(result.contains("visible frame"), "{result}");
}

fn assert_preview_boundary(page: &mut Browser) {
    page.click("image-result-1");
    page.wait_until(LIMIT_READY);
    page.click("image-result-2");
    page.wait_until(OVER_READY);
    assert_eq!(
        page.eval("document.getElementById('image-preview')"),
        Value::Null
    );
}

fn assert_withheld_media(page: &mut Browser) {
    page.click("image-result-3");
    page.wait_until(SVG_READY);
    assert_eq!(
        page.eval("document.getElementById('image-xss')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.querySelector('#image-inspector svg')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__imageXss"), json!(0));
    page.click("image-result-4");
    page.wait_until(BAD_READY);
    assert_eq!(attr(page, "image-modal", "data-image"), "bad-b64");
    assert_eq!(
        page.eval("document.getElementById('image-preview')"),
        Value::Null
    );
}

fn assert_stale_frame(page: &mut Browser) {
    page.click("image-result-5");
    page.wait_until(&format!(
        "{SELECTED_LOADING} && window.__imagePending() > 0"
    ));
    assert!(text(page, "image-history").contains("png-ok"));
    page.click("image-result-0");
    page.wait_until(PNG_READY);
    let reads = sends(page, "get_image");
    page.eval("window.__releaseImage('frame-late')");
    wait_for_send(page, "get_image", reads);
    page.eval("new Promise((resolve) => setTimeout(resolve, 300))");
    assert_eq!(attr(page, "image-modal", "data-image"), "png-ok");
}

fn assert_private_storage(page: &mut Browser) {
    let stored = storage_blob(page);
    assert!(!stored.contains("iVBORw0KGgo"), "{stored}");
    assert!(!stored.contains("png-ok"), "{stored}");
    assert!(!stored.contains("visible frame"), "{stored}");
    assert_eq!(page.eval("JSON.stringify(sessionStorage)"), json!("{}"));
    assert_eq!(page.eval("document.cookie"), json!(""));
}

fn assert_empty_source(page: &mut Browser) {
    choose_source(page, "cam-empty");
    page.wait_until(CAM_EMPTY);
    assert_eq!(
        text(page, "image-history-status"),
        "No images for this source."
    );
    assert_eq!(text(page, "image-history-alert"), "");
    assert_eq!(attr(page, "image-history-status", "role"), "status");
    assert!(!text(page, "image-history-status").contains("match"));
    assert!(!text(page, "image-history-status").contains("could not be read"));
}

fn assert_failures(page: &mut Browser, images: &MemoryImages, handle: &tokio::runtime::Handle) {
    choose_source(page, "cam-preview");
    page.wait_until(PREVIEW_SOURCE);
    page.click("image-result-0");
    page.wait_until(PNG_READY);
    handle.block_on(images.set_unavailable("history offline"));
    page.click("image-current");
    page.wait_until(
        "document.getElementById('image-current-status')?.textContent.includes('Current image could not be captured.') && document.getElementById('image-current-alert')?.textContent.includes('history offline')",
    );
    assert!(text(page, "image-history").contains("png-ok"));
    type_input(page, "image-search", "visible");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'error' && document.getElementById('image-history-alert')?.textContent.includes('history offline') && document.getElementById('image-modal')?.dataset.image === 'png-ok'",
    );
    assert!(text(page, "image-history-status").contains("Images could not be read."));
    assert_ne!(
        text(page, "image-history-status"),
        text(page, "image-history-alert")
    );
    assert!(!text(page, "image-source-status").contains("could not be read"));
    handle.block_on(images.clear_unavailable());
    clear_criteria(page);
    page.wait_until(PREVIEW_SOURCE);
    handle.block_on(images.set_unavailable("selected offline"));
    page.click("image-result-1");
    page.wait_until(
        "document.getElementById('image-modal')?.dataset.state === 'selected-error' && !document.getElementById('image-preview')",
    );
    assert!(text(page, "image-history").contains("png-ok"));
    assert_eq!(source_value(page), "cam-preview");
    handle.block_on(images.clear_unavailable());
    page.click("image-result-1");
    page.wait_until(LIMIT_READY);
}

fn assert_transport(page: &mut Browser) {
    page.fail_network(true);
    type_input(page, "image-search", "visible");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'error' && document.getElementById('image-history-alert')?.textContent.includes('could not reach this origin')",
    );
    assert!(text(page, "image-history-status").contains("Images could not be read."));
    assert_eq!(source_value(page), "cam-preview");
    page.fail_network(false);
    clear_criteria(page);
    page.wait_until(PREVIEW_SOURCE);
}

fn assert_no_sources(page: &mut Browser) {
    assert_eq!(text(page, "image-source-status"), "No image sources.");
    assert_eq!(attr(page, "image-source-status", "role"), "status");
    assert_eq!(attr(page, "image-source-alert", "role"), "alert");
    assert_eq!(text(page, "image-source-alert"), "");
    assert!(!text(page, "image-history-status").contains("could not be read"));
    assert_eq!(
        page.eval("document.getElementById('image-current')?.disabled && document.getElementById('image-current-source')?.disabled"),
        Value::Bool(true)
    );
    assert_hidden_shell(page);
}

fn assert_bearer(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.wait_until("document.getElementById('bearer-token')");
    page.set_value("bearer-token", TOKEN);
    page.show("#images");
    show_every_image(&mut page);
    page.wait_until(
        "document.getElementById('image-result-0')?.textContent.includes('[redacted]') && document.getElementById('image-history-status')?.dataset.state === 'ready'",
    );
    page.click("image-result-0");
    page.wait_until(
        "document.getElementById('image-preview')?.alt === '[redacted]' && document.getElementById('image-modal')?.dataset.image === 'secret-frame'",
    );
    let view = text(&mut page, "view-images");
    assert!(!view.contains(TOKEN), "{view}");
    assert!(!storage_blob(&mut page).contains(TOKEN));
    assert!(!storage_blob(&mut page).contains("secret-frame"));
    assert!(!storage_blob(&mut page).contains("iVBORw0KGgo"));
    assert_eq!(page.eval("JSON.stringify(sessionStorage)"), json!("{}"));
    assert_eq!(page.eval("document.cookie"), json!(""));
    let sent = page
        .requests(base)
        .into_iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .any(|request| {
            header(&request, "authorization").is_some_and(|value| value.contains(TOKEN))
        });
    assert!(sent, "bearer token was not sent on image reads");
    let console = page.console_text();
    assert!(!console.contains(TOKEN), "{console}");
    assert_console(&mut page, "bearer");
}

fn assert_hidden_shell(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('inventory-images')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    let body = visible_text(page, "view-images");
    assert!(!body.contains("More image sources"), "{body}");
    assert!(!body.contains("Command"), "{body}");
}

fn visible_text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.innerText.trim() ?? ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn assert_focus_valid(page: &mut Browser, case: &str) {
    let report = page.eval(FOCUS);
    assert_eq!(report["connected"], Value::Bool(true), "{case} {report}");
    assert_eq!(report["hidden"], Value::Bool(false), "{case} {report}");
}

fn assert_console(page: &mut Browser, case: &str) {
    let console = page.console_text();
    assert!(
        !console.contains("Content Security Policy"),
        "{case} {console}"
    );
    assert!(!console.contains("EvalError"), "{case} {console}");
    assert!(!console.contains("Refused to"), "{case} {console}");
}

fn tab_controls(page: &mut Browser, start: &str, expected: &[&str]) -> Vec<String> {
    page.focus(start);
    let mut ids = vec![active_id(page)];
    for _ in 0..24 {
        if ids.len() == expected.len() {
            break;
        }
        page.press_tab();
        let id = active_id(page);
        if ids.last() != Some(&id) {
            ids.push(id);
        }
    }
    ids
}

fn wait_for_send(page: &mut Browser, operation: &str, before: usize) {
    let start = std::time::Instant::now();
    while sends(page, operation) <= before && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(sends(page, operation) > before, "{operation} was not sent");
}

fn facet_ids(page: &mut Browser) -> Vec<String> {
    page.eval(
        "(() => [...document.querySelectorAll('#image-sources .log-facet')].map((el) => el.id))()",
    )
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|item| item.as_str().map(str::to_owned))
    .collect()
}

fn choose_source(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let clicked = page.eval(&format!(
        "(() => {{ const target = document.querySelector('#image-sources [data-source=' + {id} + ']'); if (!target) return false; const selected = () => [...document.querySelectorAll('#image-sources [aria-pressed=\"true\"]')].filter((el) => el.dataset.source); const all = document.getElementById('image-source-all'); if (all && all.getAttribute('aria-pressed') === 'true') {{ target.click(); return true; }} const id = target.dataset.source; if (!selected().some((el) => el.dataset.source === id)) target.click(); for (const el of selected()) if (el.dataset.source !== id) el.click(); return true; }})()"
    ));
    assert_eq!(clicked, Value::Bool(true), "missing source");
}

fn source_value(page: &mut Browser) -> String {
    page.eval("document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn press_result(page: &mut Browser, key: &str) {
    press_group(
        page,
        "#image-history [aria-checked=\"true\"], #image-history [tabindex=\"0\"]",
        key,
    );
}

fn press_group(page: &mut Browser, selector: &str, key: &str) {
    let selector = serde_json::to_string(selector).unwrap_or_default();
    let key = serde_json::to_string(key).unwrap_or_default();
    let moved = page.eval(&format!(
        "(() => {{ const current = document.querySelector({selector}); if (!current) return false; current.focus(); current.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}})); return true; }})()"
    ));
    assert_eq!(moved, Value::Bool(true), "{selector} {key}");
}

fn show_every_image(page: &mut Browser) {
    page.wait_until(
        "document.getElementById('image-range-start')?.value && document.getElementById('image-range-end')?.value",
    );
    type_input(page, "image-range-start", "");
    type_input(page, "image-range-end", "");
}

fn clear_criteria(page: &mut Browser) {
    page.eval("['image-search','image-range-start','image-range-end'].forEach((id) => { const el = document.getElementById(id); el.value = ''; el.dispatchEvent(new Event('input', {bubbles:true})); })");
}

fn type_input(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('input', {{bubbles:true}}))"
    ));
}

fn sends(page: &mut Browser, operation: &str) -> usize {
    let needle = serde_json::to_string(&format!("\"{operation}\"")).unwrap_or_default();
    usize::try_from(
        page.eval(&format!(
            "(window.__imageSends || []).filter((body) => body.includes({needle})).length"
        ))
        .as_u64()
        .unwrap_or(0),
    )
    .unwrap_or(0)
}

fn last_send(page: &mut Browser, operation: &str) -> String {
    let needle = serde_json::to_string(&format!("\"{operation}\"")).unwrap_or_default();
    page.eval(&format!(
        "(window.__imageSends || []).findLast((body) => body.includes({needle})) || ''"
    ))
    .as_str()
    .unwrap_or("")
    .to_owned()
}

fn request_url(request: &Value) -> &str {
    request["params"]["request"]["url"].as_str().unwrap_or("")
}

fn text(page: &mut Browser, id: &str) -> String {
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id})?.textContent.trim() ?? ''"
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

fn active_id(page: &mut Browser) -> String {
    page.eval("document.activeElement?.id ?? ''")
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

fn prelude(bad: &str) -> String {
    let bad = serde_json::to_string(bad).unwrap_or_default();
    format!(
        r#"( () => {{
  const orig = window.fetch.bind(window);
  const pending = [];
  const fresh = window.name !== 'image-closeout';
  window.name = 'image-closeout';
  let listed = 0;
  window.__imageSends = [];
  const BAD = {bad};
  window.__imagePending = () => pending.length;
  window.__releaseImage = (needle) => {{
    const ready = [];
    const rest = [];
    for (const item of pending) {{
      if (!needle || item.body.includes(needle)) ready.push(item);
      else rest.push(item);
    }}
    pending.length = 0;
    pending.push(...rest);
    for (const item of ready) item.resolve();
  }};
  window.fetch = async (input, init) => {{
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    let body = '';
    if (String(url).includes('/message:send')) {{
      try {{
        const copy = input instanceof Request ? input.clone() : new Request(input, init);
        body = await copy.text();
      }} catch (error) {{}}
    }}
    const history = body.includes('"list_images"') || body.includes('"search_images"');
    const first = fresh && listed === 0 && history;
    if (history) listed += 1;
    const late = body.includes('cam-late') || body.includes('frame-late');
    const paused = window.__holdNext === true && body.includes('"list_images"');
    const current = window.__holdCurrent === true && body.includes('"get_current_image"');
    if (paused) window.__holdNext = false;
    if (current) window.__holdCurrent = false;
    let marker = '';
    if (first) marker = 'first-list';
    else if (paused) marker = 'paused-list';
    else if (current) marker = 'paused-current';
    else if (late) marker = body;
    if (marker) await new Promise((resolve) => pending.push({{ body: marker, resolve }}));
    if (body) window.__imageSends.push(body);
    const response = await orig(input, init);
    if (!body.includes('bad-b64')) return response;
    const text = await response.text();
    const headers = new Headers(response.headers);
    headers.delete('content-length');
    return new Response(text.split(BAD).join('****'), {{ status: response.status, statusText: response.statusText, headers }});
  }};
}})();"#
    )
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

async fn matrix_lab() -> (String, MemoryImages) {
    let images = MemoryImages::new();
    insert_source(&images, "cam-a", "Camera A").await;
    insert_source(&images, "cam-empty", "Empty").await;
    insert_source(&images, "cam-late", "Late").await;
    insert_source(&images, "cam-preview", "Preview").await;
    for index in 0..101 {
        let caption = (index == 0).then_some("plate well");
        insert_frame(
            &images,
            &format!("frame-{index:03}"),
            "cam-a",
            &stamp(index),
            "image/png",
            caption,
            vec![1, 2, 3, 4],
        )
        .await;
    }
    insert_frame(
        &images,
        "late-only",
        "cam-late",
        "2024-02-01T00:00:00Z",
        "image/png",
        Some("late frame"),
        vec![1, 2, 3, 4],
    )
    .await;
    seed_preview(&images).await;
    images
        .set_current(
            ImageSourceId::new("cam-preview").unwrap(),
            ImageId::new("live-now").unwrap(),
        )
        .await
        .unwrap();
    (serve(images.clone(), false).await, images)
}

async fn seed_preview(images: &MemoryImages) {
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1)
        .unwrap();
    let limit = pad_png(262_144);
    let mut over = limit.clone();
    over.push(0);
    insert_frame(
        images,
        "png-ok",
        "cam-preview",
        "2024-03-01T00:00:00Z",
        "image/png",
        Some("visible frame"),
        png.clone(),
    )
    .await;
    insert_frame(
        images,
        "png-limit",
        "cam-preview",
        "2024-03-01T00:01:00Z",
        "image/png",
        None,
        limit,
    )
    .await;
    insert_frame(
        images,
        "png-over",
        "cam-preview",
        "2024-03-01T00:02:00Z",
        "image/png",
        None,
        over,
    )
    .await;
    insert_frame(
        images,
        "svg-mark",
        "cam-preview",
        "2024-03-01T00:03:00Z",
        "image/svg+xml",
        Some(MARKUP),
        MARKUP.as_bytes().to_vec(),
    )
    .await;
    insert_frame(
        images,
        "bad-b64",
        "cam-preview",
        "2024-03-01T00:04:00Z",
        "image/gif",
        None,
        BAD_BYTES.to_vec(),
    )
    .await;
    insert_frame(
        images,
        "frame-late",
        "cam-preview",
        "2024-03-01T00:05:00Z",
        "image/png",
        Some("late selection"),
        png.clone(),
    )
    .await;
    insert_frame(
        images,
        "live-now",
        "cam-preview",
        "2024-03-01T00:06:00Z",
        "image/png",
        Some("live frame"),
        png,
    )
    .await;
}

async fn secured_lab() -> String {
    let images = MemoryImages::new();
    insert_source(&images, "cam-secret", "Secret").await;
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1)
        .unwrap();
    insert_frame(
        &images,
        "secret-frame",
        "cam-secret",
        "2024-04-01T00:00:00Z",
        "image/png",
        Some(TOKEN),
        png,
    )
    .await;
    serve(images, true).await
}

async fn serve(images: MemoryImages, secured: bool) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service).with_web_ui();
    if secured {
        let schemes = HashMap::from([(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]);
        let requirements: Vec<SecurityRequirement> =
            vec![[("bearerAuth".to_owned(), Vec::new())].into()];
        server = server.with_security(schemes, requirements);
    }
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

fn stamp(index: u32) -> String {
    let hour = index / 60;
    let minute = index % 60;
    format!("2024-01-01T{hour:02}:{minute:02}:00Z")
}

fn bad_b64() -> String {
    base64::engine::general_purpose::STANDARD.encode(BAD_BYTES)
}

fn pad_png(target: usize) -> Vec<u8> {
    assert_eq!(png_crc(b"123456789"), 0xCBF4_3926);
    let mut png = base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1)
        .unwrap();
    let iend = png.windows(4).position(|window| window == b"IEND").unwrap() - 4;
    let extra = target - png.len();
    let data_len = extra - 12;
    let mut chunk = Vec::with_capacity(extra);
    chunk.extend_from_slice(&u32::try_from(data_len).unwrap().to_be_bytes());
    chunk.extend_from_slice(b"coTx");
    chunk.extend(vec![b'.'; data_len]);
    let crc = png_crc(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    png.splice(iend..iend, chunk);
    assert_eq!(png.len(), target);
    png
}

fn png_crc(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

const CAM_A_EXPR: &str = "!!(document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-result-0')?.dataset.image === 'frame-000' && !document.getElementById('image-history')?.innerText.includes('late-only'))";
const RESULT_STOPS: &str = "(() => [...document.querySelectorAll('#image-history [role=\"radio\"]')].filter((el) => el.tabIndex === 0).map((el) => el.id))()";
const UNIQUE_IDS: &str = "(() => { const ids = [...document.querySelectorAll('#image-history [data-id]')].map((el) => el.dataset.id); return new Set(ids).size === ids.length && ids[0] === 'frame-000' && ids.at(-1) === 'frame-100'; })()";
const FOCUS: &str = "(() => { const active = document.activeElement; return { id: active?.id || active?.tagName || '', connected: !!active && document.contains(active), hidden: !!active?.closest('[hidden]') }; })()";
const NAMES: &str = r#"(() => ({
  sources: document.querySelector('#image-sources > p')?.textContent.trim() || '',
  history: document.getElementById('image-history')?.getAttribute('aria-label') || '',
  filters: document.getElementById('image-filters')?.getAttribute('aria-label') || '',
  search: document.querySelector('label[for="image-search"]')?.textContent.trim() || '',
  start: document.querySelector('label[for="image-range-start"]')?.textContent.trim() || '',
  end: document.querySelector('label[for="image-range-end"]')?.textContent.trim() || '',
  inspector: document.getElementById('image-modal')?.getAttribute('aria-label') || '',
  result: document.getElementById('image-result-0')?.innerText || ''
}))()"#;
const DESKTOP: &str = r"( () => {
  const filters = document.getElementById('image-filters').getBoundingClientRect();
  const results = document.getElementById('image-results').getBoundingClientRect();
  const list = document.querySelector('#image-results .log-scroll');
  const doc = document.scrollingElement;
  const beforeDoc = doc.scrollTop;
  const beforeFilters = filters && document.getElementById('image-filters').scrollTop;
  const spacer = document.createElement('div');
  spacer.style.height = '2400px';
  list.appendChild(spacer);
  list.scrollTop = 900;
  const scrolled = list.scrollTop > 100;
  const docMoved = doc.scrollTop !== beforeDoc;
  const filtersMoved = document.getElementById('image-filters').scrollTop !== beforeFilters;
  list.scrollTop = 0;
  spacer.remove();
  const paint = (name) => {
    const probe = document.createElement('div');
    probe.style.backgroundColor = 'var(' + name + ')';
    document.body.appendChild(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  };
  const body = getComputedStyle(document.body);
  const titles = [...document.querySelectorAll('h1, h2')].filter((node) => {
    if (node.closest('[hidden]')) return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 2 && rect.height > 2;
  }).map((node) => node.textContent.trim()).filter((title) => ['Agent', 'Logs', 'Metrics', 'Tasks', 'Images', 'Chat'].includes(title));
  const row = Math.abs(filters.top - results.top) < 4 && filters.right <= results.left + 2;
  const pass = Math.abs(filters.width - 240) < 2 && row && titles.length === 1 && titles[0] === 'Images' && body.backgroundColor === paint('--background') && body.color === paint('--foreground') && getComputedStyle(list).overflowY === 'auto' && getComputedStyle(document.getElementById('image-filters')).overflowY === 'auto' && scrolled && !docMoved && !filtersMoved && document.documentElement.scrollWidth <= window.innerWidth + 1 && !document.getElementById('image-inspector') && document.getElementById('inventory-images').hasAttribute('hidden') && document.getElementById('workbench').hasAttribute('hidden') && !!document.getElementById('image-current') && !!document.getElementById('image-current-source') && document.getElementById('image-current-actions')?.parentElement?.id === 'image-view' && document.getElementById('image-current-actions')?.nextElementSibling?.id === 'image-browser' && !document.querySelector('#image-filters .log-actions') && !document.getElementById('image-refresh') && !document.getElementById('image-clear');
  return { pass, titles, width: filters.width, row, scrolled, docMoved, filtersMoved, overflow: document.documentElement.scrollWidth };
})()";
