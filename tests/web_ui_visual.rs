//! Closeout visual parity across routes, viewports, and color schemes.

#![cfg(feature = "web-ui")]

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, ImageSource, ImageSourceId, JsonObject, LogLevel,
    LogRecord, LogSource, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor,
    MetricId, MetricPoint, SourceId, TaskDefinition, TaskId, UtcTimestamp, bind_local,
};
use serde_json::Value;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const ROUTES: [Route; 6] = [
    Route {
        hash: "#logs",
        title: "Logs",
        kind: Kind::Logs,
        ready: "!!document.querySelector('#log-view-entries li')",
    },
    Route {
        hash: "#metrics",
        title: "Metrics",
        kind: Kind::Metrics,
        ready: "document.getElementById('metric-chart-title')?.textContent === 'Latency' && !!document.querySelector('#metric-chart .dx-line-path')",
    },
    Route {
        hash: "#tasks",
        title: "Tasks",
        kind: Kind::Tasks,
        ready: "document.getElementById('task-detail-name')?.textContent === 'Home'",
    },
    Route {
        hash: "#images",
        title: "Images",
        kind: Kind::Images,
        ready: "document.getElementById('image-history-status')?.dataset.state === 'empty'",
    },
    Route {
        hash: "#chat",
        title: "Chat",
        kind: Kind::Chat,
        ready: "!!document.querySelector('#chat-log .chat-row-assistant')",
    },
    Route {
        hash: "#workbench",
        title: "Logs",
        kind: Kind::Workbench,
        ready: "document.getElementById('nav-logs')?.getAttribute('aria-current') === 'page' && !document.getElementById('workbench')?.hasAttribute('hidden')",
    },
];

const VIEWPORTS: [(u32, u32); 3] = [(767, 768), (768, 768), (1843, 768)];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Logs,
    Metrics,
    Tasks,
    Images,
    Chat,
    Workbench,
}

#[derive(Clone, Copy)]
struct Route {
    hash: &'static str,
    title: &'static str,
    kind: Kind,
    ready: &'static str,
}

struct Spec {
    route: Route,
    width: u32,
    height: u32,
    scheme: &'static str,
}

struct Check {
    name: &'static str,
    pass: bool,
    detail: String,
}

#[tokio::test]
async fn browser_visual_parity_covers_every_route() {
    let base = lab().await;
    tokio::task::spawn_blocking(move || visual_flow(&base))
        .await
        .unwrap();
}

