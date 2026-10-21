//! Live Dioxus SVG metric chart in the metrics view (single app runtime).

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use dioxus_charts::{LineChart, LineChartPoint};
use wasm_bindgen::JsCast;

use crate::inventory::Item;
use crate::metrics::{QueryRange, Sample};
use crate::shell::session;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartModel {
    pub samples: Vec<Sample>,
    pub unit: String,
    pub status: String,
    pub alert: String,
}

#[derive(Clone, PartialEq)]
pub struct MetricQuery {
    pub metric: String,
    pub range: QueryRange,
}

impl Default for MetricQuery {
    fn default() -> Self {
        Self {
            metric: String::new(),
            range: QueryRange::Window,
        }
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
    static SCOPE: RefCell<Option<ScopeId>> = const { RefCell::new(None) };
    static PICKS: RefCell<Option<Signal<Vec<Item>>>> = const { RefCell::new(None) };
    static SELECTED: RefCell<Option<Signal<String>>> = const { RefCell::new(None) };
    static QUERY: RefCell<Option<Signal<MetricQuery>>> = const { RefCell::new(None) };
    static FETCH: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
}

fn fill_default_range(query: Signal<MetricQuery>, mut filled: Signal<bool>) {
    if filled() {
        return;
    }
    let QueryRange::Bounded { start, end } = query.read().range.clone() else {
        return;
    };
    let start = datetime_local(&start);
    let end = datetime_local(&end);
    session::write_control("metric-range-start", &start);
    session::write_control("metric-range-end", &end);
    if session::control_value("metric-range-end") == end {
        filled.set(true);
    }
}

fn datetime_local(stamp: &str) -> String {
    stamp.chars().take(19).collect()
}

fn bind(
    picks: Signal<Vec<Item>>,
    selected: Signal<String>,
    query: Signal<MetricQuery>,
    fetch: Signal<u32>,
) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
    SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
    PICKS.with(|slot| *slot.borrow_mut() = Some(picks));
    SELECTED.with(|slot| *slot.borrow_mut() = Some(selected));
    QUERY.with(|slot| *slot.borrow_mut() = Some(query));
    FETCH.with(|slot| *slot.borrow_mut() = Some(fetch));
}

fn bump_fetch() {
    run(|| {
        FETCH.with(|slot| {
            if let Some(mut fetch) = *slot.borrow() {
                let next = fetch.peek().wrapping_add(1);
                fetch.set(next);
            }
        });
    });
}

pub fn run<F: FnOnce()>(job: F) {
    let runtime = RUNTIME.with(|slot| slot.borrow().clone());
    let scope = SCOPE.with(|slot| *slot.borrow());
    if let (Some(runtime), Some(scope)) = (runtime, scope) {
        let _guard = RuntimeGuard::new(runtime.clone());
        runtime.in_scope(scope, job);
        runtime.needs_update(scope);
        return;
    }
    job();
}

pub fn set_picks(items: Vec<Item>) {
    run(move || {
        PICKS.with(|slot| {
            if let Some(mut picks) = *slot.borrow() {
                picks.set(items);
            }
        });
    });
}

pub fn set_selected(id: String) {
    run(move || {
        SELECTED.with(|slot| {
            if let Some(mut selected) = *slot.borrow() {
                selected.set(id);
            }
        });
    });
}

pub fn set_query(next: MetricQuery) {
    run(move || {
        QUERY.with(|slot| {
            if let Some(mut query) = *slot.borrow() {
                query.set(next);
            }
        });
        bump_fetch();
    });
}

