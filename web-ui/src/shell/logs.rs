//! Log reader. Source toggles choose which sources to load.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use std::cell::RefCell;

use crate::inventory::{self, Fault, Primitive};
use crate::logs::{self, Record};
use crate::shell::http::Transport;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::shell::session::{calls_allowed, message_id, take_secret};

pub fn mount(page: &Page, parent: &web_sys::Element) {
    present::section(page, parent, "log-view", "Logs", |body| {
        let status = present::node(page, "p", Some("log-view-status"));
        page::attr(&status, "role", "status");
        page::attr(&status, "aria-live", "polite");
        body.append_child(&status).ok();
        let alert = present::node(page, "div", Some("log-view-alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        let browser = present::node(page, "div", Some("log-browser"));
        page::attr(&browser, "class", "log-browser");
        let filters = present::node(page, "div", Some("log-filters"));
        page::attr(&filters, "role", "group");
        page::attr(&filters, "aria-label", "Log filters");
        let source_group = present::node(page, "div", Some("log-view-sources"));
        page::attr(&source_group, "class", "log-filter-group");
        let source_label = present::node(page, "p", None);
        source_label.set_text_content(Some("Source"));
        source_group.append_child(&source_label).ok();
        filters.append_child(&source_group).ok();
        let search_group = present::node(page, "div", None);
        page::attr(&search_group, "class", "log-filter-group");
        let search_label = present::node(page, "label", None);
        page::attr(&search_label, "for", "log-filter-search");
        search_label.set_text_content(Some("Search"));
        let search = present::node(page, "input", Some("log-filter-search"));
        page::attr(&search, "type", "search");
        page::attr(&search, "placeholder", "Message");
        search_group.append_child(&search_label).ok();
        search_group.append_child(&search).ok();
        filters.append_child(&search_group).ok();
        let range_group = present::node(page, "div", None);
        page::attr(&range_group, "class", "log-filter-group");
        let range_label = present::node(page, "p", None);
        range_label.set_text_content(Some("Date range"));
        let start_label = present::node(page, "label", None);
        page::attr(&start_label, "for", "log-range-start");
        start_label.set_text_content(Some("Start"));
        let start = present::node(page, "input", Some("log-range-start"));
        page::attr(&start, "type", "date");
        let end_label = present::node(page, "label", None);
        page::attr(&end_label, "for", "log-range-end");
        end_label.set_text_content(Some("End"));
        let end = present::node(page, "input", Some("log-range-end"));
        page::attr(&end, "type", "date");
        range_group.append_child(&range_label).ok();
        range_group.append_child(&start_label).ok();
        range_group.append_child(&start).ok();
        range_group.append_child(&end_label).ok();
        range_group.append_child(&end).ok();
        filters.append_child(&range_group).ok();
        let severity_group = present::node(page, "div", None);
        page::attr(&severity_group, "class", "log-filter-group");
        let severity_label = present::node(page, "p", None);
        severity_label.set_text_content(Some("Severity"));
        severity_group.append_child(&severity_label).ok();
        facet(page, &severity_group, "log-filter-all", "", "All");
        for (id, level, label) in [
            ("log-filter-trace", "trace", "Trace"),
            ("log-filter-debug", "debug", "Debug"),
            ("log-filter-info", "info", "Info"),
            ("log-filter-warn", "warn", "Warn"),
            ("log-filter-error", "error", "Error"),
        ] {
            facet(page, &severity_group, id, level, label);
        }
        filters.append_child(&severity_group).ok();
        let results = present::node(page, "div", Some("log-results"));
        page::attr(&results, "class", "log-results");
        let count = present::node(page, "p", Some("log-filter-count"));
        page::attr(&count, "class", "log-count");
        results.append_child(&count).ok();
        let scroll = present::node(page, "div", None);
        page::attr(&scroll, "class", "log-scroll");
        let chips = present::node(page, "div", Some("log-filter-chips"));
        page::attr(&chips, "class", "log-chips");
        page::attr(&chips, "hidden", "hidden");
        scroll.append_child(&chips).ok();
        let empty = present::node(page, "p", Some("log-filter-empty"));
        empty.set_text_content(Some("No logs match these filters."));
        page::attr(&empty, "hidden", "hidden");
        scroll.append_child(&empty).ok();
        let entries = present::node(page, "ul", Some("log-view-entries"));
        page::attr(&entries, "class", "log-cards");
        scroll.append_child(&entries).ok();
        results.append_child(&scroll).ok();
        browser.append_child(&filters).ok();
        browser.append_child(&results).ok();
        body.append_child(&browser).ok();
    });
    fill_sources(page, &[]);
    page.set_data("log-view-status", "data-gen", "0");
    page.set_text("log-view-status", "Choose a log source.");
}

pub fn install(page: &Page, transport: &Transport) {
    TRANSPORT.with(|slot| *slot.borrow_mut() = Some(transport.clone()));
    on_range(page);
    on_filters(page);
}

pub fn sources_changed(page: &Page) {
    let items = super::inventory::loaded(Primitive::Logs);
    let ids = items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let catalog_changed = CATALOG.with(|slot| {
        let mut catalog = slot.borrow_mut();
        if *catalog == ids {
            return false;
        }
        *catalog = ids;
        true
    });
    SELECTED.with(|slot| {
        slot.borrow_mut()
            .retain(|id| items.iter().any(|item| item.id == *id));
    });
    fill_sources(page, &items);
    if items.is_empty() {
        clear_entries(page);
        page.set_text("log-view-status", "No log sources.");
        page.set_text("log-view-alert", "");
        return;
    }
    if catalog_changed {
        reload(page);
    }
}

fn fill_sources(page: &Page, items: &[inventory::Item]) {
    let Some(group) = page.element("log-view-sources") else {
        return;
    };
    let label = group.first_child();
    page::clear_children(&group);
    if let Some(label) = label {
        group.append_child(&label).ok();
    }
    let selected = selected_ids();
    source_facet(
        page,
        &group,
        "log-source-all",
        "",
        if items.is_empty() {
            "None loaded"
        } else {
            "All"
        },
        items.is_empty() || selected.is_empty(),
    );
    for (index, item) in items.iter().enumerate() {
        source_facet(
            page,
            &group,
            &format!("log-source-{index}"),
            &item.id,
            &source_label(item),
            selected.iter().any(|id| id == &item.id),
        );
    }
}

fn source_text(source_id: &str) -> String {
    super::inventory::loaded(Primitive::Logs)
        .into_iter()
        .find(|item| item.id == source_id)
        .map(|item| {
            if item.name.is_empty() {
                item.id
            } else {
                item.name
            }
        })
        .unwrap_or_else(|| source_id.to_owned())
}

fn source_label(item: &inventory::Item) -> String {
    let identity = if item.name.is_empty() || item.name == item.id {
        item.id.clone()
    } else {
        format!("{} ({})", item.name, item.id)
    };
    if item.description.is_empty() {
        identity
    } else {
        format!("{identity} — {}", item.description)
    }
}

async fn load(page: &Page, transport: &Transport) {
    if !calls_allowed(page) {
        return;
    }
    let sources = active_sources();
    if sources.is_empty() {
        page.set_text("log-view-alert", "Choose a log source.");
        return;
    }
    let range = selected_range();
    let (start, end) = match &range {
        logs::QueryRange::Window => ("", ""),
        logs::QueryRange::Bounded { start, end } => (start.as_str(), end.as_str()),
    };
    let mut visible = vec![start.to_owned(), end.to_owned()];
    visible.extend(sources.iter().cloned());
    let visible = visible.iter().map(String::as_str).collect::<Vec<_>>();
    let Ok(secret) = take_secret(page, &visible) else {
        return;
    };
    let generation = bump(page);
    page.set_text("log-view-status", "Loading logs.");
    page.set_text("log-view-alert", "");
    page.set_data("log-view", "aria-busy", "true");
    let mut records = Vec::new();
    for source in &sources {
        let command = logs::command(source, None, &range);
        let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
            show_alert(page, "The log query could not be built.");
            return;
        };
        let result = transport.send(&body, secret.as_deref()).await;
        if page.data("log-view-status", "data-gen") != generation.to_string() {
            return;
        }
        match result {
            Err(()) => {
                show_alert(page, "The browser could not reach this origin.");
                return;
            }
            Ok(reply) => match logs::interpret(&reply, secret.as_deref()) {
                Ok(read) => records.extend(read.records),
                Err(fault) => {
                    show_alert(page, &fault_message(&fault));
                    return;
                }
            },
        }
    }
    page.set_data("log-view", "aria-busy", "false");
    records.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    show_records(page, records);
}

fn show_records(page: &Page, records: Vec<Record>) {
    let empty = records.is_empty();
    remember(&records);
    paint(page);
    if empty {
        page.set_text("log-view-status", "No logs for this source.");
    } else {
        page.set_text("log-view-status", "");
    }
    page.set_text("log-view-alert", "");
}

fn paint(page: &Page) {
    let records = remembered();
    let shown: Vec<Record> = records
        .iter()
        .filter(|record| matches_filter(record))
        .cloned()
        .collect();
    sync_facets(page);
    sync_sources(page);
    render_chips(page);
    render(page, &shown);
    page.set_text("log-filter-count", &count_text(shown.len(), records.len()));
    if records.is_empty() || !shown.is_empty() {
        page.set_data("log-filter-empty", "hidden", "hidden");
    } else {
        page.remove_attr("log-filter-empty", "hidden");
    }
}

fn count_text(shown: usize, total: usize) -> String {
    if total == 0 || shown == total {
        match total {
            1 => "1 log".to_owned(),
            count => format!("{count} logs"),
        }
    } else {
        format!("{shown} of {total} logs")
    }
}

fn matches_filter(record: &Record) -> bool {
    let (query, levels) = FILTER.with(|slot| {
        let filter = slot.borrow();
        (filter.query.clone(), filter.levels.clone())
    });
    if !levels.is_empty() && !levels.iter().any(|level| level == &record.level) {
        return false;
    }
    let query = query.trim().to_lowercase();
    query.is_empty()
        || record.message.to_lowercase().contains(&query)
        || record.attributes.to_lowercase().contains(&query)
}

fn render(page: &Page, records: &[Record]) {
    let Some(list) = page.element("log-view-entries") else {
        return;
    };
    page::clear_children(&list);
    for record in records {
        let item = present::node(page, "li", None);
        let card = present::node(page, "div", None);
        page::attr(&card, "class", "log-card");
        let identity = present::node(page, "div", None);
        page::attr(&identity, "class", "log-card-identity");
        let time = present::node(page, "time", None);
        page::attr(&time, "class", "log-card-time");
        page::attr(&time, "datetime", &record.timestamp);
        time.set_text_content(Some(&record.timestamp));
        let title = present::node(page, "p", None);
        page::attr(&title, "class", "log-card-title");
        title.set_text_content(Some(&record.message));
        identity.append_child(&time).ok();
        identity.append_child(&title).ok();
        let tags = present::node(page, "div", None);
        page::attr(&tags, "class", "log-card-tags");
        meta(page, &tags, "Source", &source_text(&record.source_id), None);
        meta(page, &tags, "Severity", &record.level, None);
        if !record.attributes.is_empty() {
            meta(page, &tags, "Attributes", &record.attributes, None);
        }
        card.append_child(&identity).ok();
        card.append_child(&tags).ok();
        item.append_child(&card).ok();
        list.append_child(&item).ok();
    }
}

fn render_chips(page: &Page) {
    let Some(chips) = page.element("log-filter-chips") else {
        return;
    };
    page::clear_children(&chips);
    let (query, levels) = FILTER.with(|slot| {
        let filter = slot.borrow();
        (filter.query.clone(), filter.levels.clone())
    });
    let sources = selected_sources();
    let range = match selected_range() {
        logs::QueryRange::Window => None,
        logs::QueryRange::Bounded { .. } => Some((
            page.control_value("log-range-start"),
            page.control_value("log-range-end"),
        )),
    };
    if query.is_empty() && levels.is_empty() && sources.is_empty() && range.is_none() {
        page.set_data("log-filter-chips", "hidden", "hidden");
        return;
    }
    page.remove_attr("log-filter-chips", "hidden");
    let heading = present::node(page, "span", None);
    page::attr(&heading, "class", "log-chips-label");
    heading.set_text_content(Some("Filters"));
    chips.append_child(&heading).ok();
    for (id, source) in sources {
        meta(
            page,
            &chips,
            "Source",
            &source,
            Some(&format!("source:{id}")),
        );
    }
    if !query.is_empty() {
        meta(page, &chips, "Search", &query, Some("search"));
    }
    if let Some((start, end)) = range {
        meta(
            page,
            &chips,
            "Date",
            &format!("{start} – {end}"),
            Some("range"),
        );
    }
    for level in levels {
        meta(page, &chips, "Severity", &level, Some(&level));
    }
}

fn meta(page: &Page, parent: &web_sys::Element, label: &str, value: &str, clear: Option<&str>) {
    let chip = present::node(page, "span", None);
    page::attr(&chip, "class", "log-meta");
    let name = present::node(page, "span", None);
    page::attr(&name, "class", "log-meta-label");
    name.set_text_content(Some(&format!("{label}:")));
    let text = present::node(page, "span", None);
    text.set_text_content(Some(value));
    chip.append_child(&name).ok();
    chip.append_child(&text).ok();
    if let Some(clear) = clear {
        let button = present::node(page, "button", None);
        page::attr(&button, "type", "button");
        page::attr(&button, "data-clear", clear);
        page::attr(&button, "aria-label", &format!("Remove {label} {value}"));
        button.set_text_content(Some("×"));
        chip.append_child(&button).ok();
    }
    parent.append_child(&chip).ok();
}

fn clear_entries(page: &Page) {
    remember(&[]);
    reset_filters(page);
    paint(page);
}

fn facet(page: &Page, parent: &web_sys::Element, id: &str, level: &str, label: &str) {
    let button = present::node(page, "button", Some(id));
    page::attr(&button, "type", "button");
    page::attr(&button, "class", "log-facet");
    page::attr(&button, "data-level", level);
    page::attr(
        &button,
        "aria-pressed",
        if level.is_empty() { "true" } else { "false" },
    );
    let mark = present::node(page, "span", None);
    page::attr(&mark, "class", "log-facet-mark");
    page::attr(&mark, "aria-hidden", "true");
    let text = present::node(page, "span", None);
    text.set_text_content(Some(label));
    button.append_child(&mark).ok();
    button.append_child(&text).ok();
    parent.append_child(&button).ok();
}

fn on_filters(page: &Page) {
    if let Some(search) = page.element("log-filter-search") {
        let committed = page.clone();
        let change = Closure::wrap(Box::new(move |_event: Event| {
            commit_search(&committed);
        }) as Box<dyn FnMut(Event)>);
        let _ = search.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        change.forget();
        let committed = page.clone();
        let key = Closure::wrap(Box::new(move |event: Event| {
            let Some(key_event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
                return;
            };
            if key_event.key() != "Enter" {
                return;
            }
            key_event.prevent_default();
            commit_search(&committed);
        }) as Box<dyn FnMut(Event)>);
        let _ = search.add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref());
        key.forget();
    }
    if let Some(filters) = page.element("log-filters") {
        let page = page.clone();
        let closure = Closure::wrap(Box::new(move |event: Event| {
            let Some(button) = event
                .target()
                .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
                .and_then(|element| element.closest("button").ok().flatten())
            else {
                return;
            };
            if let Some(source) = button.get_attribute("data-source") {
                toggle_source(&page, &source);
                return;
            }
            let Some(level) = button.get_attribute("data-level") else {
                return;
            };
            FILTER.with(|slot| {
                let mut filter = slot.borrow_mut();
                if level.is_empty() {
                    filter.levels.clear();
                } else if let Some(index) = filter.levels.iter().position(|item| item == &level) {
                    filter.levels.remove(index);
                } else {
                    filter.levels.push(level);
                }
            });
            paint(&page);
        }) as Box<dyn FnMut(Event)>);
        let _ = filters.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }
    let Some(chips) = page.element("log-filter-chips") else {
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
        match clear.as_str() {
            "search" => {
                FILTER.with(|slot| slot.borrow_mut().query.clear());
                page.set_control("log-filter-search", "");
                paint(&page);
            }
            clear if clear.starts_with("source:") => {
                let id = clear.trim_start_matches("source:").to_owned();
                SELECTED.with(|slot| slot.borrow_mut().retain(|source| source != &id));
                sync_sources(&page);
                reload(&page);
            }
            "range" => {
                RANGE.with(|slot| *slot.borrow_mut() = logs::QueryRange::Window);
                page.set_control("log-range-start", "");
                page.set_control("log-range-end", "");
                reload(&page);
            }
            level => {
                let level = level.to_owned();
                FILTER.with(|slot| slot.borrow_mut().levels.retain(|item| item != &level));
                paint(&page);
            }
        }
    }) as Box<dyn FnMut(Event)>);
    let _ = chips.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn on_range(page: &Page) {
    for id in ["log-range-start", "log-range-end"] {
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
    let start = page.control_value("log-range-start");
    let end = page.control_value("log-range-end");
    match logs::date_range(&start, &end) {
        Ok(range) => {
            page.set_text("log-view-alert", "");
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
        Err(message) => page.set_text("log-view-alert", message),
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

fn selected_range() -> logs::QueryRange {
    RANGE.with(|slot| slot.borrow().clone())
}

fn selected_sources() -> Vec<(String, String)> {
    selected_ids()
        .into_iter()
        .map(|id| {
            let label = super::inventory::loaded(Primitive::Logs)
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| source_label(&item))
                .unwrap_or_else(|| id.clone());
            (id, label)
        })
        .collect()
}

fn selected_ids() -> Vec<String> {
    SELECTED.with(|slot| slot.borrow().clone())
}

fn active_sources() -> Vec<String> {
    let items = super::inventory::loaded(Primitive::Logs);
    let selected = selected_ids();
    if selected.is_empty() {
        items.into_iter().map(|item| item.id).collect()
    } else {
        selected
            .into_iter()
            .filter(|id| items.iter().any(|item| item.id == *id))
            .collect()
    }
}

fn toggle_source(page: &Page, source: &str) {
    let changed = SELECTED.with(|slot| {
        let mut selected = slot.borrow_mut();
        if source.is_empty() {
            if selected.is_empty() {
                return false;
            }
            selected.clear();
        } else if let Some(index) = selected.iter().position(|id| id == source) {
            selected.remove(index);
        } else {
            selected.push(source.to_owned());
        }
        true
    });
    if changed {
        sync_sources(page);
        reload(page);
    }
}

fn sync_sources(page: &Page) {
    let selected = selected_ids();
    let items = super::inventory::loaded(Primitive::Logs);
    page.set_data(
        "log-source-all",
        "aria-pressed",
        if items.is_empty() || selected.is_empty() {
            "true"
        } else {
            "false"
        },
    );
    for (index, item) in items.iter().enumerate() {
        page.set_data(
            &format!("log-source-{index}"),
            "aria-pressed",
            if selected.iter().any(|id| id == &item.id) {
                "true"
            } else {
                "false"
            },
        );
    }
}

fn source_facet(
    page: &Page,
    parent: &web_sys::Element,
    id: &str,
    source: &str,
    label: &str,
    pressed: bool,
) {
    let button = present::node(page, "button", Some(id));
    page::attr(&button, "type", "button");
    page::attr(&button, "class", "log-facet");
    page::attr(&button, "data-source", source);
    page::attr(&button, "aria-pressed", if pressed { "true" } else { "false" });
    let mark = present::node(page, "span", None);
    page::attr(&mark, "class", "log-facet-mark");
    page::attr(&mark, "aria-hidden", "true");
    let text = present::node(page, "span", None);
    text.set_text_content(Some(label));
    button.append_child(&mark).ok();
    button.append_child(&text).ok();
    parent.append_child(&button).ok();
}

fn commit_search(page: &Page) {
    let query = page.control_value("log-filter-search").trim().to_owned();
    let changed = FILTER.with(|slot| {
        let mut filter = slot.borrow_mut();
        if filter.query == query {
            return false;
        }
        filter.query = query;
        true
    });
    if changed {
        paint(page);
    }
}

fn sync_facets(page: &Page) {
    let levels = FILTER.with(|slot| slot.borrow().levels.clone());
    let all = if levels.is_empty() { "true" } else { "false" };
    page.set_data("log-filter-all", "aria-pressed", all);
    for (id, level) in [
        ("log-filter-trace", "trace"),
        ("log-filter-debug", "debug"),
        ("log-filter-info", "info"),
        ("log-filter-warn", "warn"),
        ("log-filter-error", "error"),
    ] {
        let pressed = if levels.iter().any(|item| item == level) {
            "true"
        } else {
            "false"
        };
        page.set_data(id, "aria-pressed", pressed);
    }
}

fn reset_filters(page: &Page) {
    FILTER.with(|slot| {
        let mut filter = slot.borrow_mut();
        filter.query.clear();
        filter.levels.clear();
    });
    page.set_control("log-filter-search", "");
    sync_facets(page);
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The log result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

fn show_alert(page: &Page, message: &str) {
    page.set_data("log-view", "aria-busy", "false");
    page.set_text("log-view-status", "Logs could not be read.");
    page.set_text("log-view-alert", message);
}

fn bump(page: &Page) -> u32 {
    let current = page
        .data("log-view-status", "data-gen")
        .parse::<u32>()
        .unwrap_or(0);
    let next = current.wrapping_add(1);
    page.set_data("log-view-status", "data-gen", &next.to_string());
    next
}

fn transport() -> Option<Transport> {
    TRANSPORT.with(|slot| slot.borrow().clone())
}

std::thread_local! {
    static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) };
    static RECORDS: RefCell<Vec<Record>> = const { RefCell::new(Vec::new()) };
    static FILTER: RefCell<LogFilter> = const { RefCell::new(LogFilter::new()) };
    static RANGE: RefCell<logs::QueryRange> = const { RefCell::new(logs::QueryRange::Window) };
    static SELECTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CATALOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct LogFilter {
    query: String,
    levels: Vec<String>,
}

impl LogFilter {
    const fn new() -> Self {
        Self {
            query: String::new(),
            levels: Vec::new(),
        }
    }
}

fn remember(records: &[Record]) {
    RECORDS.with(|slot| *slot.borrow_mut() = records.to_vec());
}

fn remembered() -> Vec<Record> {
    RECORDS.with(|slot| slot.borrow().clone())
}
