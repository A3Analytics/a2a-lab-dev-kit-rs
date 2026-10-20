//! Metrics chart page: card, palette, states, and the metric radiogroup.

#![cfg(feature = "web-ui")]

use std::future::Future;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, ListMetricsRequest, MemoryLogs, MemoryMetrics,
    MemoryTasks, MetricDescriptor, MetricId, MetricPoint, MetricProvider, Page, QueryMetricRequest,
    UtcTimestamp, bind_local,
};
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

#[tokio::test]
async fn browser_metrics_chart_matches_the_chart_surface() {
    let base = chart_lab().await;
    tokio::task::spawn_blocking(move || chart_page(&base))
        .await
        .unwrap();
}

fn chart_page(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("metrics");
    page.wait_until(DOOR_READY);
    assert_desktop_rail(&mut page);
    assert_chart_card(&mut page);
    assert_palette(&mut page);
    assert_arrow_keys(&mut page);
    assert_dark_labels(&mut page);
    assert_mobile_stack(&mut page);
    assert_empty_state(&mut page);
    assert_error_state(&mut page);
    assert_loading_state(&mut page);
    assert_catalog_stays_off(&mut page);
    assert_visible_csv(&mut page);
    let console = page.console_text();
    assert!(!console.contains("EvalError"), "{console}");
    assert!(!console.contains("Content Security Policy"), "{console}");
}

fn assert_desktop_rail(page: &mut Browser) {
    page.viewport(1100, 900);
    let layout = page.eval(LAYOUT);
    let rail = layout["rail"].as_f64().unwrap_or(0.0);
    assert!((rail - 240.0).abs() < 2.0, "{layout}");
    let plot_left = layout["plotLeft"].as_f64().unwrap_or(0.0);
    let rail_right = layout["railRight"].as_f64().unwrap_or(0.0);
    assert!(plot_left + 1.0 >= rail_right, "{layout}");
    assert!(
        layout["plotWidth"].as_f64().unwrap_or(0.0) > rail,
        "{layout}"
    );
    assert!(layout["border"].as_f64().unwrap_or(0.0) >= 1.0, "{layout}");
    assert!(layout["radius"].as_f64().unwrap_or(0.0) > 0.0, "{layout}");
}

fn assert_chart_card(page: &mut Browser) {
    let card = page.eval(CARD);
    assert_eq!(card["kind"], json!(""));
    assert_eq!(card["title"], json!("Door"));
    assert_eq!(card["description"], json!(DOOR_DESCRIPTION));
    assert_eq!(card["nowrap"], json!("nowrap"));
    assert_eq!(card["takeaway"], json!(""));
    assert_eq!(card["download"], json!("door.csv"));
    assert_eq!(card["hiddenClass"], json!("visually-hidden"));
    assert_eq!(card["tabs"], json!(["metric-pick-0"]));
    assert_chart_series(&card);
}

fn assert_chart_series(card: &Value) {
    let csv = card["csv"].as_str().unwrap_or_default();
    assert!(csv.contains("timestamp,value"), "{card}");
    assert!(csv.contains("2024-01-01T00:00:00Z,0"), "{card}");
    assert!(csv.contains("2024-01-01T00:01:00Z,1"), "{card}");
    let labels = card["labels"].as_str().unwrap_or_default();
    assert!(labels.contains("00:00"), "{card}");
    assert!(labels.contains("00:01"), "{card}");
    assert!(!labels.contains("2024-01-01 00:00 UTC"), "{card}");
    assert!(labels.contains("open"), "{card}");
    let readings = card["readings"].as_str().unwrap_or_default();
    assert!(readings.contains("2024-01-01T00:01:00Z 1 open"), "{card}");
}

fn assert_palette(page: &mut Browser) {
    let paint = page.eval(PAINT);
    let stroke = paint["stroke"].as_str().unwrap_or_default();
    let grid = paint["grid"].as_str().unwrap_or_default();
    let fill = paint["fill"].as_str().unwrap_or_default();
    assert!(stroke.contains("--chart-"), "{paint}");
    assert_eq!(grid, "var(--border)");
    assert_eq!(fill, "var(--foreground)");
    assert!(!stroke.contains("40, 40"), "{paint}");
    assert_ne!(grid, "rgba(20, 20, 20, 0.8)");
}

