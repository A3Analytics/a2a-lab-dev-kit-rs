//! Headless regression for sliding H.264 live store playback in the web UI.

#![cfg(feature = "web-ui")]

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

#[path = "support/h264_live_playback_bench.rs"]
mod h264_live_playback_bench;

use browser::{Browser, header, request_method, request_url};
use h264_live_playback_bench::{BEARER_TOKEN, BENCH_SOURCE, live_store_lab};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const OBSERVE: Duration = Duration::from_secs(8);
const SAMPLE: Duration = Duration::from_millis(200);
const MAX_GEN_CHANGES: usize = 12;

#[tokio::test]
async fn browser_h264_live_store_playback_without_effect_loop() {
    let lab = live_store_lab(12).await.expect("bench");
    let base = lab.base.clone();
    tokio::task::spawn_blocking(move || live_regression(&base))
        .await
        .unwrap();
}

fn live_regression(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.viewport(1280, 800);
    assert_no_csp_violations(&page, "open");
    page.wait_until("document.getElementById('bearer-token')");
    page.set_value("bearer-token", BEARER_TOKEN);
    page.eval(
        "(() => { const el = document.getElementById('bearer-token'); el?.dispatchEvent(new Event('input', { bubbles: true })); })()",
    );
    page.show("#videos");
    page.wait_until(&format!(
        "document.getElementById('video-source-status')?.dataset.state === 'ready' && document.getElementById('video-source')?.value === '{BENCH_SOURCE}'"
    ));
    page.wait_until("document.getElementById('video-timeline')?.dataset.ready === 'true'");
    page.click("video-follow-newest");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-status')?.dataset.state === 'connecting' || window.__a2aHlsTrace?.appendOrder?.length > 0",
    );
    if !wait_live_progress(&mut page, Duration::from_secs(60)) {
        let diagnostics = page.eval(
            "JSON.stringify({
                status: document.getElementById('video-status')?.dataset.state ?? '',
                alert: document.getElementById('video-alert')?.textContent ?? '',
                playback: document.getElementById('video-player')?.dataset.playback ?? '',
                currentTime: document.getElementById('video-player')?.currentTime ?? 0,
                src: document.getElementById('video-player')?.getAttribute('src') ?? '',
                trace: window.__a2aHlsTrace ?? null,
            })",
        );
        panic!("live HLS never made decode progress: {diagnostics}");
    }
    resume_live_player(&mut page);
    install_regression_probe(&mut page);
    let trace = observe_window(&mut page);
    assert_no_csp_violations(&page, "playback");
    assert_live_diagnostics(base, &page, &trace);
}

fn wait_live_progress(page: &mut Browser, budget: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < budget {
        let ready = page.eval(
            "(() => {
                const trace = window.__a2aHlsTrace;
                const player = document.getElementById('video-player');
                const appended = trace?.appendOrder?.some((item) => item.endsWith('.m4s'));
                const currentTime = player instanceof HTMLVideoElement ? player.currentTime : 0;
                const state = document.getElementById('video-status')?.dataset.state ?? '';
                return appended && (currentTime > 0 || state === 'playing' || state === 'connecting');
            })()",
        );
        if ready == serde_json::json!(true) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn resume_live_player(page: &mut Browser) {
    page.eval(
        "(() => {
            if (typeof window.__a2aNativePlay === 'function') window.__a2aNativePlay();
        })()",
    );
    page.wait_until(
        "(() => {
            const player = document.getElementById('video-player');
            return player instanceof HTMLVideoElement && !player.paused && player.currentTime > 0;
        })()",
    );
}

fn install_regression_probe(page: &mut Browser) {
    page.eval(
        "(() => {
            const view = () => document.getElementById('video-view');
            window.__a2aLiveRegression = {
                seen: [],
                last: '',
                oscillations: 0,
                note(gen) {
                    if (!gen || gen === this.last) return;
                    if (this.seen.length >= 2) {
                        const a = this.seen[this.seen.length - 2];
                        const b = this.seen[this.seen.length - 1];
                        if (a === gen && b === this.last) this.oscillations += 1;
                    }
                    this.seen.push(gen);
                    if (this.seen.length > 48) this.seen.shift();
                    this.last = gen;
                },
                tick() {
                    this.note(view()?.dataset.gen ?? '');
                },
                summary() {
                    return {
                        seen: this.seen.slice(),
                        oscillations: this.oscillations,
                        last: this.last,
                    };
                },
            };
            window.__a2aLiveRegression._timer = setInterval(
                () => window.__a2aLiveRegression.tick(),
                50,
            );
            return true;
        })()",
    );
}

fn observe_window(page: &mut Browser) -> LiveTrace {
    let start = Instant::now();
    let mut samples = Vec::new();
    while start.elapsed() < OBSERVE {
        samples.push(read_sample(page));
        thread::sleep(SAMPLE);
    }
    LiveTrace {
        samples,
        hls: page.eval("window.__a2aHlsTrace ?? null"),
        regression: page.eval("window.__a2aLiveRegression?.summary?.() ?? null"),
        console: page.console_text(),
    }
}

