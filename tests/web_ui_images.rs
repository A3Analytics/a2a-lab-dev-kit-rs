//! Image history browser: one source, search, pages, and recovery.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, Image, ImageDescriptor, ImageId, ImageSource, ImageSourceId,
    JsonObject, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, UtcTimestamp, bind_local,
};
use base64::Engine;
use serde_json::Value;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";

#[tokio::test]
async fn browser_image_history_browses_searches_and_recovers() {
    let (filled, images) = filled_lab().await;
    let empty = empty_lab().await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || flow(&filled, &empty, &images, &handle))
        .await
        .unwrap();
}

async fn filled_lab() -> (String, MemoryImages) {
    let images = MemoryImages::new();
    insert_source(&images, "cam-a", "Camera A").await;
    insert_source(&images, "cam-b", "Camera B").await;
    insert_source(&images, "cam-c", "Empty").await;
    for index in 0..101 {
        let caption = (index == 0).then_some("plate well");
        let attributes = if index == 0 {
            JsonObject::parse(r#"{"slot":"1"}"#).unwrap()
        } else {
            JsonObject::empty()
        };
        insert_frame(
            &images,
            &format!("frame-{index:03}"),
            "cam-a",
            &stamp(index),
            caption,
            attributes,
        )
        .await;
    }
    insert_frame(
        &images,
        "other-1",
        "cam-b",
        "2024-02-01T00:00:00Z",
        Some("other bench"),
        JsonObject::empty(),
    )
    .await;
    (serve(images.clone()).await, images)
}

async fn empty_lab() -> String {
    serve(MemoryImages::new()).await
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
    caption: Option<&str>,
    attributes: JsonObject,
) {
    let descriptor = ImageDescriptor::new(
        ImageId::new(id).unwrap(),
        ImageSourceId::new(source).unwrap(),
        UtcTimestamp::parse(at).unwrap(),
        "image/png",
        8,
        4,
        caption.map(str::to_owned),
        attributes,
    )
    .unwrap();
    images
        .insert_image(Image::new(descriptor, vec![1, 2, 3, 4]).unwrap())
        .await
        .unwrap();
}

fn stamp(index: u32) -> String {
    let hour = index / 60;
    let minute = index % 60;
    format!("2024-01-01T{hour:02}:{minute:02}:00Z")
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

#[tokio::test]
async fn browser_image_inspector_previews_current_and_recovers() {
    let (base, images) = inspector_lab().await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || inspect_flow(&base, &images, &handle))
        .await
        .unwrap();
}

fn flow(filled: &str, empty: &str, images: &MemoryImages, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{filled}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#images");
    page.wait_until(CAM_A_PAGE);
    assert_reader(&mut page);
    assert_first_frame(&mut page);
    assert_page(&mut page, filled);
    assert_selection_survives_reload(&mut page);
    assert_source_change(&mut page);
    choose_source(&mut page, "cam-a");
    page.wait_until(CAM_A_PAGE);
    assert_search_clear_and_validation(&mut page, filled);
    assert_history_failure(&mut page, images, handle);
    assert_source_failure(&mut page, images, handle);
    page.goto(&format!("{empty}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#images");
    page.wait_until(
        "document.getElementById('image-source-status')?.dataset.state === 'sources' && document.getElementById('image-source-status')?.textContent.includes('No image sources.') && document.querySelector('#image-sources .log-facet')?.disabled === true && document.querySelector('#image-sources .log-facet')?.textContent.includes('None loaded')",
    );
    assert_workbench_routes(&mut page);
}

fn assert_reader(page: &mut Browser) {
    let width = page
        .eval("document.getElementById('image-filters').getBoundingClientRect().width")
        .as_f64()
        .unwrap_or(0.0);
    assert!((width - 240.0).abs() < 2.0, "{width}");
    assert_eq!(page.eval(ONE_TAB), Value::Bool(true));
    assert_eq!(
        page.eval("document.querySelector('#image-sources > p')?.textContent.trim() ?? ''")
            .as_str()
            .unwrap_or(""),
        "Source"
    );
    let option = page
        .eval("document.querySelector('#image-sources [data-source=\"cam-a\"]')?.textContent ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(option.contains("Camera A"), "{option}");
    assert!(option.contains("cam-a"), "{option}");
    assert!(option.contains("Camera A frame"), "{option}");
    assert_eq!(
        page.eval(
            "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source ?? ''"
        )
        .as_str()
        .unwrap_or(""),
        "cam-a"
    );
    assert_eq!(text(page, "image-source-status"), "");
    assert_live(page, "image-source-status", "status", "polite");
    assert_live(page, "image-source-alert", "alert", "assertive");
    assert_live(page, "image-history-status", "status", "polite");
    assert_live(page, "image-history-alert", "alert", "assertive");
    assert_hidden_catalog(page);
}

fn assert_live(page: &mut Browser, id: &str, role: &str, live: &str) {
    assert_eq!(attr(page, id, "role"), role);
    assert_eq!(attr(page, id, "aria-live"), live);
}

fn assert_hidden_catalog(page: &mut Browser) {
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('inventory-images')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    let body = page
        .eval("document.getElementById('view-images')?.innerText ?? ''")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(!body.contains("More image sources"), "{body}");
    assert!(!body.contains("Command"), "{body}");
}

fn assert_first_frame(page: &mut Browser) {
    let body = text(page, "image-result-0");
    assert!(body.contains("frame-000"), "{body}");
    assert!(body.contains("2024-01-01T00:00:00Z"), "{body}");
    assert!(body.contains("plate well"), "{body}");
    assert!(body.contains("image/png"), "{body}");
    assert!(body.contains("8 × 4"), "{body}");
    assert!(body.contains("slot"), "{body}");
    assert!(!text(page, "image-result-1").contains("plate well"));
}

fn assert_page(page: &mut Browser, base: &str) {
    assert_eq!(
        page.eval("document.getElementById('image-count')"),
        Value::Null
    );
    let before = sends(page, base, "list_images");
    page.click("image-more");
    page.wait_until(FULL_PAGE);
    assert!(sends(page, base, "list_images") > before);
    assert_eq!(page.eval(UNIQUE_IDS), Value::Bool(true));
}

fn assert_selection_survives_reload(page: &mut Browser) {
    page.click("image-result-0");
    page.wait_until(
        "document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true'",
    );
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'frame-000' && document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true'",
    );
    clear_criteria(page);
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'frame-000' && document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true'",
    );
}

fn assert_source_change(page: &mut Browser) {
    choose_source(page, "cam-b");
    page.wait_until(CAM_B);
    assert!(!text(page, "image-history").contains("frame-000"));
    assert!(text(page, "image-history").contains("other bench"));
    choose_source(page, "cam-c");
    page.wait_until(
        "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-c' && document.getElementById('image-history-status')?.textContent.includes('No images for this source.')",
    );
    assert_eq!(page.eval(ONE_TAB), Value::Bool(true));
    page.click("image-source-all");
    page.wait_until(
        "document.getElementById('image-source-all')?.getAttribute('aria-pressed') === 'true' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-history')?.innerText.includes('frame-000') && document.getElementById('image-history')?.innerText.includes('other bench')",
    );
    assert_eq!(
        page.eval("document.getElementById('image-current')"),
        Value::Null
    );
}

fn assert_search_clear_and_validation(page: &mut Browser, base: &str) {
    let searches = sends(page, base, "search_images");
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 1 && document.getElementById('image-result-0')?.dataset.image === 'frame-000'",
    );
    assert!(sends(page, base, "search_images") > searches);
    let body = last_send(page, base, "search_images");
    assert!(body.contains("cam-a"), "{body}");
    assert!(body.contains("plate"), "{body}");
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
    assert_invalid(
        page,
        base,
        "image-search",
        "   ",
        "Enter a search, or clear the field.",
    );
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
    assert_invalid(
        page,
        base,
        "image-range-start",
        "2024-01-02T00:00",
        "Enter a start and an end.",
    );
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
    type_input(page, "image-range-start", "2024-01-02T00:00");
    type_input(page, "image-range-end", "2024-01-01T00:00");
    page.wait_until(
        "document.getElementById('image-history-alert')?.textContent.includes('The start must be before the end.')",
    );
    assert_eq!(sends(page, base, "search_images"), searches + 1);
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
    type_input(page, "image-range-start", "2024-01-01T00:00");
    type_input(page, "image-range-end", "2024-01-01T00:02");
    page.wait_until(
        "document.querySelectorAll('#image-history [data-id]').length === 2 && document.getElementById('image-result-0')?.dataset.image === 'frame-000' && document.getElementById('image-result-1')?.dataset.image === 'frame-001'",
    );
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
}

