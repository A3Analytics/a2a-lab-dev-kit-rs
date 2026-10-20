//! Image history reader. The sidebar toggles sources; the main column lists their frames.

use std::cell::RefCell;

use dioxus::core::{Runtime, RuntimeGuard, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::images::{
    self, Descriptor, Frame, InspectTicket, Page, Read, SourcePhase, Thumb, ThumbTicket, Ticket,
};
use crate::inventory::{Fault, Item, Primitive};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

#[derive(Clone, PartialEq)]
pub struct ImageUi {
    pub reader: images::Reader,
    pub inspector: images::Inspector,
    pub gallery: images::Gallery,
    pub modal_open: bool,
    modal_origin: String,
    capture_shown: u32,
    pub sources: Vec<Item>,
    pub phase: SourcePhase,
    pub source_alert: String,
    current_source: String,
    current_generation: u32,
    current_expected: Option<CurrentTicket>,
    current_loading: bool,
    current_status: String,
    current_fault: String,
}

impl ImageUi {
    pub fn new() -> Self {
        let mut reader = images::Reader::new();
        if let crate::metrics::QueryRange::Bounded { start, end } = current_last_day() {
            reader.preset_range(start, end);
        }
        Self {
            reader,
            inspector: images::Inspector::new(),
            gallery: images::Gallery::new(),
            modal_open: false,
            modal_origin: String::new(),
            capture_shown: 0,
            sources: Vec::new(),
            phase: SourcePhase::Loading,
            source_alert: String::new(),
            current_source: String::new(),
            current_generation: 0,
            current_expected: None,
            current_loading: false,
            current_status: String::new(),
            current_fault: String::new(),
        }
    }
}

#[derive(Clone, PartialEq)]
enum ImageJob {
    Idle,
    Run(Ticket),
}

#[derive(Clone, PartialEq)]
enum ImageOutcome {
    Idle,
    Ready { ticket: Ticket, page: Page },
    Failed { ticket: Ticket, message: String },
}

#[derive(Clone, PartialEq)]
enum InspectJob {
    Idle,
    Run(InspectTicket),
}

#[derive(Clone, PartialEq)]
enum InspectOutcome {
    Idle,
    Ready {
        ticket: InspectTicket,
        frame: Frame,
    },
    Failed {
        ticket: InspectTicket,
        message: String,
    },
}

#[derive(Clone, PartialEq)]
enum ThumbJob {
    Idle,
    Run(ThumbTicket),
}

#[derive(Clone, PartialEq)]
enum ThumbOutcome {
    Idle,
    Ready { ticket: ThumbTicket, frame: Frame },
    Failed { ticket: ThumbTicket },
}

#[derive(Clone, PartialEq, Eq)]
struct CurrentTicket {
    generation: u32,
    source_id: String,
}

#[derive(Clone, PartialEq)]
enum CurrentJob {
    Idle,
    Run(CurrentTicket),
}

#[derive(Clone, PartialEq)]
enum CurrentOutcome {
    Idle,
    Ready {
        ticket: CurrentTicket,
        frame: Frame,
    },
    Failed {
        ticket: CurrentTicket,
        message: String,
    },
}

#[derive(Clone, Copy)]
struct Loader {
    history: Signal<ImageJob>,
    inspect: Signal<InspectJob>,
    thumbs: Signal<ThumbJob>,
    current: Signal<CurrentJob>,
}

thread_local! {
    static UI: RefCell<Option<Signal<ImageUi>>> = const { RefCell::new(None) };
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<ImageUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn catalog_changed() {
    let snap = super::inventory::snapshot(Primitive::Images);
    let ids = snap
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let phase = images::source_phase(snap.kind, ids.len());
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.sources = snap.items;
    next.phase = phase;
    next.source_alert = if phase == SourcePhase::Error {
        snap.alert
    } else {
        String::new()
    };
    let ticket = next.reader.note_catalog(phase, &ids);
    if !ids.iter().any(|id| id == &next.current_source) {
        clear_current(&mut next);
        next.current_source = next.reader.bound_source().to_owned();
        if next.current_source.is_empty() {
            next.current_source = ids.first().cloned().unwrap_or_default();
        }
    }
    next.inspector.rebind(next.reader.bound_source());
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
}

fn current_last_day() -> crate::metrics::QueryRange {
    let now = js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default();
    crate::metrics::last_day(&now).unwrap_or(crate::metrics::QueryRange::Window)
}

fn fill_default_range(ui: Signal<ImageUi>, mut filled: Signal<bool>) {
    if filled() {
        return;
    }
    let images::Read::Search {
        range: Some((start, end)),
        ..
    } = ui.read().reader.read.clone()
    else {
        return;
    };
    let start = datetime_local(&start);
    let end = datetime_local(&end);
    session::write_control("image-range-start", &start);
    session::write_control("image-range-end", &end);
    if session::control_value("image-range-end") == end {
        filled.set(true);
    }
}

fn datetime_local(stamp: &str) -> String {
    stamp.chars().take(19).collect()
}

fn bound() -> Option<Signal<ImageUi>> {
    UI.with(|slot| *slot.borrow())
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

#[component]
pub fn ImageView(ui: Signal<ImageUi>) -> Element {
    let history = use_signal(|| ImageJob::Idle);
    let inspect = use_signal(|| InspectJob::Idle);
    let thumbs = use_signal(|| ThumbJob::Idle);
    let current = use_signal(|| CurrentJob::Idle);
    let mut filled_range = use_signal(|| false);
    use_hook(|| {
        install_modal_escape();
        LOADER.with(|slot| {
            *slot.borrow_mut() = Some(Loader {
                history,
                inspect,
                thumbs,
                current,
            });
        });
    });
    let mut focused_capture = use_signal(|| 0u32);
    use_effect(move || fill_default_range(ui, filled_range));
    use_effect(move || {
        let generation = ui().capture_shown;
        if generation == focused_capture() {
            return;
        }
        focused_capture.set(generation);
        if ui.peek().modal_open {
            focus_control("image-modal");
        }
    });
    use_effect(move || {
        let state = ui();
        let (slug, _) = images::history_status(&state.reader, state.phase == SourcePhase::Ready);
        if slug != "ready" {
            release_hidden_history_focus();
        }
    });
    let history_reply = use_resource(move || {
        let job = history();
        let secret = session::native_secret();
        async move { run_job(job, secret).await }
    });
    let inspect_reply = use_resource(move || {
        let job = inspect();
        let secret = session::native_secret();
        async move { run_inspect(job, secret).await }
    });
    let thumb_reply = use_resource(move || {
        let job = thumbs();
        let secret = session::native_secret();
        async move { run_thumb(job, secret).await }
    });
    let current_reply = use_resource(move || {
        let job = current();
        let secret = session::native_secret();
        async move { run_current(job, secret).await }
    });
    let _ = history_reply.read();
    let _ = inspect_reply.read();
    let _ = thumb_reply.read();
    let _ = current_reply.read();
    if let Some(outcome) = history_reply() {
        apply_outcome(ui, outcome);
    }
    if let Some(outcome) = inspect_reply() {
        apply_inspect(ui, outcome);
    }
    if let Some(outcome) = thumb_reply() {
        apply_thumb(ui, outcome);
    }
    if let Some(outcome) = current_reply() {
        apply_current(ui, outcome);
    }
    let state = ui();
    let (history_slug, history_text) =
        images::history_status(&state.reader, state.phase == SourcePhase::Ready);
    let busy = state.phase == SourcePhase::Loading || state.reader.loading || state.current_loading;
    rsx! {
        section {
            id: "image-view",
            aria_labelledby: "image-view-heading",
            aria_busy: if busy { "true" } else { "false" },
            "data-gen": state.reader.generation.to_string(),
            h2 { id: "image-view-heading", class: "visually-hidden", tabindex: "-1", "Images" }
            CurrentActions { state: state.clone() }
            div { id: "image-browser", class: "log-browser",
                FilterRail { state: state.clone() }
                Results { ui, state: state.clone(), history_slug, history_text }
            }
            PreviewModal { state: state.clone() }
        }
    }
}

#[component]
fn FilterRail(state: ImageUi) -> Element {
    rsx! {
        div { id: "image-filters", role: "group", aria_label: "Image filters",
            div { id: "image-sources", class: "log-filter-group",
                p { "Source" }
                p {
                    id: "image-source-status",
                    role: "status",
                    aria_live: "polite",
                    "data-state": source_slug(state.phase),
                    "{images::source_copy(state.phase)}"
                }
                div { id: "image-source-alert", role: "alert", aria_live: "assertive", "{state.source_alert}" }
                if state.sources.is_empty() {
                    button {
                        id: "image-source-empty",
                        class: "log-facet",
                        r#type: "button",
                        disabled: true,
                        aria_pressed: "true",
                        "data-source": "",
                        span { class: "log-facet-mark", aria_hidden: "true" }
                        span { "{images::source_placeholder(state.phase)}" }
                    }
                } else {
                    button {
                        id: "image-source-all",
                        class: "log-facet",
                        r#type: "button",
                        "data-source": "",
                        aria_pressed: if state.reader.all { "true" } else { "false" },
                        onclick: move |_| choose_all(),
                        span { class: "log-facet-mark", aria_hidden: "true" }
                        span { "All" }
                    }
                    for (index, item) in state.sources.iter().enumerate() {
                        SourceFacet {
                            key: "{item.id}",
                            index,
                            item: item.clone(),
                            pressed: !state.reader.all
                                && state.reader.chosen.iter().any(|id| id == &item.id),
                        }
                    }
                }
            }
            div { class: "log-filter-group",
                label { r#for: "image-search", "Search" }
                input {
                    id: "image-search",
                    r#type: "search",
                    autocomplete: "off",
                    spellcheck: "false",
                    oninput: move |_| commit_criteria(),
                    onchange: move |_| commit_criteria(),
                }
            }
            div { class: "log-filter-group",
                p { "Time range" }
                label { r#for: "image-range-start", "Start" }
                input {
                    id: "image-range-start",
                    r#type: "datetime-local",
                    step: "1",
                    oninput: move |_| commit_criteria(),
                    onchange: move |_| commit_criteria(),
                }
                label { r#for: "image-range-end", "End" }
                input {
                    id: "image-range-end",
                    r#type: "datetime-local",
                    step: "1",
                    oninput: move |_| commit_criteria(),
                    onchange: move |_| commit_criteria(),
                }
            }
        }
    }
}

#[component]
fn SourceFacet(index: usize, item: Item, pressed: bool) -> Element {
    let id = item.id.clone();
    rsx! {
        button {
            id: "image-source-{index}",
            class: "log-facet",
            r#type: "button",
            "data-source": "{item.id}",
            aria_pressed: if pressed { "true" } else { "false" },
            onclick: move |_| choose_source(&id),
            span { class: "log-facet-mark", aria_hidden: "true" }
            span { "{source_option(&item)}" }
        }
    }
}

fn source_slug(phase: SourcePhase) -> &'static str {
    match phase {
        SourcePhase::Loading => "loading",
        SourcePhase::None => "sources",
        SourcePhase::Error => "error",
        SourcePhase::Ready => "ready",
    }
}

#[component]
fn CurrentActions(state: ImageUi) -> Element {
    let current_disabled = state.phase != SourcePhase::Ready
        || state.current_source.is_empty()
        || state.current_loading;
    rsx! {
        div { id: "image-current-actions", class: "image-current-actions",
            label { r#for: "image-current-source", "Capture from" }
            select {
                id: "image-current-source",
                disabled: state.current_loading || state.sources.is_empty(),
                onchange: move |_| set_current_source(),
                for item in &state.sources {
                    option {
                        value: "{item.id}",
                        selected: item.id == state.current_source,
                        "{item.name} · {item.id}"
                    }
                }
            }
            button {
                id: "image-current",
                class: "log-primary",
                r#type: "button",
                disabled: current_disabled,
                aria_busy: if state.current_loading { "true" } else { "false" },
                onclick: move |_| request_current(),
                "Capture current image"
            }
            div {
                id: "image-current-status",
                role: "status",
                aria_live: "polite",
                "{state.current_status}"
            }
            div {
                id: "image-current-alert",
                role: "alert",
                aria_live: "assertive",
                "{state.current_fault}"
            }
        }
    }
}

#[component]
fn Results(
    ui: Signal<ImageUi>,
    state: ImageUi,
    history_slug: &'static str,
    history_text: &'static str,
) -> Element {
    let _ = ui;
    let ready = history_slug == "ready";
    let alert = images::history_alert(&state.reader);
    rsx! {
        div { id: "image-results", class: "log-results",
            div { class: "log-scroll",
                p {
                    id: "image-history-status",
                    role: "status",
                    aria_live: "polite",
                    "data-state": "{history_slug}",
                    "{history_text}"
                }
                div { id: "image-history-alert", role: "alert", aria_live: "assertive", "{alert}" }
                div {
                    id: "image-history",
                    class: "log-cards",
                    role: "radiogroup",
                    aria_label: "Image history",
                    hidden: !ready,
                    onkeydown: result_key,
                    for (index, item) in state.reader.items.iter().enumerate() {
                        ResultCard {
                            index,
                            item: item.clone(),
                            selected: state.reader.selected.clone(),
                            thumb: state.gallery.get(&item.id).cloned(),
                        }
                    }
                }
                if state.reader.next_cursor.is_some() && ready {
                    button {
                        id: "image-more",
                        r#type: "button",
                        onclick: move |_| load_more(),
                        "More images"
                    }
                }
            }
        }
    }
}

#[component]
fn ResultCard(index: usize, item: Descriptor, selected: String, thumb: Option<Thumb>) -> Element {
    let id = item.id.clone();
    let checked = item.id == selected;
    let any = !selected.is_empty();
    let alt = card_alt(&item);
    rsx! {
        button {
            id: "image-result-{index}",
            r#type: "button",
            class: "image-result",
            role: "radio",
            tabindex: radio_tab(checked, index, any),
            "data-id": "{item.id}",
            "data-image": "{item.id}",
            aria_checked: if checked { "true" } else { "false" },
            onclick: move |_| open_result(&id),
            Thumbnail { index, alt: alt.clone(), thumb: thumb.clone() }
            span { class: "image-result-copy",
                span { class: "log-card-id", "{item.id}" }
                time { datetime: "{item.captured_at}", "{item.captured_at}" }
                if !item.caption.is_empty() {
                    span { class: "log-card-title", "{item.caption}" }
                }
                span { class: "log-meta",
                    span { class: "log-meta-label", "Media type" }
                    span { "{item.media_type}" }
                }
                span { class: "log-meta",
                    span { class: "log-meta-label", "Dimensions" }
                    span { "{item.width} × {item.height}" }
                }
                if !item.attributes.is_empty() {
                    span { class: "log-meta",
                        span { class: "log-meta-label", "Attributes" }
                        span { "{item.attributes}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Thumbnail(index: usize, alt: String, thumb: Option<Thumb>) -> Element {
    let url = thumb
        .as_ref()
        .map(|item| item.url.clone())
        .filter(|url| !url.is_empty());
    rsx! {
        if let Some(url) = url {
            img { id: "image-thumb-{index}", class: "image-thumb", alt: "{alt}", src: "{url}" }
        } else {
            span { class: "image-thumb image-thumb-empty", aria_hidden: "true" }
        }
    }
}

#[component]
fn PreviewModal(state: ImageUi) -> Element {
    if !state.modal_open {
        return rsx! {};
    }
    let (slug, _) = images::inspect_status(&state.inspector);
    let frame = state.inspector.frame.clone();
    let image_id = frame
        .as_ref()
        .map(|frame| frame.id.clone())
        .unwrap_or_default();
    let origin = frame
        .as_ref()
        .map(|_| state.modal_origin.clone())
        .unwrap_or_default();
    let size = frame.as_ref().and_then(preview_size);
    let preview = frame.and_then(|frame| {
        let alt = images::preview_alt(&frame);
        frame.preview.preview_url.map(|url| (alt, url))
    });
    rsx! {
        div {
            id: "image-modal",
            class: "image-modal",
            role: "dialog",
            aria_modal: "true",
            aria_label: "Image preview",
            tabindex: "-1",
            "data-state": "{slug}",
            "data-image": "{image_id}",
            "data-origin": "{origin}",
            onclick: move |_| {
                close_modal();
            },
            if let Some((alt, url)) = preview {
                div {
                    class: "image-preview-frame",
                    onclick: move |event| event.stop_propagation(),
                    img {
                        id: "image-preview",
                        alt: "{alt}",
                        src: "{url}",
                        onload: move |_| upscale_preview(),
                    }
                    if let Some(size) = size {
                        p { id: "image-size", class: "image-size", "{size}" }
                    }
                }
            }
        }
    }
}

fn preview_size(frame: &images::Frame) -> Option<String> {
    if frame.width == 0 || frame.height == 0 || frame.preview.preview_url.is_none() {
        return None;
    }
    Some(format!("{} × {}", frame.width, frame.height))
}

fn upscale_preview() {
    let Some(image) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("image-preview"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlImageElement>().ok())
    else {
        return;
    };
    let Some((width, height)) =
        images::preview_display_size(image.natural_width(), image.natural_height())
    else {
        return;
    };
    let Ok(element) = image.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let style = element.style();
    let _ = style.set_property("width", &format!("{width}px"));
    let _ = style.set_property("height", &format!("{height}px"));
    let _ = style.set_property("image-rendering", "pixelated");
}

fn card_alt(item: &Descriptor) -> String {
    let caption = item.caption.trim();
    if !caption.is_empty() {
        return caption.to_owned();
    }
    if item.id.is_empty() {
        return "Image".to_owned();
    }
    item.id.clone()
}

fn commit_criteria() {
    let text = session::control_value("image-search");
    let start = session::control_value("image-range-start");
    let end = session::control_value("image-range-end");
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let ticket = match next.reader.commit(&text, &start, &end) {
        Ok(ticket) => ticket,
        Err(_) => {
            ui.set(next);
            return;
        }
    };
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
}

fn choose_all() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let ticket = next.reader.select_all();
    next.inspector.rebind(next.reader.bound_source());
    if ticket.is_some() {
        next.gallery.retarget(&next.reader.scope_key());
        next.modal_open = false;
    }
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
}

fn choose_source(id: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let ids = ui
        .peek()
        .sources
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let mut next = ui.peek().clone();
    let ticket = next.reader.select_source(id, &ids);
    next.inspector.rebind(next.reader.bound_source());
    if ticket.is_some() {
        next.gallery.retarget(&next.reader.scope_key());
        next.modal_open = false;
    }
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
}

fn request_current() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let source_id = next.current_source.clone();
    if source_id.is_empty() || next.current_loading {
        return;
    }
    next.current_generation = next.current_generation.wrapping_add(1);
    let ticket = CurrentTicket {
        generation: next.current_generation,
        source_id,
    };
    next.current_expected = Some(ticket.clone());
    next.current_loading = true;
    next.current_status = "Capturing current image.".to_owned();
    next.current_fault.clear();
    ui.set(next);
    launch_current(ticket);
}

fn set_current_source() {
    let source = session::control_value("image-current-source");
    let Some(mut ui) = bound() else {
        return;
    };
    if !ui.peek().sources.iter().any(|item| item.id == source) {
        return;
    }
    let mut next = ui.peek().clone();
    clear_current(&mut next);
    next.current_source = source;
    ui.set(next);
}

fn clear_current(state: &mut ImageUi) {
    state.current_generation = state.current_generation.wrapping_add(1);
    state.current_expected = None;
    state.current_loading = false;
    state.current_status.clear();
    state.current_fault.clear();
}

fn open_result(id: &str) {
    choose_result(id);
}

fn close_modal() -> bool {
    let Some(mut ui) = bound() else {
        return false;
    };
    let mut next = ui.peek().clone();
    if !next.modal_open {
        return false;
    }
    next.modal_open = false;
    ui.set(next);
    true
}

fn choose_result(id: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.reader.select_result(id);
    next.modal_open = !next.reader.selected.is_empty();
    if next.modal_open {
        next.modal_origin = "history".to_owned();
    }
    let selected = next.reader.selected.clone();
    let source = next
        .reader
        .items
        .iter()
        .find(|item| item.id == selected)
        .map(|item| item.source_id.clone())
        .filter(|source| !source.is_empty())
        .unwrap_or_else(|| next.reader.source_id.clone());
    let index = next
        .reader
        .items
        .iter()
        .position(|item| item.id == selected);
    let ticket = next.inspector.open_selected(&source, &selected);
    ui.set(next);
    if let Some(index) = index {
        focus_control(&format!("image-result-{index}"));
    }
    if let Some(ticket) = ticket {
        launch_inspect(ticket);
    }
}

fn result_key(event: KeyboardEvent) {
    let delta: isize = match event.key() {
        Key::ArrowDown | Key::ArrowRight => 1,
        Key::ArrowUp | Key::ArrowLeft => -1,
        _ => return,
    };
    event.prevent_default();
    let Some(ui) = bound() else {
        return;
    };
    let items = ui.peek().reader.items.clone();
    if items.is_empty() {
        return;
    }
    let current = ui.peek().reader.selected.clone();
    let next = if current.is_empty() {
        0
    } else {
        let index = items
            .iter()
            .position(|item| item.id == current)
            .unwrap_or(0);
        (index as isize + delta).rem_euclid(items.len() as isize) as usize
    };
    choose_result(&items[next].id);
}

fn load_more() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let ticket = next.reader.more();
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
}

fn launch(ticket: Ticket) {
    let Some(mut loader) = loader() else {
        return;
    };
    if !calls_allowed() || take_secret(&visible_parts(&ticket)).is_err() {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let _ = next.reader.cancel(&ticket);
        ui.set(next);
        return;
    }
    loader.history.set(ImageJob::Run(ticket));
}

fn launch_inspect(ticket: InspectTicket) {
    let Some(mut loader) = loader() else {
        return;
    };
    let blocked = {
        let mut parts = vec![ticket.source_id.as_str()];
        if !ticket.image_id.is_empty() {
            parts.push(ticket.image_id.as_str());
        }
        !calls_allowed() || take_secret(&parts).is_err()
    };
    if blocked {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let _ = next.inspector.cancel(&ticket);
        ui.set(next);
        return;
    }
    loader.inspect.set(InspectJob::Run(ticket));
}

fn launch_thumb(ticket: ThumbTicket) {
    let Some(mut loader) = loader() else {
        return;
    };
    let blocked = !calls_allowed()
        || take_secret(&[ticket.source_id.as_str(), ticket.image_id.as_str()]).is_err();
    if blocked {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if !next.gallery.reject(&ticket) {
            return;
        }
        let follow = next.gallery.pump();
        ui.set(next);
        if let Some(follow) = follow {
            launch_thumb(follow);
        }
        return;
    }
    loader.thumbs.set(ThumbJob::Run(ticket));
}

fn launch_current(ticket: CurrentTicket) {
    let Some(mut loader) = loader() else {
        return;
    };
    if !calls_allowed() || take_secret(&[ticket.source_id.as_str()]).is_err() {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if next.current_expected.as_ref() == Some(&ticket) {
            clear_current(&mut next);
            ui.set(next);
        }
        return;
    }
    loader.current.set(CurrentJob::Run(ticket));
}

fn install_modal_escape() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let runtime = Runtime::current();
    let scope = current_scope_id();
    let closure =
        wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Escape" {
                return;
            }
            let _guard = RuntimeGuard::new(runtime.clone());
            let closed = runtime.in_scope(scope, close_modal);
            if closed {
                runtime.needs_update(scope);
                event.prevent_default();
            }
        }) as Box<dyn FnMut(_)>);
    let _ = window.add_event_listener_with_callback_and_bool(
        "keydown",
        closure.as_ref().unchecked_ref(),
        true,
    );
    closure.forget();
}

fn visible_parts(ticket: &Ticket) -> Vec<&str> {
    let mut parts = Vec::new();
    if !ticket.source_id.is_empty() {
        parts.push(ticket.source_id.as_str());
    }
    for source in &ticket.sources {
        parts.push(source.as_str());
    }
    if let Some(cursor) = ticket.cursor.as_deref() {
        parts.push(cursor);
    }
    if let Read::Search { text, range } = &ticket.read {
        if let Some(text) = text.as_deref() {
            parts.push(text);
        }
        if let Some((start, end)) = range {
            parts.push(start.as_str());
            parts.push(end.as_str());
        }
    }
    parts
}

async fn run_job(job: ImageJob, secret: Option<String>) -> ImageOutcome {
    let ImageJob::Run(ticket) = job else {
        return ImageOutcome::Idle;
    };
    if ticket.source_id.is_empty() && !ticket.sources.is_empty() {
        return gather(ticket, secret).await;
    }
    let command = images::command(&ticket.source_id, &ticket.read, ticket.cursor.as_deref());
    let reply = match post(command, secret.clone()).await {
        Ok(reply) => reply,
        Err(message) => return ImageOutcome::Failed { ticket, message },
    };
    match images::interpret(&reply, secret.as_deref()) {
        Ok(page) => ImageOutcome::Ready { ticket, page },
        Err(fault) => ImageOutcome::Failed {
            ticket,
            message: fault_message(&fault),
        },
    }
}

async fn gather(ticket: Ticket, secret: Option<String>) -> ImageOutcome {
    let mut items = Vec::new();
    for source in ticket.sources.clone() {
        let mut cursor = None;
        loop {
            let command = images::command(&source, &ticket.read, cursor.as_deref());
            let reply = match post(command, secret.clone()).await {
                Ok(reply) => reply,
                Err(message) => return ImageOutcome::Failed { ticket, message },
            };
            let page = match images::interpret(&reply, secret.as_deref()) {
                Ok(page) => page,
                Err(fault) => {
                    return ImageOutcome::Failed {
                        ticket,
                        message: fault_message(&fault),
                    };
                }
            };
            let next = page.next_cursor.clone().filter(|cursor| !cursor.is_empty());
            items.extend(page.items);
            if next.is_none() || next == cursor {
                break;
            }
            cursor = next;
        }
    }
    items.sort_by(|left, right| {
        left.captured_at
            .cmp(&right.captured_at)
            .then(left.id.cmp(&right.id))
    });
    ImageOutcome::Ready {
        ticket,
        page: Page {
            items,
            next_cursor: None,
        },
    }
}

async fn run_thumb(job: ThumbJob, secret: Option<String>) -> ThumbOutcome {
    let ThumbJob::Run(ticket) = job else {
        return ThumbOutcome::Idle;
    };
    let command = images::image_command(&ticket.image_id);
    let reply = match post(command, secret.clone()).await {
        Ok(reply) => reply,
        Err(_) => return ThumbOutcome::Failed { ticket },
    };
    match images::interpret_frame(&reply, secret.as_deref()) {
        Ok(frame) => ThumbOutcome::Ready { ticket, frame },
        Err(_) => ThumbOutcome::Failed { ticket },
    }
}

async fn run_inspect(job: InspectJob, secret: Option<String>) -> InspectOutcome {
    let InspectJob::Run(ticket) = job else {
        return InspectOutcome::Idle;
    };
    let command = images::image_command(&ticket.image_id);
    let reply = match post(command, secret.clone()).await {
        Ok(reply) => reply,
        Err(message) => return InspectOutcome::Failed { ticket, message },
    };
    match images::interpret_frame(&reply, secret.as_deref()) {
        Ok(frame) => InspectOutcome::Ready { ticket, frame },
        Err(fault) => InspectOutcome::Failed {
            ticket,
            message: fault_message(&fault),
        },
    }
}

async fn run_current(job: CurrentJob, secret: Option<String>) -> CurrentOutcome {
    let CurrentJob::Run(ticket) = job else {
        return CurrentOutcome::Idle;
    };
    let command = images::current_command(&ticket.source_id);
    let reply = match post(command, secret.clone()).await {
        Ok(reply) => reply,
        Err(message) => return CurrentOutcome::Failed { ticket, message },
    };
    match images::interpret_current_frame(&reply, secret.as_deref()) {
        Ok(frame) if frame.source_id == ticket.source_id => CurrentOutcome::Ready { ticket, frame },
        Ok(_) => CurrentOutcome::Failed {
            ticket,
            message: fault_message(&Fault::Malformed),
        },
        Err(fault) => CurrentOutcome::Failed {
            ticket,
            message: fault_message(&fault),
        },
    }
}

async fn post(
    command: serde_json::Value,
    secret: Option<String>,
) -> Result<crate::codec::Reply, String> {
    let Some(transport) = session::transport() else {
        return Err("The browser could not reach this origin.".to_owned());
    };
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        return Err("The image query could not be built.".to_owned());
    };
    match transport.send(&body, secret.as_deref()).await {
        Err(()) => Err("The browser could not reach this origin.".to_owned()),
        Ok(reply) => Ok(reply),
    }
}

fn apply_outcome(mut ui: Signal<ImageUi>, outcome: ImageOutcome) {
    let mut next = ui.peek().clone();
    let changed = match outcome {
        ImageOutcome::Idle => false,
        ImageOutcome::Ready { ticket, page } => next.reader.accept(&ticket, page),
        ImageOutcome::Failed { ticket, message } => next.reader.reject(&ticket, &message),
    };
    if changed {
        let ids = next
            .reader
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let scope = next.reader.scope_key();
        let thumb = next.gallery.note(&scope, &ids);
        ui.set(next);
        if let Some(thumb) = thumb {
            launch_thumb(thumb);
        }
    }
}

fn apply_inspect(mut ui: Signal<ImageUi>, outcome: InspectOutcome) {
    let mut next = ui.peek().clone();
    let changed = match outcome {
        InspectOutcome::Idle => false,
        InspectOutcome::Ready { ticket, frame } => next.inspector.accept(&ticket, frame),
        InspectOutcome::Failed { ticket, message } => next.inspector.reject(&ticket, &message),
    };
    if changed {
        ui.set(next);
    }
}

fn apply_thumb(mut ui: Signal<ImageUi>, outcome: ThumbOutcome) {
    let mut next = ui.peek().clone();
    let applied = match outcome {
        ThumbOutcome::Idle => return,
        ThumbOutcome::Ready { ticket, frame } => {
            next.gallery.accept(&ticket, images::thumb_from(&frame))
        }
        ThumbOutcome::Failed { ticket } => next.gallery.reject(&ticket),
    };
    if !applied {
        return;
    }
    let follow = next.gallery.pump();
    ui.set(next);
    if let Some(follow) = follow {
        launch_thumb(follow);
    }
}

fn apply_current(mut ui: Signal<ImageUi>, outcome: CurrentOutcome) {
    let mut next = ui.peek().clone();
    let (ticket, shown, failure) = match outcome {
        CurrentOutcome::Idle => return,
        CurrentOutcome::Ready { ticket, frame } => (ticket, Some(frame), None),
        CurrentOutcome::Failed { ticket, message } => (ticket, None, Some(message)),
    };
    if next.current_expected.as_ref() != Some(&ticket) {
        return;
    }
    next.current_expected = None;
    next.current_loading = false;
    let refresh = if let Some(message) = failure {
        next.current_status = "Current image could not be captured.".to_owned();
        next.current_fault = message;
        None
    } else if let Some(frame) = shown {
        next.current_status.clear();
        next.current_fault.clear();
        next.inspector.show(frame);
        next.modal_open = true;
        next.modal_origin = "current".to_owned();
        next.capture_shown = next.capture_shown.wrapping_add(1);
        next.reader.refresh()
    } else {
        None
    };
    ui.set(next);
    if let Some(refresh) = refresh {
        launch(refresh);
    }
}

fn release_hidden_history_focus() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(active) = document.active_element() else {
        return;
    };
    let Some(history) = document.get_element_by_id("image-history") else {
        return;
    };
    if history.contains(Some(&active)) {
        focus_control("image-search");
    }
}

fn focus_control(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        && let Ok(html) = element.dyn_into::<HtmlElement>()
    {
        let _ = html.focus();
    }
}

fn radio_tab(checked: bool, index: usize, any: bool) -> &'static str {
    if checked || (!any && index == 0) {
        "0"
    } else {
        "-1"
    }
}

fn source_option(item: &Item) -> String {
    let mut text = if item.name.is_empty() {
        item.id.clone()
    } else {
        format!("{} · {}", item.name, item.id)
    };
    if !item.description.is_empty() {
        text.push_str(" · ");
        text.push_str(&item.description);
    }
    text
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The image result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}