#[component]
pub fn MetricView() -> Element {
    let picks = use_signal(Vec::<Item>::new);
    let selected = use_signal(String::new);
    let query = use_signal(|| MetricQuery {
        metric: String::new(),
        range: super::metrics::selected_range(),
    });
    let range_alert = use_signal(String::new);
    let fetch = use_signal(|| 0u32);
    let filled_range = use_signal(|| false);
    use_hook(|| bind(picks, selected, query, fetch));
    use_effect(move || fill_default_range(query, filled_range));
    let chart = use_resource(move || {
        let _n = fetch();
        let q = query();
        async move {
            let metric = q.metric.clone();
            let model = super::metrics::load_chart(q).await;
            (metric, model)
        }
    });
    let _ = chart.read();
    let requested = query();
    let loaded = chart()
        .filter(|(metric, _)| metric == &requested.metric)
        .map(|(_, model)| model);
    let pending = loaded.is_none() && !requested.metric.is_empty();
    let data = loaded.unwrap_or_default();
    let shown = visible_samples(data.samples.clone(), &requested.range);
    let model = ChartModel {
        samples: shown,
        unit: data.unit.clone(),
        status: data.status.clone(),
        alert: data.alert.clone(),
    };
    let view = readout(
        pending,
        &requested.metric,
        &data,
        model.samples.is_empty(),
        &range_alert(),
    );
    let identity = format!("{}:{}", chart_identity(&model), model.samples.len());
    let choice = selected_choice(&picks(), &requested.metric);
    let csv = csv_href(&model.samples);
    let filename = csv_name(&choice.id);
    let status = view.status.clone();
    let alert = view.alert.clone();
    let phase = view.phase;
    rsx! {
        section {
            id: "metric-view",
            aria_labelledby: "metric-view-heading",
            aria_busy: if phase == "loading" { "true" } else { "false" },
            h2 { id: "metric-view-heading", class: "visually-hidden", tabindex: "-1", "Metrics" }
            p { id: "metric-view-status", role: "status", aria_live: "polite", "{status.clone()}" }
            div { id: "metric-view-alert", role: "alert", aria_live: "assertive", "{alert}" }
            div { id: "metric-browser", class: "log-browser",
                div {
                    id: "metric-filters",
                    role: "group",
                    aria_label: "Metric filters",
                    div { class: "log-filter-group",
                        p { "Time range" }
                        label { r#for: "metric-range-start", "Start" }
                        input {
                            id: "metric-range-start",
                            r#type: "datetime-local",
                            step: "1",
                            onchange: move |event| commit_range(event, query, range_alert, fetch, true),
                            onkeydown: move |event| range_tab(event, true),
                        }
                        label { r#for: "metric-range-end", "End" }
                        input {
                            id: "metric-range-end",
                            r#type: "datetime-local",
                            step: "1",
                            onchange: move |event| commit_range(event, query, range_alert, fetch, false),
                            onkeydown: move |event| range_tab(event, false),
                        }
                    }
                    div { class: "log-filter-group",
                        p { "Metric" }
                        MetricPicks { picks, selected, query, fetch }
                    }
                }
                div { id: "metric-results", class: "log-results",
                    p { id: "metric-filter-count", class: "log-count", {count_text(model.samples.len())} }
                    div { class: "log-scroll",
                        FilterChips { query, fetch }
                        ChartCard {
                            phase,
                            title: choice.title,
                            description: choice.description,
                            csv,
                            filename,
                            message: status,
                            model: model.clone(),
                            identity: identity.clone(),
                        }
                    }
                }
            }
        }
    }
}

struct Readout {
    status: String,
    alert: String,
    phase: &'static str,
}

fn readout(
    pending: bool,
    metric: &str,
    data: &ChartModel,
    shown_empty: bool,
    range_alert: &str,
) -> Readout {
    let phase = if pending {
        "loading"
    } else if !data.alert.is_empty() {
        "error"
    } else if shown_empty && !metric.is_empty() {
        "empty"
    } else {
        "ready"
    };
    Readout {
        status: readout_status(phase, data),
        alert: readout_alert(phase, data, range_alert),
        phase,
    }
}

fn readout_status(phase: &str, data: &ChartModel) -> String {
    if phase == "loading" {
        return "Loading samples.".to_owned();
    }
    if phase == "empty" {
        return empty_copy(&data.status);
    }
    data.status.clone()
}

fn empty_copy(status: &str) -> String {
    if status.is_empty() {
        "No samples for this metric.".to_owned()
    } else {
        status.to_owned()
    }
}

fn readout_alert(phase: &str, data: &ChartModel, range_alert: &str) -> String {
    if !range_alert.is_empty() {
        return range_alert.to_owned();
    }
    if phase == "error" {
        return data.alert.clone();
    }
    String::new()
}

struct Choice {
    id: String,
    title: String,
    description: String,
}

fn selected_choice(items: &[Item], id: &str) -> Choice {
    let item = items.iter().find(|item| item.id == id);
    Choice {
        id: id.to_owned(),
        title: item.map(metric_title).unwrap_or_default(),
        description: item
            .map(|item| item.description.clone())
            .unwrap_or_default(),
    }
}

fn metric_title(item: &Item) -> String {
    if item.name.is_empty() {
        item.id.clone()
    } else {
        item.name.clone()
    }
}

#[component]
fn ChartCard(
    phase: &'static str,
    title: String,
    description: String,
    csv: String,
    filename: String,
    message: String,
    model: ChartModel,
    identity: String,
) -> Element {
    rsx! {
        figure { id: "metric-chart-card", class: "metric-chart-card",
            figcaption { class: "metric-chart-caption",
                h3 { id: "metric-chart-title", "{title}" }
                p { id: "metric-chart-description", "{description}" }
            }
            div {
                id: "metric-plot",
                class: "metric-plot",
                "data-state": "{phase}",
                if phase == "loading" {
                    div { id: "metric-plot-skeleton", class: "metric-plot-skeleton skeleton-block" }
                } else if phase == "empty" {
                    div { id: "metric-empty", class: "metric-state",
                        p { "No samples for this metric." }
                        p { "Try a wider time range." }
                    }
                } else if phase == "error" {
                    div { id: "metric-error", class: "metric-state",
                        p { "{message}" }
                    }
                } else {
                    for token in [identity.clone()] {
                        ChartLayer { key: "{token}", model: model.clone() }
                    }
                }
            }
            div { class: "metric-chart-foot",
                a {
                    id: "metric-chart-csv",
                    href: "{csv}",
                    download: "{filename}",
                    aria_label: "Download visible samples",
                    "Data CSV"
                }
            }
        }
    }
}

fn visible_samples(samples: Vec<Sample>, range: &QueryRange) -> Vec<Sample> {
    match range {
        QueryRange::Window => samples,
        QueryRange::Bounded { start, end } => {
            let start = stamp_key(start);
            let end = stamp_key(end);
            samples
                .into_iter()
                .filter(|sample| {
                    let stamp = stamp_key(&sample.timestamp);
                    stamp.as_str() >= start.as_str() && stamp.as_str() < end.as_str()
                })
                .collect()
        }
    }
}

fn stamp_key(value: &str) -> String {
    value.chars().take(19).collect()
}

#[component]
fn FilterChips(mut query: Signal<MetricQuery>, mut fetch: Signal<u32>) -> Element {
    let range = match query().range {
        QueryRange::Window => None,
        QueryRange::Bounded { start, end } => Some((start, end)),
    };
    rsx! {
        div {
            id: "metric-filter-chips",
            class: "log-chips",
            hidden: range.is_none(),
            if let Some((start, end)) = range.clone() {
                span { class: "log-chips-label", "Filters" }
                span { class: "log-meta",
                    span { class: "log-meta-label", "Time:" }
                    span { {format!("{} – {}", clock_label(&start), clock_label(&end))} }
                    button {
                        r#type: "button",
                        "data-clear": "range",
                        aria_label: format!(
                            "Remove Time {} – {}",
                            clock_label(&start),
                            clock_label(&end)
                        ),
                        onclick: move |_| {
                            session::write_control("metric-range-start", "");
                            session::write_control("metric-range-end", "");
                            super::metrics::remember_range(QueryRange::Window);
                            let mut next = query();
                            next.range = QueryRange::Window;
                            query.set(next);
                            let next_fetch = *fetch.peek();
                            fetch.set(next_fetch.wrapping_add(1));
                        },
                        "×"
                    }
                }
            }
        }
    }
}

#[component]
fn MetricPicks(
    picks: Signal<Vec<Item>>,
    selected: Signal<String>,
    query: Signal<MetricQuery>,
    fetch: Signal<u32>,
) -> Element {
    let items = picks();
    let current = selected();
    rsx! {
        div {
            id: "metric-sources",
            role: "radiogroup",
            aria_label: "Metrics",
            onkeydown: picks_keydown,
            if items.is_empty() {
                button {
                    id: "metric-pick-0",
                    r#type: "button",
                    class: "metric-source",
                    role: "radio",
                    tabindex: "0",
                    aria_checked: "false",
                    aria_disabled: "true",
                    "None loaded"
                }
            } else {
                for (index, item) in items.into_iter().enumerate() {
                    MetricPick {
                        index,
                        item,
                        selected: current.clone(),
                        query,
                        fetch,
                    }
                }
            }
        }
    }
}

#[component]
fn MetricPick(
    index: usize,
    item: Item,
    selected: String,
    mut query: Signal<MetricQuery>,
    mut fetch: Signal<u32>,
) -> Element {
    let id = item.id.clone();
    let name = if item.name.is_empty() {
        item.id.clone()
    } else {
        item.name.clone()
    };
    let detail = super::metrics::source_detail(&item);
    let checked = item.id == selected;
    rsx! {
        button {
            id: "metric-pick-{index}",
            r#type: "button",
            class: "metric-source",
            role: "radio",
            tabindex: if checked { "0" } else { "-1" },
            "data-metric": "{item.id}",
            aria_checked: if checked { "true" } else { "false" },
            onclick: move |_| {
                super::metrics::apply_pick(id.clone());
                let mut next = query();
                next.metric = id.clone();
                query.set(next);
                let next_fetch = *fetch.peek();
                fetch.set(next_fetch.wrapping_add(1));
            },
            span { class: "metric-source-name", "{name}" }
            span { class: "metric-source-detail", "{detail}" }
        }
    }
}

#[component]
pub fn ChartLayer(model: ChartModel) -> Element {
    let identity = format!("{}:{}", chart_identity(&model), model.samples.len());
    rsx! {
        if !model.samples.is_empty() {
            div {
                key: "{identity}",
                MetricChart { key: "{identity}", model }
            }
        }
    }
}

fn commit_range(
    event: FormEvent,
    query: Signal<MetricQuery>,
    range_alert: Signal<String>,
    fetch: Signal<u32>,
    is_start: bool,
) {
    let event_value = event.value();
    let mut start = session::control_value("metric-range-start");
    let mut end = session::control_value("metric-range-end");
    if is_start && !event_value.is_empty() {
        start = event_value;
    } else if !is_start && !event_value.is_empty() {
        end = event_value;
    }
    commit_bounds(start, end, query, range_alert, fetch);
}

fn commit_bounds(
    start: String,
    end: String,
    mut query: Signal<MetricQuery>,
    mut range_alert: Signal<String>,
    _fetch: Signal<u32>,
) {
    match crate::metrics::query_range(&start, &end) {
        Ok(QueryRange::Window) => range_alert.set(String::new()),
        Ok(range) => {
            range_alert.set(String::new());
            super::metrics::remember_range(range.clone());
            let mut next = query();
            next.range = range.clone();
            query.set(next);
            bump_fetch();
            present_shown(&visible_samples(super::metrics::cached_samples(), &range));
        }
        Err(message) => range_alert.set(message.to_owned()),
    }
}

fn present_shown(samples: &[Sample]) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let unit = super::metrics::cached_unit();
    if let Some(count) = document.get_element_by_id("metric-filter-count") {
        count.set_text_content(Some(&count_text(samples.len())));
    }
    if let Some(link) = document.get_element_by_id("metric-chart-csv") {
        let _ = link.set_attribute("href", &csv_href(samples));
    }
    if let Some(chart) = document.get_element_by_id("metric-chart") {
        let _ = chart.set_attribute("aria-label", &chart_summary(samples, &unit));
        if let Ok(points) = chart.query_selector_all(".dx-line-point") {
            let keep = crate::metrics::chart_values(samples);
            for index in (0..points.length()).rev() {
                let Some(point) = points
                    .item(index)
                    .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                let value = point
                    .get_attribute("data-value")
                    .and_then(|raw| raw.parse::<f32>().ok());
                let shown = value.is_some_and(|value| {
                    keep.iter().any(|item| (*item - value).abs() < f32::EPSILON)
                });
                if !shown {
                    point.remove();
                }
            }
        }
    }
    if let Some(list) = document.get_element_by_id("metric-view-entries")
        && let Ok(items) = list.query_selector_all("li")
    {
        for index in (0..items.length()).rev() {
            let Some(item) = items
                .item(index)
                .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let text = item.text_content().unwrap_or_default();
            let shown = samples
                .iter()
                .any(|sample| text.contains(&sample.timestamp) && text.contains(&sample.value));
            if !shown {
                item.remove();
            }
        }
    }
}

fn range_tab(event: KeyboardEvent, start: bool) {
    if event.key() != Key::Tab {
        return;
    }
    if event.modifiers().shift() {
        if !start {
            event.prevent_default();
            super::nav::focus_heading("metric-range-start");
        }
        return;
    }
    event.prevent_default();
    if start {
        super::nav::focus_heading("metric-range-end");
    } else {
        focus_checked_pick();
    }
}

fn focus_checked_pick() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let mut slot = 0;
    let mut fallback = None;
    loop {
        let Some(element) = document.get_element_by_id(&format!("metric-pick-{slot}")) else {
            break;
        };
        if slot == 0 {
            fallback = Some(element.clone());
        }
        if element.get_attribute("aria-checked").as_deref() == Some("true") {
            focus_html(element);
            return;
        }
        slot += 1;
    }
    if let Some(element) = fallback {
        focus_html(element);
    }
}