fn assert_invalid(page: &mut Browser, base: &str, id: &str, value: &str, message: &str) {
    let searches = sends(page, base, "search_images");
    let lists = sends(page, base, "list_images");
    type_input(page, id, value);
    page.wait_until(&format!(
        "document.getElementById('image-history-alert')?.textContent.includes({})",
        serde_json::to_string(message).unwrap_or_default()
    ));
    assert_eq!(sends(page, base, "search_images"), searches);
    assert_eq!(sends(page, base, "list_images"), lists);
}

fn assert_history_failure(
    page: &mut Browser,
    images: &MemoryImages,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(images.set_unavailable("history offline"));
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'error' && document.getElementById('image-history-alert')?.textContent.includes('history offline') && document.getElementById('image-source-status')?.dataset.state === 'ready'",
    );
    assert!(text(page, "image-history-status").contains("Images could not be read."));
    assert!(!text(page, "image-source-status").contains("could not be read"));
    handle.block_on(images.clear_unavailable());
    clear_criteria(page);
    page.wait_until(CAM_A_PAGE);
}

fn assert_source_failure(
    page: &mut Browser,
    images: &MemoryImages,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(images.set_unavailable("camera offline"));
    commit(page, "inventory-images-limit", "100");
    page.wait_until(
        "document.getElementById('image-source-status')?.dataset.state === 'error' && document.getElementById('image-source-alert')?.textContent.includes('camera offline')",
    );
    assert!(text(page, "image-source-status").contains("Image sources could not be read."));
    assert_eq!(
        page.eval("document.querySelector('#image-sources .log-facet')?.disabled"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.querySelector('#image-sources .log-facet')?.textContent.trim() ?? ''")
            .as_str()
            .unwrap_or(""),
        "None loaded"
    );
    assert_ne!(
        text(page, "image-history-status"),
        "No images for this source."
    );
    handle.block_on(images.clear_unavailable());
    commit(page, "inventory-images-limit", "100");
    page.wait_until(CAM_A_PAGE);
}

fn assert_workbench_routes(page: &mut Browser) {
    page.show("#workbench");
    page.wait_until("!document.getElementById('workbench')?.hasAttribute('hidden')");
    assert_eq!(text(page, "workbench-heading"), "Command");
    page.show("#logs");
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    page.show("#images");
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
}

fn clear_criteria(page: &mut Browser) {
    type_input(page, "image-search", "");
    type_input(page, "image-range-start", "");
    type_input(page, "image-range-end", "");
}

fn type_input(page: &mut Browser, id: &str, value: &str) {
    page.set_value(id, value);
    let id = serde_json::to_string(id).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById({id}).dispatchEvent(new Event('input', {{bubbles:true}}))"
    ));
}

