//! Metric reader. The sidebar lists metrics; choosing one loads its samples.

use std::cell::RefCell;

use wasm_bindgen::JsCast;

use crate::inventory::{Fault, Item, Primitive};
use crate::metrics::{self, Sample};
use crate::shell::http::Transport;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

pub fn redraw() {}

pub fn catalog_changed() {
    let items = super::inventory::loaded(Primitive::Metrics);
    let previous = selected();
    super::metric_chart::set_picks(items.clone());
    let next = if items.iter().any(|item| item.id == previous) {
        previous.clone()
    } else {
        items
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default()
    };
    choose(&next);
    if next.is_empty() {
        remember(&[]);
        super::metric_chart::set_query(super::metric_chart::MetricQuery::default());
        return;
    }
    if next != previous || remembered().is_empty() {
        super::metric_chart::set_query(super::metric_chart::MetricQuery {
            metric: next,
            range: selected_range(),
        });
    }
}

pub(super) fn source_detail(item: &Item) -> String {
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

pub(super) fn apply_pick(id: String) {
    if id.is_empty() || id == selected() {
        return;
    }
    choose(&id);
}

pub(super) fn select_neighbor(delta: isize) {
    let Some(id) = neighbor(delta) else {
        return;
    };
    if id == selected() {
        return;
    }
    choose(&id);
    focus_selected(&id);
    super::metric_chart::set_query(super::metric_chart::MetricQuery {
        metric: id,
        range: selected_range(),
    });
}

pub(super) fn remember_range(range: metrics::QueryRange) {
    RANGE.with(|slot| *slot.borrow_mut() = range);
}

pub(super) async fn load_chart(
    query: super::metric_chart::MetricQuery,
) -> super::metric_chart::ChartModel {
    let Some(transport) = session::transport() else {
        return super::metric_chart::ChartModel::default();
    };
    load(&transport, &query).await
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

fn choose(id: &str) {
    SELECTED.with(|slot| *slot.borrow_mut() = id.to_owned());
    super::metric_chart::set_selected(id.to_owned());
}

fn focus_selected(id: &str) {
    let items = super::inventory::loaded(Primitive::Metrics);
    let Some(index) = items.iter().position(|item| item.id == id) else {
        return;
    };
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    for slot in 0..items.len() {
        let Some(element) = document.get_element_by_id(&format!("metric-pick-{slot}")) else {
            continue;
        };
        let tab = if slot == index { "0" } else { "-1" };
        let _ = element.set_attribute("tabindex", tab);
    }
    let Some(element) = document.get_element_by_id(&format!("metric-pick-{index}")) else {
        return;
    };
    if let Ok(html) = element.dyn_into::<web_sys::HtmlElement>() {
        let _ = html.focus();
    }
}

async fn load(
    transport: &Transport,
    query: &super::metric_chart::MetricQuery,
) -> super::metric_chart::ChartModel {
    if !calls_allowed() {
        return super::metric_chart::ChartModel::default();
    }
    let metric = query.metric.clone();
    if metric.is_empty() {
        let status = if super::inventory::loaded(Primitive::Metrics).is_empty() {
            "No metrics."
        } else {
            "Choose a metric."
        };
        return chart_status(status, "");
    }
    let range = query.range.clone();
    let (start, end) = match &range {
        metrics::QueryRange::Window => ("", ""),
        metrics::QueryRange::Bounded { start, end } => (start.as_str(), end.as_str()),
    };
    let Ok(secret) = take_secret(&[metric.as_str(), start, end]) else {
        return super::metric_chart::ChartModel::default();
    };
    let command = metrics::command(&metric, None, &range);
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        return chart_status(
            "Samples could not be read.",
            "The metric query could not be built.",
        );
    };
    let result = transport.send(&body, secret.as_deref()).await;
    match result {
        Err(()) => chart_status(
            "Samples could not be read.",
            "The browser could not reach this origin.",
        ),
        Ok(reply) => match metrics::interpret(&reply, secret.as_deref()) {
            Ok(read) => show_samples(read.samples),
            Err(fault) => chart_status("Samples could not be read.", &fault_message(&fault)),
        },
    }
}

fn show_samples(samples: Vec<Sample>) -> super::metric_chart::ChartModel {
    let empty = samples.is_empty();
    remember(&samples);
    super::metric_chart::ChartModel {
        unit: selected_unit(),
        status: if empty {
            "No samples for this metric.".to_owned()
        } else {
            String::new()
        },
        alert: String::new(),
        samples,
    }
}

fn chart_status(status: &str, alert: &str) -> super::metric_chart::ChartModel {
    super::metric_chart::ChartModel {
        samples: Vec::new(),
        unit: String::new(),
        status: status.to_owned(),
        alert: alert.to_owned(),
    }
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

fn selected_range() -> metrics::QueryRange {
    RANGE.with(|slot| slot.borrow().clone())
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The metric result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

fn selected() -> String {
    SELECTED.with(|slot| slot.borrow().clone())
}

std::thread_local! {
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

pub(super) fn cached_samples() -> Vec<Sample> {
    remembered()
}

pub(super) fn cached_unit() -> String {
    selected_unit()
}