fn visual_flow(base: &str) {
    let mut failures = Vec::new();
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    for scheme in ["light", "dark"] {
        apply_scheme(&mut page, scheme);
        seed_chat(&mut page);
        for (width, height) in VIEWPORTS {
            page.viewport(width, height);
            for route in ROUTES {
                failures.extend(visit(
                    &mut page,
                    &Spec {
                        route,
                        width,
                        height,
                        scheme,
                    },
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn visit(page: &mut Browser, spec: &Spec) -> Vec<String> {
    page.show(spec.route.hash);
    wait_ready(page, spec);
    let report = page.eval(PROBE);
    let mut checks = vec![
        check_bootstrap(spec, &report),
        check_shell(spec, &report),
        check_title(spec, &report),
        check_list(spec, &report),
        check_chart(spec, &report),
        check_chat(spec, &report),
        check_states(spec, &report),
        check_motion(spec, &report),
        check_intentional(spec, &report),
    ];
    checks.push(check_rail(page, spec));
    checks.push(check_focus(page, spec));
    println!("{}", record(spec, &checks));
    checks
        .into_iter()
        .filter(|check| !check.pass)
        .map(|check| check.detail)
        .collect()
}

fn wait_ready(page: &mut Browser, spec: &Spec) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(8) {
        if spec.route.hash == "#metrics" {
            page.eval(
                "document.querySelector('#metric-filter-chips [data-clear=\"range\"]')?.click()",
            );
        }
        if spec.route.hash == "#logs" || spec.route.hash == "#workbench" {
            page.eval(
                "document.querySelector('#log-filter-chips [data-clear=\"range\"]')?.click()",
            );
        }
        if spec.route.hash == "#images" {
            page.eval("['image-range-start','image-range-end'].forEach((id) => { const el = document.getElementById(id); if (!el || !el.value) return; el.value = ''; el.dispatchEvent(new Event('input', {bubbles:true})); })");
        }
        if page.eval(&format!("!!({})", spec.route.ready)) == Value::Bool(true) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let status = page.eval(
        "document.getElementById('metric-view-status')?.textContent || document.getElementById('log-view-status')?.textContent || document.getElementById('task-view-status')?.textContent || ''",
    );
    println!("unsettled {} status={status}", spec.route.hash);
}

fn record(spec: &Spec, checks: &[Check]) -> String {
    let mut line = spec_label(spec);
    for check in checks {
        line.push(' ');
        line.push_str(check.name);
        line.push('=');
        line.push_str(if check.pass { "pass" } else { "fail" });
    }
    line
}

fn check_bootstrap(spec: &Spec, report: &Value) -> Check {
    let children = strings(report, "children");
    let leftover = flag(report, "bootstrap");
    let pass = children.as_slice() == ["shell"] && !leftover;
    let actual = format!("children {children:?} bootstrap {leftover}");
    judged(
        spec,
        "bootstrap",
        pass,
        "no bootstrap line and #app child shell",
        &actual,
    )
}

fn check_shell(spec: &Spec, report: &Value) -> Check {
    let y = num(report, "shellY");
    judged(spec, "shell", y.abs() < 1.0, "shell y=0", &format!("{y}"))
}

fn check_title(spec: &Spec, report: &Value) -> Check {
    let titles = strings(report, "titles");
    let pass = titles.as_slice() == [spec.route.title];
    judged(
        spec,
        "title",
        pass,
        &format!("one visible route title {}", spec.route.title),
        &format!("{titles:?}"),
    )
}

fn check_list(spec: &Spec, report: &Value) -> Check {
    let present = flag(report, "listPresent");
    let stacked = flag(report, "listStacked");
    let width = num(report, "listWidth");
    let (pass, expected) = list_expectation(spec, present, stacked, width);
    judged(
        spec,
        "list",
        pass,
        expected,
        &format!("present {present} stacked {stacked} width {width:.1}"),
    )
}

fn list_expectation(spec: &Spec, present: bool, stacked: bool, width: f64) -> (bool, &'static str) {
    if !spec.route.kind.has_list() {
        return (!present, "no list rail");
    }
    if spec.width < 768 {
        return (present && stacked && width > 300.0, "stacked list rail");
    }
    (present && !stacked && near(width, 240.0), "15rem list rail")
}

fn check_chart(spec: &Spec, report: &Value) -> Check {
    let tokens = token_problem(spec, report);
    let card = card_problem(spec, report);
    let detail = join_problems([&tokens, &card]);
    judged(
        spec,
        "chart",
        detail.is_empty(),
        "token colors and chart card",
        if detail.is_empty() {
            "matched"
        } else {
            &detail
        },
    )
}

fn token_problem(spec: &Spec, report: &Value) -> String {
    let body = text(report, "bodyBg");
    let background = text(report, "tokenBg");
    let foreground = text(report, "bodyFg");
    let token_fg = text(report, "tokenFg");
    let nav = text(report, "navBg");
    let cream = text(report, "tokenCream");
    let lum = num(report, "bodyLum");
    let tone = if spec.scheme == "dark" {
        lum < 80.0
    } else {
        lum > 200.0
    };
    if body == background && foreground == token_fg && nav == cream && tone && !body.is_empty() {
        return String::new();
    }
    format!(
        "body {body} token {background} fg {foreground}/{token_fg} nav {nav}/{cream} lum {lum:.0}"
    )
}

fn card_problem(spec: &Spec, report: &Value) -> String {
    if spec.route.kind != Kind::Metrics {
        return hidden_card(report);
    }
    metric_card(spec, report)
}

fn hidden_card(report: &Value) -> String {
    if flag(report, "chartVisible") {
        "chart card visible off Metrics".to_owned()
    } else {
        String::new()
    }
}

fn metric_card(spec: &Spec, report: &Value) -> String {
    let mut problems = Vec::new();
    expect_text(&mut problems, report, "kind", "");
    expect_text(&mut problems, report, "takeaway", "");
    expect_contains(&mut problems, report, "labels", "ms");
    expect_text(&mut problems, report, "csv", "latency.csv");
    expect_text(&mut problems, report, "grid", "var(--border)");
    expect_text(&mut problems, report, "fill", "var(--foreground)");
    expect_contains(&mut problems, report, "stroke", "--chart-");
    expect_same(
        &mut problems,
        report,
        "cardBg",
        "tokenCard",
        "card background",
    );
    expect_same(
        &mut problems,
        report,
        "cardBorder",
        "tokenBorder",
        "card border",
    );
    if num(report, "borderWidth") < 1.0 {
        problems.push("card border missing".to_owned());
    }
    if num(report, "radius") <= 0.0 {
        problems.push("card radius missing".to_owned());
    }
    if !readable_label(spec.scheme, num(report, "labelLum")) {
        problems.push(format!("label lum {}", num(report, "labelLum")));
    }
    if !flag(report, "chartVisible") {
        problems.push("chart card not visible".to_owned());
    }
    problems.join("; ")
}

fn readable_label(scheme: &str, lum: f64) -> bool {
    if scheme == "dark" {
        lum > 180.0
    } else {
        lum < 80.0
    }
}

fn check_chat(spec: &Spec, report: &Value) -> Check {
    let (pass, actual) = chat_observation(spec, report);
    judged(
        spec,
        "chat",
        pass,
        "chat rows and independent scroll",
        &actual,
    )
}

fn chat_observation(spec: &Spec, report: &Value) -> (bool, String) {
    if spec.route.kind != Kind::Chat {
        let hidden = flag(report, "chatHidden");
        return (hidden, format!("chat hidden {hidden}"));
    }
    let rows = num(report, "rows");
    let overflow = text(report, "chatOverflow");
    let inside = flag(report, "draftInside");
    let main = text(report, "mainOverflow");
    let independent = flag(report, "independent");
    let pass = rows >= 2.0 && overflow == "auto" && !inside && main == "hidden" && independent;
    (
        pass,
        format!(
            "rows {rows} overflow {overflow} draftInside {inside} main {main} independent {independent}"
        ),
    )
}

fn check_states(spec: &Spec, report: &Value) -> Check {
    let main = text(report, "visibleMain");
    let loading = text(report, "loading");
    let mut problems = Vec::new();
    for phrase in spec.route.kind.required() {
        if !main.contains(phrase) {
            problems.push(format!("missing {phrase}"));
        }
    }
    for phrase in spec.route.kind.forbidden() {
        if main.contains(phrase) {
            problems.push(format!("showing {phrase}"));
        }
    }
    if !loading.is_empty() {
        problems.push(format!("loading {loading}"));
    }
    let actual = if problems.is_empty() {
        "settled copy".to_owned()
    } else {
        problems.join("; ")
    };
    judged(
        spec,
        "states",
        problems.is_empty(),
        "loading, empty, and error copy for this route",
        &actual,
    )
}

fn check_motion(spec: &Spec, report: &Value) -> Check {
    let motion = text(report, "motion");
    let wave = text(report, "wave");
    let pass = motion == "0ms" && wave == "none";
    judged(
        spec,
        "motion",
        pass,
        "reduced motion duration 0ms and strand animation none",
        &format!("motion {motion} wave {wave}"),
    )
}

fn check_intentional(spec: &Spec, report: &Value) -> Check {
    let mut problems = Vec::new();
    shared_differences(&mut problems, report);
    route_differences(&mut problems, spec, report);
    let actual = if problems.is_empty() {
        "held".to_owned()
    } else {
        problems.join("; ")
    };
    judged(
        spec,
        "intentional",
        problems.is_empty(),
        "intentional lab differences",
        &actual,
    )
}

fn shared_differences(problems: &mut Vec<String>, report: &Value) {
    if flag(report, "account") {
        problems.push("account or footer is present".to_owned());
    }
    if !text(report, "cookie").is_empty() {
        problems.push(format!("cookie {}", text(report, "cookie")));
    }
    if strings(report, "nav").as_slice() != NAV {
        problems.push(format!("nav {:?}", strings(report, "nav")));
    }
    if flag(report, "chartJs") {
        problems.push("Chart.js is present".to_owned());
    }
    if !flag(report, "catalogLogs")
        || !flag(report, "catalogMetrics")
        || !flag(report, "catalogTasks")
        || !flag(report, "catalogImages")
    {
        problems.push("a catalog is visible".to_owned());
    }
}

fn route_differences(problems: &mut Vec<String>, spec: &Spec, report: &Value) {
    if spec.route.kind == Kind::Metrics && !flag(report, "dioxusLine") {
        problems.push("metrics chart is not a Dioxus SVG line".to_owned());
    }
    if spec.route.kind == Kind::Workbench {
        workbench_differences(problems, report);
    }
    if spec.route.kind == Kind::Logs && !flag(report, "runnerHidden") {
        problems.push("command runner is visible on Logs".to_owned());
    }
    if spec.route.kind == Kind::Images {
        image_differences(problems, report);
    }
}

fn image_differences(problems: &mut Vec<String>, report: &Value) {
    if !flag(report, "runnerHidden") {
        problems.push("command runner is visible on Images".to_owned());
    }
    if !flag(report, "imageParts") {
        problems.push("Images is missing its history and preview parts".to_owned());
    }
    if !flag(report, "imageCapture") {
        problems.push("Images is missing current-image capture".to_owned());
    }
}

fn workbench_differences(problems: &mut Vec<String>, report: &Value) {
    if text(report, "current") != "page" {
        problems.push(format!("nav-logs {}", text(report, "current")));
    }
    if flag(report, "runnerHidden") {
        problems.push("command runner hidden on #workbench".to_owned());
    }
    if text(report, "command") != "Command" {
        problems.push(format!("command heading {}", text(report, "command")));
    }
}

fn check_rail(page: &mut Browser, spec: &Spec) -> Check {
    let (pass, actual) = if spec.width < 768 {
        mobile_rail(page)
    } else {
        desktop_rail(page)
    };
    judged(spec, "rail", pass, "16rem rail", &actual)
}

fn desktop_rail(page: &mut Browser) -> (bool, String) {
    let nav = page.computed("#primary-nav");
    let gap = page.computed(".rail-gap");
    let width = num(&nav, "width");
    let gap_width = num(&gap, "width");
    let position = text(&nav, "position");
    let hidden = flag(&nav, "hidden");
    let top = num(&nav, "top");
    let pass = near(width, 256.0)
        && near(gap_width, 256.0)
        && position == "absolute"
        && !hidden
        && top.abs() < 1.5;
    (
        pass,
        format!("width {width:.1} gap {gap_width:.1} {position} hidden {hidden} top {top:.1}"),
    )
}

fn mobile_rail(page: &mut Browser) -> (bool, String) {
    let closed = page.computed("#primary-nav");
    let closed_hidden = flag(&closed, "hidden");
    if !closed_hidden {
        page.click("sidebar-toggle");
    }
    let closed = page.computed("#primary-nav");
    let closed_hidden = flag(&closed, "hidden");
    page.click("sidebar-toggle");
    let opened = poll(
        page,
        "document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'true'",
    );
    let open = page.computed("#primary-nav");
    let width = num(&open, "width");
    let position = text(&open, "position");
    let top = num(&open, "top");
    let left = num(&open, "left");
    page.eval(
        "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
    );
    let closed_again = poll(
        page,
        "document.getElementById('primary-nav')?.hasAttribute('hidden')",
    );
    let pass = closed_hidden
        && opened
        && closed_again
        && near(width, 256.0)
        && position == "fixed"
        && top.abs() < 1.5
        && left.abs() < 1.5;
    (
        pass,
        format!("closed {closed_hidden} width {width:.1} {position} top {top:.1} left {left:.1}"),
    )
}

fn check_focus(page: &mut Browser, spec: &Spec) -> Check {
    page.click_box("sidebar-toggle");
    let mouse = text(&page.computed("#sidebar-toggle"), "outlineStyle");
    page.click("sidebar-toggle");
    page.press_tab();
    let ring = page
        .eval("getComputedStyle(document.activeElement).outlineWidth")
        .as_str()
        .unwrap_or("")
        .to_owned();
    let pass = mouse == "none" && ring.contains("3px");
    judged(
        spec,
        "focus",
        pass,
        "mouse outline none and keyboard outline 3px",
        &format!("mouse {mouse} keyboard {ring}"),
    )
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
        "document.documentElement.dataset.theme === {quoted} && {class} && document.getElementById('connection')?.dataset.state === 'connected'"
    ));
    page.set_media(scheme, true);
}

fn seed_chat(page: &mut Browser) {
    page.show("#chat");
    page.wait_until("document.getElementById('chat')?.dataset.enabled === 'true'");
    page.set_value("chat-draft", "visual row");
    page.click("chat-send");
    page.wait_until(
        "document.querySelector('#chat-log .chat-row-assistant')?.textContent.includes('echo:visual row')",
    );
}

fn judged(spec: &Spec, name: &'static str, pass: bool, expected: &str, actual: &str) -> Check {
    let detail = if pass {
        String::new()
    } else {
        format!(
            "{}: {name} expected {expected}, actual {actual}",
            spec_label(spec)
        )
    };
    Check { name, pass, detail }
}

fn spec_label(spec: &Spec) -> String {
    format!(
        "{} {}x{} {}",
        spec.route.hash, spec.width, spec.height, spec.scheme
    )
}

fn expect_text(problems: &mut Vec<String>, report: &Value, key: &str, expected: &str) {
    let actual = text(report, key);
    if actual != expected {
        problems.push(format!("{key} {actual}"));
    }
}

fn expect_contains(problems: &mut Vec<String>, report: &Value, key: &str, needle: &str) {
    let actual = text(report, key);
    if !actual.contains(needle) {
        problems.push(format!("{key} {actual}"));
    }
}

fn expect_same(problems: &mut Vec<String>, report: &Value, left: &str, right: &str, label: &str) {
    let actual = text(report, left);
    let expected = text(report, right);
    if actual != expected || actual.is_empty() {
        problems.push(format!("{label} {actual} vs {expected}"));
    }
}

fn join_problems(parts: [&String; 2]) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("; ")
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn num(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 2.0
}

fn poll(page: &mut Browser, expression: &str) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(2) {
        if page.eval(&format!("!!({expression})")) == Value::Bool(true) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    false
}

impl Kind {
    const fn has_list(self) -> bool {
        matches!(
            self,
            Self::Logs | Self::Metrics | Self::Tasks | Self::Images | Self::Workbench
        )
    }

    fn required(self) -> &'static [&'static str] {
        match self {
            Self::Logs => &["pump ready"],
            Self::Metrics => &["Latency", "Data CSV", "ms"],
            Self::Tasks => &["Home", "This task has no input fields."],
            Self::Images => &["Bench", "No images for this source."],
            Self::Chat => &["visual row", "echo:visual row"],
            Self::Workbench => &["pump ready", "Command"],
        }
    }

    fn forbidden(self) -> &'static [&'static str] {
        match self {
            Self::Logs | Self::Workbench => LOG_FORBIDDEN,
            Self::Metrics => METRIC_FORBIDDEN,
            Self::Tasks => TASK_FORBIDDEN,
            Self::Images => IMAGE_FORBIDDEN,
            Self::Chat => CHAT_FORBIDDEN,
        }
    }
}

const NAV: [&str; 5] = ["/ui/chat", "/ui/logs", "/ui/metrics", "/ui/tasks", "/ui/images"];
const LOG_FORBIDDEN: &[&str] = &[
    "Loading the lab interface.",
    "Loading logs.",
    "No logs for this source.",
    "No log sources.",
    "Logs could not be read.",
];
const METRIC_FORBIDDEN: &[&str] = &[
    "Loading samples.",
    "No samples for this metric.",
    "Samples could not be read.",
];
const TASK_FORBIDDEN: &[&str] = &[
    "Choose a task.",
    "Loading task definitions.",
    "The inventory is empty.",
    "could not be started",
    "could not be read",
];
const IMAGE_FORBIDDEN: &[&str] = &[
    "Loading image sources.",
    "No image sources.",
    "Image sources could not be read.",
    "Loading images.",
    "No images match this search.",
    "Images could not be read.",
];
const CHAT_FORBIDDEN: &[&str] = &[
    "Send a message to start.",
    "does not advertise agent-message",
    "Enter a message.",
];

const PROBE: &str = r#"(() => {
  const visible = (el) => {
    if (!el || el.closest('[hidden]')) return false;
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return false;
    const box = el.getBoundingClientRect();
    return box.width > 2 && box.height > 2;
  };
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
  const names = new Set(['Logs', 'Metrics', 'Tasks', 'Images', 'Chat']);
  const view = document.querySelector('#main > .view:not([hidden])');
  const rail = view ? view.querySelector('#log-filters, #metric-filters, #task-filters, #image-filters') : null;
  const railBox = rail ? rail.getBoundingClientRect() : null;
  const card = document.getElementById('metric-chart-card');
  const cardStyle = card ? getComputedStyle(card) : null;
  const path = document.querySelector('#metric-chart .dx-line-path');
  const grid = document.querySelector('#metric-chart .dx-grid-line');
  const label = document.querySelector('#metric-chart .dx-grid-label');
  const labelColor = label ? getComputedStyle(label).fill : '';
  const log = document.getElementById('chat-log');
  const chatView = document.getElementById('view-chat');
  let independent = true;
  if (chatView && !chatView.hasAttribute('hidden') && log) {
    const spacer = document.createElement('div');
    spacer.style.height = '2400px';
    log.appendChild(spacer);
    const before = document.scrollingElement.scrollTop;
    log.scrollTop = 600;
    independent = log.scrollTop > 100 && document.scrollingElement.scrollTop === before;
    log.scrollTop = 0;
    spacer.remove();
  }
  const shell = document.getElementById('shell').getBoundingClientRect();
  const body = getComputedStyle(document.body);
  const nav = document.getElementById('primary-nav');
  return {
    bootstrap: document.body.innerText.includes('Loading the lab interface.'),
    children: [...document.getElementById('app').children].map((node) => node.id),
    shellY: shell.top,
    titles: [...document.querySelectorAll('h1, h2')].filter(visible).map((node) => node.textContent.trim()).filter((title) => names.has(title)),
    listPresent: !!(rail && visible(rail)),
    listStacked: !!(rail && getComputedStyle(rail.parentElement).flexDirection === 'column'),
    listWidth: railBox ? railBox.width : 0,
    chartVisible: visible(card),
    kind: document.getElementById('metric-chart-kind')?.textContent || '',
    takeaway: document.getElementById('metric-chart-takeaway')?.textContent || '',
    labels: [...document.querySelectorAll('#metric-chart .dx-grid-label')].map((node) => node.textContent).join('|'),
    csv: document.getElementById('metric-chart-csv')?.getAttribute('download') || '',
    stroke: path?.getAttribute('stroke') || '',
    grid: grid?.getAttribute('stroke') || '',
    fill: label?.getAttribute('fill') || '',
    cardBg: cardStyle ? cardStyle.backgroundColor : '',
    tokenCard: paint('--card'),
    cardBorder: cardStyle ? cardStyle.borderTopColor : '',
    tokenBorder: paint('--border'),
    borderWidth: cardStyle ? parseFloat(cardStyle.borderTopWidth) : 0,
    radius: cardStyle ? parseFloat(cardStyle.borderTopLeftRadius) : 0,
    labelLum: lum(labelColor),
    bodyBg: body.backgroundColor,
    tokenBg: paint('--background'),
    bodyFg: body.color,
    tokenFg: paint('--foreground'),
    bodyLum: lum(body.backgroundColor),
    navBg: getComputedStyle(nav).backgroundColor,
    tokenCream: paint('--a3-cream'),
    rows: document.querySelectorAll('#chat-log .chat-row').length,
    chatOverflow: log ? getComputedStyle(log).overflowY : '',
    draftInside: !!(log && log.contains(document.getElementById('chat-draft'))),
    mainOverflow: getComputedStyle(document.getElementById('main')).overflowY,
    independent,
    chatHidden: !!chatView?.hasAttribute('hidden'),
    visibleMain: document.getElementById('main')?.innerText || '',
    loading: visible(document.getElementById('loading-root')) ? document.getElementById('loading-root').innerText : '',
    motion: getComputedStyle(document.documentElement).getPropertyValue('--motion-normal').trim(),
    wave: getComputedStyle(document.querySelector('.strand-wave')).animationName,
    account: !!document.querySelector('footer') || /log out/i.test(document.body.innerText),
    cookie: document.cookie,
    nav: [...document.querySelectorAll('#primary-nav > a')].map((node) => node.getAttribute('href')),
    chartJs: typeof window.Chart === 'function' || !!document.querySelector('canvas, script[src*="chart.js"]'),
    dioxusLine: !!path,
    catalogLogs: document.getElementById('inventory-logs')?.hasAttribute('hidden') === true,
    catalogMetrics: document.getElementById('inventory-metrics')?.hasAttribute('hidden') === true,
    catalogTasks: document.getElementById('inventory-tasks')?.hasAttribute('hidden') === true,
    catalogImages: document.getElementById('inventory-images')?.hasAttribute('hidden') === true,
    imageParts: ['image-filters', 'image-results'].every((id) => !!document.getElementById(id)) && !document.getElementById('image-inspector'),
    imageCapture: !!document.getElementById('image-current') && !!document.getElementById('image-current-source') && document.getElementById('image-current-actions')?.parentElement?.id === 'image-view' && document.getElementById('image-current-actions')?.nextElementSibling?.id === 'image-browser' && !document.querySelector('#image-filters .log-actions') && !document.getElementById('image-refresh') && !document.getElementById('image-clear') && document.querySelector('#image-filters > .log-filter-group') === document.getElementById('image-sources') && (document.getElementById('image-sources')?.querySelectorAll('.log-facet').length || 0) > 0,
    current: document.getElementById('nav-logs')?.getAttribute('aria-current') || '',
    runnerHidden: document.getElementById('workbench')?.hasAttribute('hidden') === true,
    command: document.getElementById('workbench-heading')?.textContent || ''
  };
})()"#;

async fn lab() -> String {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app").unwrap(),
        name: "App".to_owned(),
        description: "App log".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_record(LogRecord {
        source_id: SourceId::new("app").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        level: LogLevel::Info,
        message: "pump ready".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await
    .unwrap();
    let metrics = MemoryMetrics::new();
    let metric_id = MetricId::new("latency").unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: "Latency".to_owned(),
            description: "Request latency.".to_owned(),
            unit: "ms".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            metric_id,
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(), 5.0).unwrap(),
        )
        .await
        .unwrap();
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("home").unwrap(),
            name: "Home".to_owned(),
            description: "Home the axes".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let images = MemoryImages::new();
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new("bench").unwrap(),
            name: "Bench".to_owned(),
            description: "Bench frame".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await
        .unwrap();
    let service = A2aLabService::new(logs, metrics, tasks)
        .with_images(images)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service)
        .with_web_ui()
        .with_message_handler(Arc::new(Echo));
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

struct Echo;

impl AgentMessageHandler for Echo {
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>> {
        Box::pin(async move {
            Ok(AgentMessageReply {
                text: format!("echo:{}", request.text),
            })
        })
    }
}