fn choose_source(page: &mut Browser, id: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let clicked = page.eval(&format!(
        "(() => {{ const el = document.querySelector('#image-sources [data-source=' + {id} + ']'); if (!el) return false; el.click(); return true; }})()"
    ));
    assert_eq!(clicked, Value::Bool(true), "missing source");
}

fn commit(page: &mut Browser, id: &str, value: &str) {
    let id = serde_json::to_string(id).unwrap_or_default();
    let value = serde_json::to_string(value).unwrap_or_default();
    page.eval(&format!(
        "(() => {{ const el = document.getElementById({id}); el.value = {value}; el.dispatchEvent(new Event('change', {{bubbles:true}})); }})()"
    ));
}

fn sends(page: &Browser, base: &str, operation: &str) -> usize {
    let needle = format!("\"{operation}\"");
    page.requests(base)
        .iter()
        .filter(|request| request_url(request).contains("/message:send"))
        .filter(|request| post_data(request).contains(&needle))
        .count()
}

fn last_send(page: &Browser, base: &str, operation: &str) -> String {
    let needle = format!("\"{operation}\"");
    page.requests(base)
        .iter()
        .rev()
        .filter(|request| request_url(request).contains("/message:send"))
        .map(post_data)
        .find(|body| body.contains(&needle))
        .unwrap_or_default()
}