fn focus_html(element: web_sys::Element) {
    if let Ok(html) = element.dyn_into::<web_sys::HtmlElement>() {
        let _ = html.focus();
    }
}

fn picks_keydown(event: KeyboardEvent) {
    let delta: isize = match event.key() {
        Key::ArrowDown | Key::ArrowRight => 1,
        Key::ArrowUp | Key::ArrowLeft => -1,
        _ => return,
    };
    event.prevent_default();
    super::metrics::select_neighbor(delta);
}

fn chart_identity(data: &ChartModel) -> String {
    let mut identity = format!("{}:{}", data.unit, data.samples.len());
    for sample in &data.samples {
        identity.push(':');
        identity.push_str(&sample.timestamp);
        identity.push(':');
        identity.push_str(&sample.value);
    }
    identity
}

fn count_text(total: usize) -> String {
    match total {
        1 => "1 sample".to_owned(),
        count => format!("{count} samples"),
    }
}

fn clock_label(timestamp: &str) -> String {
    utc_label(timestamp)
}

struct StampParts<'a> {
    date: &'a str,
    hour: &'a str,
    minute: &'a str,
    second: &'a str,
}

fn stamp_parts(timestamp: &str) -> Option<StampParts<'_>> {
    let (date, rest) = timestamp.split_once('T')?;
    let clock = rest.trim_end_matches('Z').split('.').next().unwrap_or(rest);
    let mut parts = clock.split(':');
    Some(StampParts {
        date,
        hour: parts.next()?,
        minute: parts.next().unwrap_or("00"),
        second: parts.next().unwrap_or("00"),
    })
}