fn read_sample(page: &mut Browser) -> LiveSample {
    let value = page.eval(
        "(() => ({
            gen: document.getElementById('video-view')?.dataset.gen ?? '',
            buffering: document.getElementById('video-player')?.dataset.buffering === 'true',
            currentTime: document.getElementById('video-player')?.currentTime ?? 0,
            status: document.getElementById('video-status')?.dataset.state ?? '',
            append_count: (window.__a2aHlsTrace?.appendOrder ?? []).length,
        }))()",
    );
    LiveSample {
        generation: value["gen"].as_str().unwrap_or("").to_owned(),
        buffering: value["buffering"].as_bool().unwrap_or(false),
        current_time: value["currentTime"].as_f64(),
        status: value["status"].as_str().unwrap_or("").to_owned(),
        append_count: value["append_count"].as_u64().unwrap_or(0),
    }
}

fn assert_no_csp_violations(page: &Browser, stage: &str) {
    let console = page.console_text();
    assert!(
        !console.contains("Content Security Policy"),
        "{stage} console: {console}"
    );
    assert!(!console.contains("EvalError"), "{stage} console: {console}");
    assert!(
        !console.contains("Refused to"),
        "{stage} console: {console}"
    );
}

fn assert_live_diagnostics(base: &str, page: &Browser, trace: &LiveTrace) {
    let buffering_hits: Vec<_> = trace
        .samples
        .iter()
        .filter(|sample| sample.buffering)
        .collect();
    assert!(
        buffering_hits.is_empty(),
        "buffering stalled during observation: {buffering_hits:#?}\nregression: {}\nconsole: {}",
        trace.regression,
        trace.console
    );
    assert_eq!(
        trace.samples.last().map(|sample| sample.buffering),
        Some(false),
        "expected data-buffering=false at end: {:#?}\nconsole: {}",
        trace.samples,
        trace.console
    );
    let min_time = trace
        .samples
        .iter()
        .filter_map(|sample| sample.current_time)
        .fold(f64::INFINITY, f64::min);
    let max_time = trace
        .samples
        .iter()
        .filter_map(|sample| sample.current_time)
        .fold(0.0_f64, f64::max);
    let append_advance = trace
        .samples
        .last()
        .map_or(0, |sample| sample.append_count)
        .saturating_sub(
            trace
                .samples
                .first()
                .map_or(0, |sample| sample.append_count),
        );
    let time_advance = max_time - min_time;
    assert!(
        time_advance > 0.1 || append_advance > 0,
        "video.currentTime and MSE appends stalled (time {time_advance}, appends +{append_advance}); trace: {:#?}\nconsole: {}",
        trace.samples,
        trace.console
    );
    let gen_changes = trace
        .samples
        .windows(2)
        .filter(|pair| pair[0].generation != pair[1].generation)
        .count();
    assert!(
        gen_changes <= MAX_GEN_CHANGES,
        "video-view data-gen churned {gen_changes} times (max {MAX_GEN_CHANGES}): {:#?}\nregression: {}",
        trace.samples,
        trace.regression
    );
    let oscillations = trace
        .regression
        .get("oscillations")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert_eq!(
        oscillations, 0,
        "repeated Dioxus effect loop suspected: {}\nconsole: {}",
        trace.regression, trace.console
    );
    assert!(
        trace.samples.iter().all(|sample| {
            matches!(sample.status.as_str(), "playing" | "paused" | "connecting")
        }),
        "playback entered a failure state: {:#?}\nconsole: {}",
        trace.samples,
        trace.console
    );
    assert_authenticated_live_fetches(base, page, trace);
}

fn assert_authenticated_live_fetches(base: &str, page: &Browser, trace: &LiveTrace) {
    let media_requests: Vec<_> = page
        .requests(base)
        .into_iter()
        .filter(|request| request_url(request).contains("/media/video-live/"))
        .collect();
    assert!(
        media_requests.len() >= 3,
        "expected live manifest, init, and segment fetches; got {} requests\nhls trace: {}\nconsole: {}",
        media_requests.len(),
        trace.hls,
        trace.console
    );
    let bearer = format!("Bearer {BEARER_TOKEN}");
    for request in &media_requests {
        assert_eq!(request_method(request), "GET");
        assert_eq!(header(request, "Authorization"), Some(bearer.as_str()));
        let url = request_url(request);
        assert!(!url.contains("token="), "credential leaked into URL {url}");
    }
    let trace_hls = &trace.hls;
    if trace_hls.is_object() {
        let order = trace_hls
            .get("appendOrder")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            order.iter().any(|item| {
                item.as_str().is_some_and(|uri| {
                    uri.ends_with("init.mp4")
                        || std::path::Path::new(uri)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("m4s"))
                })
            }),
            "MSE trace missing segment append: {trace_hls}"
        );
    }
}

#[derive(Debug)]
struct LiveSample {
    generation: String,
    buffering: bool,
    current_time: Option<f64>,
    status: String,
    append_count: u64,
}

struct LiveTrace {
    samples: Vec<LiveSample>,
    hls: Value,
    regression: Value,
    console: String,
}