fn post_data(request: &Value) -> String {
    request["params"]["request"]["postData"]
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

const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const MARKUP: &str = r#"<img id="image-xss" src=x onerror="window.__imageXss=1">"#;

async fn inspector_lab() -> (String, MemoryImages) {
    let images = MemoryImages::new();
    insert_source(&images, "cam-a", "Camera A").await;
    insert_source(&images, "cam-b", "Camera B").await;
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG_B64)
        .unwrap();
    insert_typed(
        &images,
        "png-a",
        "cam-a",
        "2024-01-01T00:00:00Z",
        "image/png",
        Some("plate well"),
        JsonObject::parse(r#"{"slot":"1"}"#).unwrap(),
        png.clone(),
    )
    .await;
    insert_typed(
        &images,
        "svg-b",
        "cam-a",
        "2024-01-01T00:01:00Z",
        "image/svg+xml",
        Some(MARKUP),
        JsonObject::empty(),
        br#"<svg xmlns="http://www.w3.org/2000/svg" onload="window.__imageXss=1"></svg>"#.to_vec(),
    )
    .await;
    insert_typed(
        &images,
        "big-c",
        "cam-a",
        "2024-01-01T00:02:00Z",
        "image/png",
        None,
        JsonObject::empty(),
        vec![9_u8; 262_145],
    )
    .await;
    insert_typed(
        &images,
        "live-d",
        "cam-a",
        "2024-01-01T00:03:00Z",
        "image/png",
        Some("live frame"),
        JsonObject::empty(),
        png,
    )
    .await;
    insert_typed(
        &images,
        "other-b",
        "cam-b",
        "2024-02-01T00:00:00Z",
        "image/png",
        Some("other bench"),
        JsonObject::empty(),
        vec![1, 2, 3, 4],
    )
    .await;
    images
        .set_current(
            ImageSourceId::new("cam-a").unwrap(),
            ImageId::new("live-d").unwrap(),
        )
        .await
        .unwrap();
    (serve(images.clone()).await, images)
}

async fn insert_typed(
    images: &MemoryImages,
    id: &str,
    source: &str,
    at: &str,
    media: &str,
    caption: Option<&str>,
    attributes: JsonObject,
    bytes: Vec<u8>,
) {
    let descriptor = ImageDescriptor::new(
        ImageId::new(id).unwrap(),
        ImageSourceId::new(source).unwrap(),
        UtcTimestamp::parse(at).unwrap(),
        media,
        8,
        4,
        caption.map(str::to_owned),
        attributes,
    )
    .unwrap();
    images
        .insert_image(Image::new(descriptor, bytes).unwrap())
        .await
        .unwrap();
}

fn inspect_flow(base: &str, images: &MemoryImages, handle: &tokio::runtime::Handle) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    page.show("#images");
    page.wait_until(INSPECTOR_READY);
    page.click("image-result-0");
    page.wait_until(PNG_READY);
    page.wait_until(
        "document.getElementById('image-modal') && document.getElementById('image-preview')?.src.startsWith('data:image/png') && !document.getElementById('image-inspector')",
    );
    assert_desktop_layout(&mut page);
    assert_png_metadata(&mut page);
    assert_result_keyboard(&mut page);
    page.click("image-result-0");
    page.wait_until(PNG_READY);
    assert_history_failure_keeps_inspector(&mut page, images, handle);
    assert_selected_failure(&mut page, images, handle);
    page.viewport(767, 900);
    assert_narrow_stack(&mut page);
    page.viewport(1280, 800);
    page.eval(
        "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
    );
    page.wait_until("!document.getElementById('image-modal')");
    choose_source(&mut page, "cam-b");
    page.wait_until(
        "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-b' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.dataset.image === 'other-b' && !document.getElementById('image-modal')",
    );
    assert!(!text(&mut page, "image-history").contains("png-a"));
}

fn assert_desktop_layout(page: &mut Browser) {
    assert_eq!(
        page.eval(DESKTOP_LAYOUT),
        Value::Bool(true),
        "desktop layout"
    );
    assert_eq!(
        page.eval(PREVIEW_BOUNDED),
        Value::Bool(true),
        "preview bounds"
    );
    assert_eq!(
        page.eval("document.getElementById('image-current')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.querySelector('#image-filters .log-actions')"),
        Value::Null
    );
    assert_eq!(attr(page, "image-history", "role"), "radiogroup");
    assert_eq!(attr(page, "image-history", "aria-label"), "Image history");
    assert_eq!(
        page.eval("document.querySelector('#image-sources > p')?.textContent.trim() ?? ''")
            .as_str()
            .unwrap_or(""),
        "Source"
    );
    assert_eq!(attr(page, "image-sources", "role"), "");
    assert_eq!(attr(page, "image-result-0", "role"), "radio");
    assert_eq!(page.eval(ONE_RESULT_TAB), Value::Bool(true));
    assert_eq!(page.eval(ONE_TAB), Value::Bool(true));
}

