//! Image history reader. The sidebar picks one source; the main column lists its frames.

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
    pub sources: Vec<Item>,
    pub phase: SourcePhase,
    pub source_alert: String,
}

impl ImageUi {
    pub fn new() -> Self {
        Self {
            reader: images::Reader::new(),
            inspector: images::Inspector::new(),
            gallery: images::Gallery::new(),
            modal_open: false,
            sources: Vec::new(),
            phase: SourcePhase::Loading,
            source_alert: String::new(),
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

#[derive(Clone, Copy)]
struct Loader {
    history: Signal<ImageJob>,
    inspect: Signal<InspectJob>,
    thumbs: Signal<ThumbJob>,
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
    next.inspector.rebind(&next.reader.source_id);
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
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
    use_hook(|| {
        install_modal_escape();
        LOADER.with(|slot| {
            *slot.borrow_mut() = Some(Loader {
                history,
                inspect,
                thumbs,
            });
        });
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
    let _ = history_reply.read();
    let _ = inspect_reply.read();
    let _ = thumb_reply.read();
    if let Some(outcome) = history_reply() {
        apply_outcome(ui, outcome);
    }
    if let Some(outcome) = inspect_reply() {
        apply_inspect(ui, outcome);
    }
    if let Some(outcome) = thumb_reply() {
        apply_thumb(ui, outcome);
    }
    let state = ui();
    let (history_slug, history_text) =
        images::history_status(&state.reader, state.phase == SourcePhase::Ready);
    let busy = state.phase == SourcePhase::Loading || state.reader.loading;
    rsx! {
        section {
            id: "image-view",
            aria_labelledby: "image-view-heading",
            aria_busy: if busy { "true" } else { "false" },
            "data-gen": state.reader.generation.to_string(),
            h2 { id: "image-view-heading", class: "visually-hidden", tabindex: "-1", "Images" }
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
                            pressed: !state.reader.all && item.id == state.reader.source_id,
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
    let origin = frame.as_ref().map(|_| "history").unwrap_or("");
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
                img {
                    id: "image-preview",
                    alt: "{alt}",
                    src: "{url}",
                    onclick: move |event| event.stop_propagation(),
                }
            }
        }
    }
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
    next.inspector.rebind("");
    if ticket.is_some() {
        next.gallery.retarget("*");
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
    next.inspector.rebind(&next.reader.source_id);
    if ticket.is_some() {
        next.gallery.retarget(&next.reader.source_id);
        next.modal_open = false;
    }
    ui.set(next);
    if let Some(ticket) = ticket {
        launch(ticket);
    }
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
    if ticket.source_id.is_empty() && matches!(ticket.read, Read::List) {
        return list_every(ticket, secret).await;
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

async fn list_every(ticket: Ticket, secret: Option<String>) -> ImageOutcome {
    let mut items = Vec::new();
    for source in ticket.sources.clone() {
        let mut cursor = None;
        loop {
            let command = images::command(&source, &Read::List, cursor.as_deref());
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
        let thumb = if next.reader.all {
            next.gallery.note("*", &ids)
        } else {
            next.gallery.note(&next.reader.source_id, &ids)
        };
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