fn utc_label(timestamp: &str) -> String {
    let Some(parts) = stamp_parts(timestamp) else {
        return timestamp.to_owned();
    };
    if parts.second == "00" {
        format!("{} {}:{} UTC", parts.date, parts.hour, parts.minute)
    } else {
        format!(
            "{} {}:{}:{} UTC",
            parts.date, parts.hour, parts.minute, parts.second
        )
    }
}

fn axis_time_labels(samples: &[Sample]) -> Vec<String> {
    let dates: Vec<&str> = samples
        .iter()
        .filter_map(|sample| stamp_parts(&sample.timestamp).map(|parts| parts.date))
        .collect();
    let one_day = dates
        .first()
        .is_some_and(|first| dates.iter().all(|date| date == first));
    let shown = axis_time_slots(samples.len());
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            if shown.contains(&index) {
                axis_time_label(&sample.timestamp, one_day)
            } else {
                String::new()
            }
        })
        .collect()
}

fn axis_time_slots(count: usize) -> Vec<usize> {
    if count <= 4 {
        return (0..count).collect();
    }
    let last = count - 1;
    let mut slots = vec![0, last / 3, (last * 2) / 3, last];
    slots.sort_unstable();
    slots.dedup();
    slots
}

fn axis_time_label(timestamp: &str, one_day: bool) -> String {
    let Some(parts) = stamp_parts(timestamp) else {
        return timestamp.to_owned();
    };
    let clock = if parts.second == "00" {
        format!("{}:{}", parts.hour, parts.minute)
    } else {
        format!("{}:{}:{}", parts.hour, parts.minute, parts.second)
    };
    if one_day {
        clock
    } else {
        format!("{} {clock}", parts.date)
    }
}