fn assert_png_metadata(page: &mut Browser) {
    assert_eq!(attr(page, "image-modal", "data-image"), "png-a");
    assert_eq!(attr(page, "image-modal", "data-origin"), "history");
    assert_eq!(attr(page, "image-preview", "alt"), "plate well");
    assert!(attr(page, "image-preview", "src").starts_with("data:image/png;base64,"));
    assert!(!attr(page, "image-preview", "src").contains("svg"));
    assert_eq!(
        page.eval("document.getElementById('image-facts')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('image-origin')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('image-withheld')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('image-modal-title')"),
        Value::Null
    );
}

fn assert_result_keyboard(page: &mut Browser) {
    page.eval("window.__imageXss = 0");
    press_result(page, "ArrowDown");
    page.wait_until(SVG_WITHHELD);
    assert_eq!(page.eval(ONE_RESULT_TAB), Value::Bool(true));
    assert_eq!(attr(page, "image-result-1", "tabindex"), "0");
    assert_eq!(attr(page, "image-result-0", "tabindex"), "-1");
    assert_eq!(attr(page, "image-modal", "data-image"), "svg-b");
    assert_eq!(
        page.eval("document.getElementById('image-preview')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('image-xss')"),
        Value::Null
    );
    assert_eq!(page.eval("window.__imageXss"), Value::from(0));
    assert_eq!(
        page.eval("document.querySelector('#image-inspector svg')"),
        Value::Null
    );
    press_result(page, "ArrowDown");
    page.wait_until(OVERSIZED_WITHHELD);
    assert_eq!(attr(page, "image-modal", "data-image"), "big-c");
    assert_eq!(
        page.eval("document.getElementById('image-preview')"),
        Value::Null
    );
}

fn assert_history_failure_keeps_inspector(
    page: &mut Browser,
    images: &MemoryImages,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(images.set_unavailable("history offline"));
    type_input(page, "image-search", "plate");
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'error' && document.getElementById('image-history-alert')?.textContent.includes('history offline') && document.getElementById('image-modal')?.dataset.image === 'png-a'",
    );
    assert!(text(page, "image-history-status").contains("Images could not be read."));
    assert_eq!(
        page.eval(
            "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source ?? ''"
        )
        .as_str()
        .unwrap_or(""),
        "cam-a"
    );
    handle.block_on(images.clear_unavailable());
    clear_criteria(page);
    page.wait_until(
        "document.getElementById('image-history-status')?.dataset.state === 'ready' && document.getElementById('image-result-0')?.getAttribute('aria-checked') === 'true' && document.getElementById('image-modal')?.dataset.image === 'png-a'",
    );
}

fn assert_selected_failure(
    page: &mut Browser,
    images: &MemoryImages,
    handle: &tokio::runtime::Handle,
) {
    handle.block_on(images.set_unavailable("selected offline"));
    page.click("image-result-1");
    page.wait_until(
        "document.getElementById('image-modal')?.dataset.state === 'selected-error' && !document.getElementById('image-preview') && document.getElementById('image-result-1')?.getAttribute('aria-checked') === 'true' && document.querySelectorAll('#image-history [data-id]').length === 4",
    );
    assert_eq!(
        page.eval(
            "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source ?? ''"
        )
        .as_str()
        .unwrap_or(""),
        "cam-a"
    );
    handle.block_on(images.clear_unavailable());
    page.click("image-result-1");
    page.wait_until(SVG_WITHHELD);
}

fn assert_narrow_stack(page: &mut Browser) {
    assert_eq!(page.eval(NARROW_STACK), Value::Bool(true), "narrow stack");
}

fn press_result(page: &mut Browser, key: &str) {
    let key = serde_json::to_string(key).unwrap_or_default();
    let moved = page.eval(&format!(
        "(() => {{ const current = document.querySelector('#image-history [aria-checked=\"true\"]') || document.querySelector('#image-history [role=\"radio\"]'); if (!current) return false; current.focus(); current.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}})); return true; }})()"
    ));
    assert_eq!(moved, Value::Bool(true));
}