fn assert_arrow_keys(page: &mut Browser) {
    press_series_key(page, "ArrowDown");
    page.wait_until(
        "document.getElementById('metric-chart-title')?.textContent === 'Edge' && [...document.querySelectorAll('#metric-chart .dx-line-point')].some((point) => Number(point.getAttribute('data-value')) === 9)",
    );
    assert_eq!(page.eval(TAB_STOP), json!(["metric-pick-1"]));
    press_series_key(page, "ArrowUp");
    page.wait_until(DOOR_READY);
    assert_eq!(page.eval(TAB_STOP), json!(["metric-pick-0"]));
}

fn assert_dark_labels(page: &mut Browser) {
    use_dark(page);
    let tone = page.eval(DARK_TONE);
    assert_eq!(tone["badLine"], Value::Bool(false), "{tone}");
    assert_eq!(tone["badGrid"], Value::Bool(false), "{tone}");
    assert_eq!(tone["readable"], Value::Bool(true), "{tone}");
}

fn assert_mobile_stack(page: &mut Browser) {
    page.viewport(700, 900);
    let layout = page.eval(LAYOUT);
    let plot_top = layout["plotTop"].as_f64().unwrap_or(0.0);
    let rail_bottom = layout["railBottom"].as_f64().unwrap_or(0.0);
    assert!(plot_top + 1.0 >= rail_bottom, "{layout}");
    page.viewport(1100, 900);
}

fn assert_empty_state(page: &mut Browser) {
    page.click("metric-pick-3");
    page.wait_until("document.getElementById('metric-plot')?.dataset.state === 'empty'");
    assert_eq!(
        text(
            page,
            "document.getElementById('metric-view-status')?.textContent"
        ),
        "No samples for this metric."
    );
    assert_eq!(
        page.eval("document.getElementById('metric-error')"),
        Value::Null
    );
    assert!(
        text(page, "document.getElementById('metric-empty')?.textContent")
            .contains("Try a wider time range.")
    );
}

fn assert_error_state(page: &mut Browser) {
    page.click("metric-pick-2");
    page.wait_until("document.getElementById('metric-plot')?.dataset.state === 'error'");
    let status = text(
        page,
        "document.getElementById('metric-view-status')?.textContent",
    );
    assert_eq!(status, "Samples could not be read.");
    assert_ne!(status, "No samples for this metric.");
    assert_eq!(
        page.eval("document.getElementById('metric-empty')"),
        Value::Null
    );
    let alert = text(
        page,
        "document.getElementById('metric-view-alert')?.textContent",
    );
    assert!(alert.contains("sensor offline"), "{alert}");
    assert!(
        !text(page, "document.getElementById('metric-plot')?.textContent").contains("No samples")
    );
}

fn assert_loading_state(page: &mut Browser) {
    page.click("metric-pick-4");
    page.wait_until("document.getElementById('metric-plot')?.dataset.state === 'loading'");
    assert_eq!(
        text(
            page,
            "document.getElementById('metric-view-status')?.textContent"
        ),
        "Loading samples."
    );
    assert_eq!(
        page.eval("document.getElementById('metric-empty')"),
        Value::Null
    );
    assert_eq!(
        page.eval("document.getElementById('metric-error')"),
        Value::Null
    );
    page.wait_until(
        "document.getElementById('metric-plot')?.dataset.state === 'ready' && [...document.querySelectorAll('#metric-chart .dx-line-point')].some((point) => Number(point.getAttribute('data-value')) === 4)",
    );
}

fn assert_catalog_stays_off(page: &mut Browser) {
    assert_eq!(page.eval("location.hash"), json!("#metrics"));
    assert_eq!(
        page.eval("document.getElementById('inventory-metrics')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.getElementById('workbench')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval(
            "document.getElementById('metric-view')?.contains(document.getElementById('workbench'))"
        ),
        Value::Bool(false)
    );
}

