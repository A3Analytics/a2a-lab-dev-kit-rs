//! Metric reader. The sidebar lists metrics; choosing one loads its samples.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use std::cell::RefCell;

use crate::inventory::{Fault, Item, Primitive};
use crate::metrics::{self, Sample};
use crate::shell::http::Transport;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::shell::session::{calls_allowed, message_id, take_secret};

pub fn mount(page: &Page, parent: &web_sys::Element) {
    present::section(page, parent, "metric-view", "Metrics", |body| {
        let status = present::node(page, "p", Some("metric-view-status"));
        page::attr(&status, "role", "status");
        page::attr(&status, "aria-live", "polite");
        body.append_child(&status).ok();
        let alert = present::node(page, "div", Some("metric-view-alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        let browser = present::node(page, "div", Some("metric-browser"));
        page::attr(&browser, "class", "log-browser");
        let filters = present::node(page, "div", Some("metric-filters"));
        page::attr(&filters, "role", "group");
        page::attr(&filters, "aria-label", "Metric filters");
        let range_group = present::node(page, "div", None);
        page::attr(&range_group, "class", "log-filter-group");
        let range_label = present::node(page, "p", None);
        range_label.set_text_content(Some("Time range"));
        let start_label = present::node(page, "label", None);
        page::attr(&start_label, "for", "metric-range-start");
        start_label.set_text_content(Some("Start"));
        let start = present::node(page, "input", Some("metric-range-start"));
        page::attr(&start, "type", "datetime-local");
        page::attr(&start, "step", "1");
        let end_label = present::node(page, "label", None);
        page::attr(&end_label, "for", "metric-range-end");
        end_label.set_text_content(Some("End"));
        let end = present::node(page, "input", Some("metric-range-end"));
        page::attr(&end, "type", "datetime-local");
        page::attr(&end, "step", "1");
        range_group.append_child(&range_label).ok();
        range_group.append_child(&start_label).ok();
        range_group.append_child(&start).ok();
        range_group.append_child(&end_label).ok();
        range_group.append_child(&end).ok();
        filters.append_child(&range_group).ok();
        let source_group = present::node(page, "div", None);
        page::attr(&source_group, "class", "log-filter-group");
        let source_label = present::node(page, "p", None);
        source_label.set_text_content(Some("Metric"));
        let sources = present::node(page, "div", Some("metric-sources"));
        page::attr(&sources, "role", "radiogroup");
        page::attr(&sources, "aria-label", "Metrics");
        source_group.append_child(&source_label).ok();
        source_group.append_child(&sources).ok();
        filters.append_child(&source_group).ok();
        let results = present::node(page, "div", Some("metric-results"));
        page::attr(&results, "class", "log-results");
        let count = present::node(page, "p", Some("metric-filter-count"));
        page::attr(&count, "class", "log-count");
        results.append_child(&count).ok();
        let scroll = present::node(page, "div", None);
        page::attr(&scroll, "class", "log-scroll");
        let chips = present::node(page, "div", Some("metric-filter-chips"));
        page::attr(&chips, "class", "log-chips");
        page::attr(&chips, "hidden", "hidden");
        scroll.append_child(&chips).ok();
        let plot = present::node(page, "div", Some("metric-plot"));
        page::attr(&plot, "class", "metric-plot");
        page::attr(&plot, "hidden", "hidden");
        let canvas = present::node(page, "canvas", Some("metric-chart"));
        page::attr(&canvas, "role", "img");
        page::attr(&canvas, "aria-label", "Metric samples");
        plot.append_child(&canvas).ok();
        scroll.append_child(&plot).ok();
        let entries = present::node(page, "ul", Some("metric-view-entries"));
        page::attr(&entries, "class", "visually-hidden");
        scroll.append_child(&entries).ok();
        results.append_child(&scroll).ok();
        browser.append_child(&filters).ok();
        browser.append_child(&results).ok();
        body.append_child(&browser).ok();
    });
    fill_sources(page, &[]);
    page.set_data("metric-view-status", "data-gen", "0");
    page.set_text("metric-view-status", "Choose a metric.");
}

pub fn redraw(page: &Page) {
    if page.element("metric-chart").is_some() {
        paint(page);
    }
}

pub fn install(page: &Page, transport: &Transport) {
    TRANSPORT.with(|slot| *slot.borrow_mut() = Some(transport.clone()));
    on_sources(page, transport);
    on_range(page);
    on_chips(page);
}

pub fn catalog_changed(page: &Page) {
    let items = super::inventory::loaded(Primitive::Metrics);
    let previous = selected();
    fill_sources(page, &items);
    let next = if items.iter().any(|item| item.id == previous) {
        previous.clone()
    } else {
        items
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default()
    };
    choose(page, &next);
    if next.is_empty() {
        clear_samples(page);
        page.set_text("metric-view-status", "No metrics.");
        page.set_text("metric-view-alert", "");
        return;
    }
    if next != previous {
        reload(page);
    }
}

fn fill_sources(page: &Page, items: &[Item]) {
    let Some(list) = page.element("metric-sources") else {
        return;
    };
    page::clear_children(&list);
    if items.is_empty() {
        let button = present::node(page, "button", Some("metric-pick-0"));
        page::attr(&button, "type", "button");
        page::attr(&button, "class", "metric-source");
        page::attr(&button, "role", "radio");
        page::attr(&button, "aria-checked", "false");
        page::attr(&button, "aria-disabled", "true");
        button.set_text_content(Some("None loaded"));
        list.append_child(&button).ok();
        return;
    }
    let current = selected();
    for (index, item) in items.iter().enumerate() {
        let button = present::node(page, "button", Some(&format!("metric-pick-{index}")));
        page::attr(&button, "type", "button");
        page::attr(&button, "class", "metric-source");
        page::attr(&button, "role", "radio");
        page::attr(&button, "data-metric", &item.id);
        page::attr(
            &button,
            "aria-checked",
            if item.id == current { "true" } else { "false" },
        );
        let name = present::node(page, "span", None);
        page::attr(&name, "class", "metric-source-name");
        name.set_text_content(Some(if item.name.is_empty() {
            &item.id
        } else {
            &item.name
        }));
        let detail = present::node(page, "span", None);
        page::attr(&detail, "class", "metric-source-detail");
        detail.set_text_content(Some(&source_detail(item)));
        button.append_child(&name).ok();
        button.append_child(&detail).ok();
        list.append_child(&button).ok();
    }
}

fn source_detail(item: &Item) -> String {
    let unit = item
        .details
        .iter()
        .find(|(label, _)| *label == "Unit")
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    match (item.description.is_empty(), unit.is_empty()) {
        (true, true) => item.id.clone(),
        (true, false) => format!("{} · {unit}", item.id),
        (false, true) => format!("{} · {}", item.id, item.description),
        (false, false) => format!("{} · {unit} · {}", item.id, item.description),
    }
}

fn on_sources(page: &Page, transport: &Transport) {
    let Some(sources) = page.element("metric-sources") else {
        return;
    };
    let page_for_click = page.clone();
    let transport_for_click = transport.clone();
    let click = Closure::wrap(Box::new(move |event: Event| {
        let Some(id) = event
            .target()
            .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
            .and_then(|element| element.closest("button").ok().flatten())
            .and_then(|button| button.get_attribute("data-metric"))
        else {
            return;
        };
        if id.is_empty() || id == selected() {
            return;
        }
        choose(&page_for_click, &id);
        let page = page_for_click.clone();
        let transport = transport_for_click.clone();
        spawn_local(async move {
            load(&page, &transport).await;
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = sources.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
    click.forget();
    let page_for_key = page.clone();
    let transport_for_key = transport.clone();
    let key = Closure::wrap(Box::new(move |event: Event| {
        let Some(key_event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        let delta: isize = match key_event.key().as_str() {
            "ArrowDown" | "ArrowRight" => 1,
            "ArrowUp" | "ArrowLeft" => -1,
            _ => return,
        };
        key_event.prevent_default();
        let Some(id) = neighbor(delta) else {
            return;
        };
        if id == selected() {
            return;
        }
        choose(&page_for_key, &id);
        focus_selected(&page_for_key);
        let page = page_for_key.clone();
        let transport = transport_for_key.clone();
        spawn_local(async move {
            load(&page, &transport).await;
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = sources.add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref());
    key.forget();
}

fn on_chips(page: &Page) {
    let Some(chips) = page.element("metric-filter-chips") else {
        return;
    };
    let page = page.clone();
    let closure = Closure::wrap(Box::new(move |event: Event| {
        let Some(clear) = event
            .target()
            .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
            .and_then(|element| element.closest("button").ok().flatten())
            .and_then(|button| button.get_attribute("data-clear"))
        else {
            return;
        };
        if clear != "range" {
            return;
        }
        RANGE.with(|slot| *slot.borrow_mut() = metrics::QueryRange::Window);
        page.set_control("metric-range-start", "");
        page.set_control("metric-range-end", "");
        reload(&page);
    }) as Box<dyn FnMut(Event)>);
    let _ = chips.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn neighbor(delta: isize) -> Option<String> {
    let items = super::inventory::loaded(Primitive::Metrics);
    if items.is_empty() {
        return None;
    }
    let current = selected();
    let index = items
        .iter()
        .position(|item| item.id == current)
        .unwrap_or(0);
    let next = index as isize + delta;
    let next = next.rem_euclid(items.len() as isize) as usize;
    Some(items[next].id.clone())
}

fn choose(page: &Page, id: &str) {
    SELECTED.with(|slot| *slot.borrow_mut() = id.to_owned());
    let items = super::inventory::loaded(Primitive::Metrics);
    for (index, item) in items.iter().enumerate() {
        page.set_data(
            &format!("metric-pick-{index}"),
            "aria-checked",
            if item.id == id { "true" } else { "false" },
        );
    }
}

fn focus_selected(page: &Page) {
    let items = super::inventory::loaded(Primitive::Metrics);
    let current = selected();
    if let Some(index) = items.iter().position(|item| item.id == current) {
        page.focus(&format!("metric-pick-{index}"));
    }
}

fn reload(page: &Page) {
    let Some(transport) = transport() else {
        return;
    };
    let page = page.clone();
    spawn_local(async move {
        load(&page, &transport).await;
    });
}

async fn load(page: &Page, transport: &Transport) {
    if !calls_allowed(page) {
        return;
    }
    let metric = selected();
    if metric.is_empty() {
        page.set_text("metric-view-alert", "Choose a metric.");
        return;
    }
    let range = selected_range();
    let (start, end) = match &range {
        metrics::QueryRange::Window => ("", ""),
        metrics::QueryRange::Bounded { start, end } => (start.as_str(), end.as_str()),
    };
    let Ok(secret) = take_secret(page, &[metric.as_str(), start, end]) else {
        return;
    };
    let command = metrics::command(&metric, None, &range);
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        show_alert(page, "The metric query could not be built.");
        return;
    };
    let generation = bump(page);
    page.set_text("metric-view-status", "Loading samples.");
    page.set_text("metric-view-alert", "");
    page.set_data("metric-view", "aria-busy", "true");
    let result = transport.send(&body, secret.as_deref()).await;
    if page.data("metric-view-status", "data-gen") != generation.to_string() {
        return;
    }
    page.set_data("metric-view", "aria-busy", "false");
    match result {
        Err(()) => show_alert(page, "The browser could not reach this origin."),
        Ok(reply) => match metrics::interpret(&reply, secret.as_deref()) {
            Ok(read) => show_samples(page, read.samples),
            Err(fault) => show_alert(page, &fault_message(&fault)),
        },
    }
}

fn show_samples(page: &Page, samples: Vec<Sample>) {
    let empty = samples.is_empty();
    remember(&samples);
    paint(page);
    if empty {
        page.set_text("metric-view-status", "No samples for this metric.");
    } else {
        page.set_text("metric-view-status", "");
    }
    page.set_text("metric-view-alert", "");
}

fn paint(page: &Page) {
    let samples = remembered();
    render_chips(page);
    render(page, &samples);
    page.set_text("metric-filter-count", &count_text(samples.len()));
}

fn count_text(total: usize) -> String {
    match total {
        1 => "1 sample".to_owned(),
        count => format!("{count} samples"),
    }
}

fn render(page: &Page, samples: &[Sample]) {
    let unit = selected_unit();
    render_readings(page, samples, &unit);
    render_plot(page, samples, &unit);
}

fn render_readings(page: &Page, samples: &[Sample], unit: &str) {
    let Some(list) = page.element("metric-view-entries") else {
        return;
    };
    page::clear_children(&list);
    for sample in samples {
        let item = present::node(page, "li", None);
        let reading = if unit.is_empty() {
            format!("{} {}", sample.timestamp, sample.value)
        } else {
            format!("{} {} {unit}", sample.timestamp, sample.value)
        };
        item.set_text_content(Some(&reading));
        list.append_child(&item).ok();
    }
}

fn render_plot(page: &Page, samples: &[Sample], unit: &str) {
    let Some(canvas) = page.element("metric-chart") else {
        return;
    };
    let Some(canvas) = canvas.dyn_into::<web_sys::HtmlCanvasElement>().ok() else {
        return;
    };
    let summary = if unit.is_empty() {
        format!("{} samples", samples.len())
    } else {
        format!("{} samples in {unit}", samples.len())
    };
    page::attr(&canvas, "aria-label", &summary);
    if samples.is_empty() {
        page.set_data("metric-plot", "hidden", "hidden");
    } else {
        page.remove_attr("metric-plot", "hidden");
    }
    if let Err(error) = render_metric_chart(&canvas, &metrics::chart_payload(samples), unit) {
        let message = error
            .as_string()
            .unwrap_or_else(|| "The chart could not be drawn.".to_owned());
        page.set_text("metric-view-alert", &message);
    }
}

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, catch, js_name = renderMetricChart)]
    fn render_metric_chart(
        canvas: &web_sys::HtmlCanvasElement,
        samples_json: &str,
        unit: &str,
    ) -> Result<(), wasm_bindgen::JsValue>;
}

fn render_chips(page: &Page) {
    let Some(chips) = page.element("metric-filter-chips") else {
        return;
    };
    page::clear_children(&chips);
    let range = match selected_range() {
        metrics::QueryRange::Window => None,
        metrics::QueryRange::Bounded { start, end } => Some((start, end)),
    };
    let Some((start, end)) = range else {
        page.set_data("metric-filter-chips", "hidden", "hidden");
        return;
    };
    page.remove_attr("metric-filter-chips", "hidden");
    let heading = present::node(page, "span", None);
    page::attr(&heading, "class", "log-chips-label");
    heading.set_text_content(Some("Filters"));
    chips.append_child(&heading).ok();
    chip(
        page,
        &chips,
        "Time",
        &format!("{} – {}", clock_label(&start), clock_label(&end)),
        "range",
    );
}

fn chip(page: &Page, parent: &web_sys::Element, label: &str, value: &str, clear: &str) {
    let chip = present::node(page, "span", None);
    page::attr(&chip, "class", "log-meta");
    let name = present::node(page, "span", None);
    page::attr(&name, "class", "log-meta-label");
    name.set_text_content(Some(&format!("{label}:")));
    let text = present::node(page, "span", None);
    text.set_text_content(Some(value));
    let button = present::node(page, "button", None);
    page::attr(&button, "type", "button");
    page::attr(&button, "data-clear", clear);
    page::attr(&button, "aria-label", &format!("Remove {label} {value}"));
    button.set_text_content(Some("×"));
    chip.append_child(&name).ok();
    chip.append_child(&text).ok();
    chip.append_child(&button).ok();
    parent.append_child(&chip).ok();
}

fn clock_label(timestamp: &str) -> String {
    timestamp.replace(".000000000Z", "Z")
}

fn selected_unit() -> String {
    let id = selected();
    super::inventory::loaded(Primitive::Metrics)
        .into_iter()
        .find(|item| item.id == id)
        .and_then(|item| {
            item.details
                .into_iter()
                .find(|(label, _)| *label == "Unit")
                .map(|(_, value)| value)
        })
        .unwrap_or_default()
}

fn on_range(page: &Page) {
    for id in ["metric-range-start", "metric-range-end"] {
        let Some(input) = page.element(id) else {
            continue;
        };
        let page = page.clone();
        let change = Closure::wrap(Box::new(move |_event: Event| {
            commit_range(&page);
        }) as Box<dyn FnMut(Event)>);
        let _ = input.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        change.forget();
    }
}

fn commit_range(page: &Page) {
    let start = page.control_value("metric-range-start");
    let end = page.control_value("metric-range-end");
    match metrics::query_range(&start, &end) {
        Ok(range) => {
            page.set_text("metric-view-alert", "");
            let changed = RANGE.with(|slot| {
                let mut current = slot.borrow_mut();
                if *current == range {
                    return false;
                }
                *current = range;
                true
            });
            if changed {
                reload(page);
            }
        }
        Err(message) => page.set_text("metric-view-alert", message),
    }
}

fn selected_range() -> metrics::QueryRange {
    RANGE.with(|slot| slot.borrow().clone())
}

fn clear_samples(page: &Page) {
    remember(&[]);
    paint(page);
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The metric result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

fn show_alert(page: &Page, message: &str) {
    page.set_data("metric-view", "aria-busy", "false");
    page.set_text("metric-view-status", "Samples could not be read.");
    page.set_text("metric-view-alert", message);
}

fn bump(page: &Page) -> u32 {
    let current = page
        .data("metric-view-status", "data-gen")
        .parse::<u32>()
        .unwrap_or(0);
    let next = current.wrapping_add(1);
    page.set_data("metric-view-status", "data-gen", &next.to_string());
    next
}

fn transport() -> Option<Transport> {
    TRANSPORT.with(|slot| slot.borrow().clone())
}

fn selected() -> String {
    SELECTED.with(|slot| slot.borrow().clone())
}

std::thread_local! {
    static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) };
    static SAMPLES: RefCell<Vec<Sample>> = const { RefCell::new(Vec::new()) };
    static SELECTED: RefCell<String> = const { RefCell::new(String::new()) };
    static RANGE: RefCell<metrics::QueryRange> = const { RefCell::new(metrics::QueryRange::Window) };
}

fn remember(samples: &[Sample]) {
    SAMPLES.with(|slot| *slot.borrow_mut() = samples.to_vec());
}

fn remembered() -> Vec<Sample> {
    SAMPLES.with(|slot| slot.borrow().clone())
}
