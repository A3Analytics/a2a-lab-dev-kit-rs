//! Plain-CSS settings primitives that follow design-system field, card, and alert semantics.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Info,
    Warning,
    Destructive,
}

#[component]
pub fn SettingsAlert(
    id: String,
    kind: AlertKind,
    title: String,
    body: String,
    hidden: bool,
) -> Element {
    let class = match kind {
        AlertKind::Info => "settings-alert",
        AlertKind::Warning => "settings-alert settings-alert-warning",
        AlertKind::Destructive => "settings-alert settings-alert-destructive",
    };
    rsx! {
        div {
            id: "{id}",
            class: "{class}",
            role: "alert",
            hidden,
            p { class: "settings-alert-title", "{title}" }
            p { class: "settings-alert-body", "{body}" }
        }
    }
}

#[component]
pub fn ReindexSign() -> Element {
    rsx! {
        span { class: "settings-impact",
            span { class: "settings-impact-mark", aria_hidden: "true", "!" }
            span { class: "settings-impact-text", "Requires semantic reindex" }
        }
    }
}

#[component]
pub fn LiveSign() -> Element {
    rsx! {
        span { class: "settings-impact settings-impact-live",
            span { class: "settings-impact-text", "Applies live" }
        }
    }
}

#[component]
pub fn SectionCard(title: String, description: String, children: Element) -> Element {
    rsx! {
        section { class: "settings-card",
            div { class: "settings-card-header",
                h2 { class: "settings-card-title", "{title}" }
                p { class: "settings-card-description", "{description}" }
            }
            div { class: "settings-card-body", {children} }
        }
    }
}

#[component]
pub fn StickyActions(locked: bool, saving: bool) -> Element {
    rsx! {
        div { class: "settings-sticky",
            p { class: "settings-sticky-note",
                if locked {
                    "Editing is unavailable for this configuration."
                } else {
                    "Save writes only changed keys. Cancel restores the last loaded document."
                }
            }
            button {
                id: "settings-cancel",
                class: "settings-button-outline",
                r#type: "reset",
                disabled: locked || saving,
                "Cancel"
            }
            button {
                id: "settings-save",
                class: "settings-button-primary",
                r#type: "submit",
                disabled: locked || saving,
                "Save"
            }
        }
    }
}

#[component]
pub fn TextField(
    id: String,
    label: String,
    help: String,
    error: String,
    restart: bool,
    #[props(default)] live: bool,
    #[props(default)] reindex: bool,
    locked: bool,
    value: String,
    oninput: EventHandler<String>,
) -> Element {
    let described = described_by(&id, &error);
    let invalid = !error.is_empty();
    rsx! {
        div { class: "settings-field",
            FieldLabel { id: id.clone(), label, restart, live, reindex }
            input {
                id: "{id}",
                r#type: "text",
                autocomplete: "off",
                spellcheck: false,
                value: "{value}",
                readonly: locked,
                "aria-invalid": if invalid { "true" } else { "false" },
                aria_describedby: "{described}",
                oninput: move |event| oninput.call(event.value()),
            }
            FieldCopy { id: id.clone(), help, error }
        }
    }
}

#[component]
pub fn SelectField(
    id: String,
    label: String,
    help: String,
    error: String,
    restart: bool,
    #[props(default)] live: bool,
    #[props(default)] reindex: bool,
    locked: bool,
    value: String,
    options: Vec<(String, String)>,
    oninput: EventHandler<String>,
) -> Element {
    let described = described_by(&id, &error);
    let invalid = !error.is_empty();
    rsx! {
        div { class: "settings-field",
            FieldLabel { id: id.clone(), label, restart, live, reindex }
            select {
                id: "{id}",
                value: "{value}",
                disabled: locked,
                "aria-invalid": if invalid { "true" } else { "false" },
                aria_describedby: "{described}",
                onchange: move |event| oninput.call(event.value()),
                for (option_value , option_label) in options {
                    option {
                        value: "{option_value}",
                        selected: option_value == value,
                        "{option_label}"
                    }
                }
            }
            FieldCopy { id: id.clone(), help, error }
        }
    }
}

#[component]
pub fn CheckField(
    id: String,
    label: String,
    help: String,
    error: String,
    restart: bool,
    #[props(default)] live: bool,
    locked: bool,
    checked: bool,
    oninput: EventHandler<bool>,
) -> Element {
    let described = described_by(&id, &error);
    let invalid = !error.is_empty();
    let watch_id = id.clone();
    rsx! {
        div { class: "settings-field settings-field-check",
            FieldLabel { id: id.clone(), label, restart, live, reindex: false }
            label { class: "settings-switch", r#for: "{id}",
                input {
                    id: "{id}",
                    class: "settings-switch-input",
                    r#type: "checkbox",
                    checked,
                    disabled: locked,
                    "aria-invalid": if invalid { "true" } else { "false" },
                    aria_describedby: "{described}",
                    onchange: move |_| {
                        let checked = web_sys::window()
                            .and_then(|window| window.document())
                            .and_then(|document| document.get_element_by_id(&watch_id))
                            .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
                            .map(|input| input.checked())
                            .unwrap_or(false);
                        oninput.call(checked);
                    },
                }
                span { class: "settings-switch-track", aria_hidden: "true",
                    span { class: "settings-switch-thumb" }
                },
            }
            FieldCopy { id: id.clone(), help, error }
        }
    }
}

#[component]
fn FieldLabel(id: String, label: String, restart: bool, live: bool, reindex: bool) -> Element {
    let label = friendly_label(&label);
    let _ = restart;
    rsx! {
        div { class: "settings-field-label-row",
            label { r#for: "{id}", "{label}" }
            if reindex {
                ReindexSign {}
            }
            if live {
                LiveSign {}
            }
        }
    }
}

fn friendly_label(path: &str) -> String {
    let name = path.rsplit('.').next().unwrap_or(path);
    match name {
        "credentials" => "Credentials".to_owned(),
        "env" => "Environment variable".to_owned(),
        "fps" => "FPS".to_owned(),
        "dvr" => "DVR".to_owned(),
        "roi" => "ROI".to_owned(),
        "top_k" => "Top K".to_owned(),
        "max_batch_items" => "Maximum batch items".to_owned(),
        "max_concurrency" => "Maximum concurrency".to_owned(),
        "max_queue_backlog" => "Maximum queue backlog".to_owned(),
        "max_retry_attempts" => "Maximum retry attempts".to_owned(),
        "max_neighbor_frames" => "Maximum neighbor frames".to_owned(),
        "max_rois_per_frame" => "Maximum ROIs per frame".to_owned(),
        "job_deadline" => "Job deadline".to_owned(),
        "retry_backoff" => "Retry backoff".to_owned(),
        "score_floor" => "Score floor".to_owned(),
        "sqlite_path" => "SQLite path".to_owned(),
        "vector_index_path" => "Vector index path".to_owned(),
        "max_disk_bytes" => "Maximum disk size".to_owned(),
        "capture_rate" => "Capture rate".to_owned(),
        "storage_path" => "Storage path".to_owned(),
        _ => name
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[component]
fn FieldCopy(id: String, help: String, error: String) -> Element {
    let help_id = format!("{id}-help");
    let error_id = format!("{id}-error");
    rsx! {
        p { id: "{help_id}", class: "settings-help", "{help}" }
        if !error.is_empty() {
            p { id: "{error_id}", class: "settings-error", "{error}" }
        }
    }
}

fn described_by(id: &str, error: &str) -> String {
    if error.is_empty() {
        format!("{id}-help")
    } else {
        format!("{id}-help {id}-error")
    }
}