fn assert_visible_csv(page: &mut Browser) {
    page.click("metric-pick-0");
    page.wait_until(DOOR_READY);
    page.eval(
        "(() => { const start = document.getElementById('metric-range-start'); const end = document.getElementById('metric-range-end'); start.value = '2024-01-01T00:01:00'; end.value = '2024-01-01T00:02:00'; end.dispatchEvent(new Event('change', { bubbles: true })); return true; })()",
    );
    page.wait_until(
        "(() => { const values = [...document.querySelectorAll('#metric-chart .dx-line-point')].map((point) => Number(point.getAttribute('data-value'))); const href = document.getElementById('metric-chart-csv')?.getAttribute('href') || ''; const csv = decodeURIComponent(href.replace('data:text/csv;charset=utf-8,', '')); return values.length === 1 && values[0] === 1 && csv.includes('2024-01-01T00:01:00Z,1') && !csv.includes('2024-01-01T00:00:00Z'); })()",
    );
    let csv = text(
        page,
        "decodeURIComponent((document.getElementById('metric-chart-csv')?.getAttribute('href') || '').replace('data:text/csv;charset=utf-8,', ''))",
    );
    assert!(csv.contains("2024-01-01T00:01:00Z,1"), "{csv}");
    assert!(!csv.contains("2024-01-01T00:00:00Z"), "{csv}");
}

fn press_series_key(page: &mut Browser, key: &str) {
    let key = serde_json::to_string(key).unwrap_or_default();
    page.eval(&format!(
        "document.getElementById('metric-sources').dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}}))"
    ));
}

fn use_dark(page: &mut Browser) {
    for _ in 0..3 {
        if page.eval("document.documentElement.getAttribute('data-theme')") == json!("dark") {
            return;
        }
        page.click("theme-toggle");
    }
    panic!("dark theme did not apply");
}