fn csv_name(id: &str) -> String {
    if id.is_empty() {
        "samples.csv".to_owned()
    } else {
        format!("{id}.csv")
    }
}

fn csv_href(samples: &[Sample]) -> String {
    format!(
        "data:text/csv;charset=utf-8,{}",
        encode_component(&csv_document(samples))
    )
}

fn csv_document(samples: &[Sample]) -> String {
    let mut csv = String::from("timestamp,value\n");
    for sample in samples {
        csv.push_str(&csv_field(&sample.timestamp));
        csv.push(',');
        csv.push_str(&csv_field(&sample.value));
        csv.push('\n');
    }
    csv
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[derive(Props, Clone, PartialEq)]
struct MetricChartProps {
    model: ChartModel,
}

#[component]
fn MetricChart(props: MetricChartProps) -> Element {
    let data = props.model;
    let values = crate::metrics::chart_values(&data.samples);
    let labels = axis_time_labels(&data.samples);
    let unit = data.unit.clone();
    let summary = chart_summary(&data.samples, &unit);
    let mut hovered = use_signal(|| None::<LineChartPoint>);
    let mut focused = use_signal(|| None::<LineChartPoint>);
    let point_label = {
        let samples = data.samples.clone();
        let unit = unit.clone();
        move |point: LineChartPoint| -> String {
            let Some(sample) = samples.get(point.point_index) else {
                return String::new();
            };
            let suffix = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            format!("{} {}{suffix}", utc_label(&sample.timestamp), sample.value)
        }
    };

    let identity = format!("{}:{}", chart_identity(&data), data.samples.len());
    rsx! {
        div { key: "{identity}", class: "metric-chart-dioxus",
            div {
                key: "{identity}",
                id: "metric-chart",
                role: "img",
                aria_label: "{summary}",
                LineChart {
                    series: vec![values],
                    labels: labels,
                    show_grid: true,
                    show_dotted_grid: false,
                    show_labels: true,
                    show_dots: true,
                    width: "100%",
                    height: "100%",
                    viewbox_width: 720,
                    viewbox_height: 400,
                    padding_top: 20,
                    padding_bottom: 44,
                    padding_left: 80,
                    padding_right: 168,
                    label_interpolation: {
                        let unit = unit.clone();
                        move |value: f32| axis_value_label(value, &unit)
                    },
                    point_label: move |point: LineChartPoint| point_label(point),
                    on_point_pointer_enter: move |point| hovered.set(Some(point)),
                    on_point_pointer_leave: move |point| {
                        if hovered() == Some(point) {
                            hovered.set(None);
                        }
                    },
                    on_point_focus: move |point| focused.set(Some(point)),
                    on_point_blur: move |point| {
                        if focused() == Some(point) {
                            focused.set(None);
                        }
                    },
                }
            }
            ul {
                id: "metric-view-entries",
                class: "visually-hidden",
                for sample in data.samples.iter() {
                    li { key: "{sample.timestamp}:{sample.value}", {reading_line(sample, &unit)} }
                }
            }
        }
    }
}

fn axis_value_label(value: f32, unit: &str) -> String {
    let number = format!("{value}");
    if unit.is_empty() {
        number
    } else {
        format!("{number} {unit}")
    }
}

fn chart_summary(samples: &[Sample], unit: &str) -> String {
    let count = samples.len();
    if unit.is_empty() {
        format!("{count} samples")
    } else {
        format!("{count} samples in {unit}")
    }
}

fn reading_line(sample: &Sample, unit: &str) -> String {
    if unit.is_empty() {
        format!("{} {}", sample.timestamp, sample.value)
    } else {
        format!("{} {} {}", sample.timestamp, sample.value, unit)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChartModel, axis_time_label, axis_time_labels, axis_value_label, csv_document, readout,
        utc_label,
    };

    fn sample(timestamp: &str, value: &str) -> crate::metrics::Sample {
        crate::metrics::Sample {
            timestamp: timestamp.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn axis_labels_include_the_unit() {
        assert_eq!(axis_value_label(4.0, "ms"), "4 ms");
        assert_eq!(axis_value_label(1.5, ""), "1.5");
    }

    #[test]
    fn axis_times_stay_compact_on_one_day() {
        assert_eq!(axis_time_label("2024-01-01T00:01:00Z", true), "00:01");
        assert_eq!(
            axis_time_label("2024-01-01T00:01:30.000000000Z", true),
            "00:01:30"
        );
        assert_eq!(
            axis_time_label("2024-01-02T00:01:00Z", false),
            "2024-01-02 00:01"
        );
        let labels = axis_time_labels(&[
            sample("2024-01-01T00:00:00Z", "0"),
            sample("2024-01-01T00:00:10Z", "0"),
            sample("2024-01-01T00:00:20Z", "1"),
            sample("2024-01-01T00:00:30Z", "1"),
            sample("2024-01-01T00:01:00Z", "0"),
            sample("2024-01-01T00:01:30Z", "0"),
            sample("2024-01-01T00:02:00Z", "1"),
            sample("2024-01-01T00:03:00Z", "0"),
        ]);
        assert_eq!(
            labels,
            ["00:00", "", "00:00:20", "", "00:01", "", "", "00:03"]
        );
    }

    #[test]
    fn utc_labels_are_readable_timestamps() {
        assert_eq!(utc_label("2024-01-01T00:01:00Z"), "2024-01-01 00:01 UTC");
        assert_eq!(
            utc_label("2024-01-01T00:01:30.000000000Z"),
            "2024-01-01 00:01:30 UTC"
        );
    }

    #[test]
    fn errors_are_not_replaced_by_the_empty_copy() {
        let failed = ChartModel {
            status: "Samples could not be read.".to_owned(),
            alert: "sensor offline".to_owned(),
            ..ChartModel::default()
        };
        let error = readout(false, "door", &failed, true, "");
        assert_eq!(error.phase, "error");
        assert_eq!(error.status, "Samples could not be read.");
        assert_eq!(error.alert, "sensor offline");
        let empty = readout(false, "door", &ChartModel::default(), true, "");
        assert_eq!(empty.phase, "empty");
        assert_eq!(empty.status, "No samples for this metric.");
        assert!(empty.alert.is_empty());
        let loading = readout(true, "door", &failed, true, "");
        assert_eq!(loading.phase, "loading");
        assert_eq!(loading.status, "Loading samples.");
        assert!(loading.alert.is_empty());
    }

    #[test]
    fn csv_uses_the_visible_samples() {
        let samples = [
            sample("2024-01-01T00:01:00Z", "1"),
            sample("2024-01-01T00:02:00Z", "4"),
        ];
        assert_eq!(
            csv_document(&samples),
            "timestamp,value\n2024-01-01T00:01:00Z,1\n2024-01-01T00:02:00Z,4\n"
        );
    }
}