const INSPECTOR_READY: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 4 && document.getElementById('image-result-0')?.dataset.image === 'png-a'";
const PNG_READY: &str = "document.getElementById('image-modal')?.dataset.state === 'selected-ready' && document.getElementById('image-modal')?.dataset.image === 'png-a' && document.getElementById('image-preview')?.complete && document.getElementById('image-preview')?.naturalWidth === 1 && !document.getElementById('image-facts') && !document.getElementById('image-origin') && !document.getElementById('image-modal-title')";
const SVG_WITHHELD: &str = "document.getElementById('image-modal')?.dataset.state === 'selected-ready' && document.getElementById('image-modal')?.dataset.image === 'svg-b' && !document.getElementById('image-preview') && !document.querySelector('#image-modal svg')";
const OVERSIZED_WITHHELD: &str = "document.getElementById('image-modal')?.dataset.image === 'big-c' && !document.getElementById('image-preview')";
const ONE_RESULT_TAB: &str = "(() => { const stops = [...document.querySelectorAll('#image-history [role=\"radio\"]')].filter(el => el.tabIndex === 0); return stops.length === 1; })()";
const DESKTOP_LAYOUT: &str = r"( () => {
  const filters = document.getElementById('image-filters').getBoundingClientRect();
  const results = document.getElementById('image-results').getBoundingClientRect();
  const row = Math.abs(filters.top - results.top) < 4;
  const order = filters.right <= results.left + 2 && results.right <= window.innerWidth + 2;
  const wide = filters.width > 200 && results.width > 80;
  const overflow = document.documentElement.scrollWidth <= window.innerWidth + 1;
  return row && order && wide && overflow && !document.getElementById('image-inspector');
})()";
const PREVIEW_BOUNDED: &str = r"( () => {
  const img = document.getElementById('image-preview');
  if (!img || !img.naturalWidth || !img.naturalHeight) return false;
  const box = img.getBoundingClientRect();
  const parent = img.parentElement.getBoundingClientRect();
  const style = getComputedStyle(img);
  const ratio = img.clientWidth / img.clientHeight;
  const natural = img.naturalWidth / img.naturalHeight;
  return style.objectFit === 'contain' && box.width <= parent.width + 1 && box.height <= window.innerHeight && Math.abs(ratio - natural) < 0.05;
})()";
const NARROW_STACK: &str = r"( () => {
  const filters = document.getElementById('image-filters');
  const results = document.getElementById('image-results');
  const box = (el) => el.getBoundingClientRect();
  const filtersBox = box(filters);
  const resultsBox = box(results);
  const stacked = filtersBox.bottom <= resultsBox.top + 2;
  const overflow = document.documentElement.scrollWidth <= window.innerWidth + 1;
  const ids = ['image-sources', 'image-result-0', 'image-modal'];
  const reach = ids.every((id) => {
    const el = document.getElementById(id);
    if (!el) return false;
    el.scrollIntoView({block: 'center'});
    const item = el.getBoundingClientRect();
    return item.width > 0 && item.height > 0 && item.top >= -1 && item.bottom <= window.innerHeight + 1 && item.left >= -1 && item.right <= window.innerWidth + 1;
  });
  return stacked && overflow && reach && getComputedStyle(document.getElementById('image-browser')).flexDirection === 'column' && !document.getElementById('image-inspector');
})()";

const CAM_A_PAGE: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-a' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 100 && document.getElementById('image-result-0')?.dataset.image === 'frame-000'";
const CAM_B: &str = "document.querySelector('#image-sources [aria-pressed=true]')?.dataset.source === 'cam-b' && document.getElementById('image-history-status')?.dataset.state === 'ready' && document.querySelectorAll('#image-history [data-id]').length === 1";
const FULL_PAGE: &str = "document.querySelectorAll('#image-history [data-id]').length === 101 && document.querySelector('#image-history [data-id]:last-child')?.dataset.id === 'frame-100'";
const UNIQUE_IDS: &str = "(() => { const ids = [...document.querySelectorAll('#image-history [data-id]')].map(el => el.dataset.id); return new Set(ids).size === ids.length && ids[0] === 'frame-000' && ids.at(-1) === 'frame-100'; })()";
const ONE_TAB: &str = "(() => { const group = document.getElementById('image-sources'); const facets = [...(group?.querySelectorAll('.log-facet') || [])]; const pressed = facets.filter((el) => el.getAttribute('aria-pressed') === 'true'); return document.querySelector('#image-filters > .log-filter-group') === group && !document.querySelector('#image-filters select') && facets.length > 0 && pressed.length === 1 && facets.every((el) => el.tagName === 'BUTTON'); })()";