fn text(page: &mut Browser, expression: &str) -> String {
    page.eval(expression)
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

const DOOR_READY: &str = "document.getElementById('metric-chart-title')?.textContent === 'Door' && document.querySelectorAll('#metric-chart .dx-line-point').length >= 2";

const LAYOUT: &str = "(() => { const rail = document.getElementById('metric-filters').getBoundingClientRect(); const plot = document.getElementById('metric-plot').getBoundingClientRect(); const card = getComputedStyle(document.getElementById('metric-chart-card')); return { rail: rail.width, railRight: rail.right, railBottom: rail.bottom, plotLeft: plot.left, plotTop: plot.top, plotWidth: plot.width, border: parseFloat(card.borderTopWidth), radius: parseFloat(card.borderTopLeftRadius) }; })()";

const CARD: &str = "(() => { const desc = document.getElementById('metric-chart-description'); const link = document.getElementById('metric-chart-csv'); const href = link?.getAttribute('href') || ''; const csv = decodeURIComponent(href.replace('data:text/csv;charset=utf-8,', '')); const labels = [...document.querySelectorAll('#metric-chart .dx-grid-label')].map((node) => node.textContent).join('|'); return { kind: document.getElementById('metric-chart-kind')?.textContent || '', title: document.getElementById('metric-chart-title')?.textContent, description: desc?.textContent, nowrap: getComputedStyle(desc).whiteSpace, takeaway: document.getElementById('metric-chart-takeaway')?.textContent || '', download: link?.getAttribute('download'), csv, labels, hiddenClass: document.getElementById('metric-view-entries')?.className, readings: document.getElementById('metric-view-entries')?.textContent, tabs: [...document.querySelectorAll('#metric-sources [role=\"radio\"]')].filter((el) => el.tabIndex === 0).map((el) => el.id) }; })()";

const PAINT: &str = "(() => { const path = document.querySelector('#metric-chart .dx-line-path'); const grid = document.querySelector('#metric-chart .dx-grid-line'); const label = document.querySelector('#metric-chart .dx-grid-label'); return { stroke: path?.getAttribute('stroke') || '', grid: grid?.getAttribute('stroke') || '', fill: label?.getAttribute('fill') || '' }; })()";

const TAB_STOP: &str = "(() => [...document.querySelectorAll('#metric-sources [role=\"radio\"]')].filter((el) => el.tabIndex === 0).map((el) => el.id))()";

const DARK_TONE: &str = "(() => { const paint = (selector, prop) => getComputedStyle(document.querySelector(selector))[prop]; const channels = (value) => { const match = String(value).match(/rgba?\\(([^)]+)\\)/); if (!match) return null; const parts = match[1].split(',').map((part) => Number(part.trim())); return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 }; }; const light = (value) => { const oklch = String(value).match(/oklch\\(\\s*([0-9.]+)/); if (oklch) return Number(oklch[1]) > 0.7; const rgb = channels(value); return !!(rgb && rgb.r > 180 && rgb.g > 180 && rgb.b > 180); }; const line = channels(paint('#metric-chart .dx-line-path', 'stroke')); const grid = channels(paint('#metric-chart .dx-grid-line', 'stroke')); const label = paint('#metric-chart .dx-grid-label', 'fill'); return { badLine: !!(line && Math.abs(line.g - 40) < 3 && Math.abs(line.b - 40) < 3), badGrid: !!(grid && grid.r < 30 && grid.g < 30 && grid.b < 30 && grid.a > 0.7), readable: light(label), line: paint('#metric-chart .dx-line-path', 'stroke'), grid: paint('#metric-chart .dx-grid-line', 'stroke'), label }; })()";

const DOOR_DESCRIPTION: &str = "How far the chamber door is open.";

struct ChartMetrics {
    inner: MemoryMetrics,
}

impl MetricProvider for ChartMetrics {
    fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> impl Future<Output = Result<Page<MetricDescriptor>, A2aLabError>> + Send {
        self.inner.list_metrics(request)
    }

    fn query(
        &self,
        request: QueryMetricRequest,
    ) -> impl Future<Output = Result<Page<MetricPoint>, A2aLabError>> + Send {
        let inner = self.inner.clone();
        async move { routed_query(inner, request).await }
    }
}

async fn routed_query(
    inner: MemoryMetrics,
    request: QueryMetricRequest,
) -> Result<Page<MetricPoint>, A2aLabError> {
    match request.metric_id.as_str() {
        "fault" => Err(A2aLabError::unavailable("sensor offline")),
        "slow" => {
            tokio::time::sleep(Duration::from_millis(800)).await;
            inner.query(request).await
        }
        _ => inner.query(request).await,
    }
}

async fn chart_lab() -> String {
    let metrics = MemoryMetrics::new();
    seed(
        &metrics,
        "door",
        "Door",
        DOOR_DESCRIPTION,
        "open",
        &[("2024-01-01T00:00:00Z", 0.0), ("2024-01-01T00:01:00Z", 1.0)],
    )
    .await;
    seed(
        &metrics,
        "edge",
        "Edge",
        "Distance to the near edge.",
        "mm",
        &[("2024-01-01T00:00:00Z", 3.0), ("2024-01-01T00:01:00Z", 9.0)],
    )
    .await;
    seed(
        &metrics,
        "fault",
        "Fault",
        "Sensor that fails.",
        "count",
        &[],
    )
    .await;
    seed(
        &metrics,
        "quiet",
        "Quiet",
        "Nothing recorded in this window.",
        "count",
        &[],
    )
    .await;
    seed(
        &metrics,
        "slow",
        "Slow",
        "Delayed reading.",
        "s",
        &[("2024-01-01T00:00:00Z", 4.0), ("2024-01-01T00:01:00Z", 8.0)],
    )
    .await;
    let service = A2aLabService::new(
        MemoryLogs::new(),
        ChartMetrics { inner: metrics },
        MemoryTasks::new(),
    )
    .share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn seed(
    metrics: &MemoryMetrics,
    id: &str,
    name: &str,
    description: &str,
    unit: &str,
    points: &[(&str, f64)],
) {
    let metric_id = MetricId::new(id).unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: name.to_owned(),
            description: description.to_owned(),
            unit: unit.to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    for (timestamp, value) in points {
        metrics
            .insert_point(
                metric_id.clone(),
                MetricPoint::new(UtcTimestamp::parse(timestamp).unwrap(), *value).unwrap(),
            )
            .await
            .unwrap();
    }
}
