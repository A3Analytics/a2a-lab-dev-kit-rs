//! Video capture settings on `/ui/config/settings`.

use std::cell::{Cell, RefCell};

use dioxus::prelude::*;
use serde::Deserialize;
use wasm_bindgen_futures::spawn_local;

use crate::shell::session::{self, take_secret};

const SETTINGS_PATH: &str = "/config/settings";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct CaptureSettingsView {
    sources: Vec<SourceRow>,
    configured_videos_fps: Option<u32>,
    live_output_fps: u32,
    live_retention_secs: Option<u64>,
    dvr_save_interval: Option<String>,
    dvr_retention_secs: Option<u64>,
    dvr_storage_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum CameraIdRow {
    Index(u32),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SourceRow {
    id: String,
    display_name: String,
    format: String,
    camera_id: Option<CameraIdRow>,
    width: Option<u32>,
    height: Option<u32>,
    capture_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsUi {
    pub alert: String,
    pub view: Option<CaptureSettingsView>,
    pub fps_input: String,
    pub loading: bool,
    pub saving: bool,
}

impl SettingsUi {
    pub fn new() -> Self {
        Self {
            alert: String::new(),
            view: None,
            fps_input: String::new(),
            loading: false,
            saving: false,
        }
    }
}

thread_local! {
    static UI: RefCell<Option<Signal<SettingsUi>>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<SettingsUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

#[component]
pub fn SettingsView(ui: Signal<SettingsUi>, visible: bool) -> Element {
    let mut shown = use_signal(|| false);
    if shown() != visible {
        shown.set(visible);
    }
    let once = use_hook(|| Cell::new(false));
    use_effect(move || {
        if !shown() || once.get() {
            return;
        }
        once.set(true);
        start_load();
    });
    let state = ui();
    let configured_fps = state
        .view
        .as_ref()
        .and_then(|view| view.configured_videos_fps)
        .map(|fps| fps.to_string())
        .unwrap_or_else(|| "not configured".to_owned());
    let dvr_retention = state
        .view
        .as_ref()
        .and_then(|view| view.dvr_retention_secs)
        .map(|secs| format!("{secs}s"))
        .unwrap_or_else(|| "not configured".to_owned());
    let alert_hidden = state.alert.is_empty();
    rsx! {
        section {
            id: "config-settings-panel",
            class: "config-settings",
            hidden: !visible,
            p { class: "config-settings-lead",
                "Live output cadence comes from videos.fps. DVR save cadence and retention are configured separately and are not changed here."
            }
            div {
                id: "config-settings-alert",
                role: "alert",
                hidden: alert_hidden,
                "{state.alert}"
            }
            if state.loading {
                p { id: "config-settings-status", role: "status", "Loading capture settings." }
            } else if let Some(view) = state.view.clone() {
                table { class: "config-settings-table", aria_label: "Configured video sources",
                    thead {
                        tr {
                            th { scope: "col", "Source" }
                            th { scope: "col", "Format" }
                            th { scope: "col", "Camera" }
                            th { scope: "col", "Width" }
                            th { scope: "col", "Height" }
                            th { scope: "col", "Capture rate" }
                            th { scope: "col", "Configured videos.fps" }
                        }
                    }
                    tbody {
                        for (index , source) in view.sources.iter().enumerate() {
                            tr {
                                id: "video-source-{index}",
                                td { "{source.display_name} ({source.id})" }
                                td { "{source.format}" }
                                td {
                                    match source.camera_id.as_ref() {
                                        Some(CameraIdRow::Index(index)) => rsx! { "{index}" },
                                        Some(CameraIdRow::Text(text)) => rsx! { "{text}" },
                                        None => rsx! { "default" },
                                    }
                                }
                                td {
                                    if let Some(width) = source.width {
                                        "{width}"
                                    }
                                }
                                td {
                                    if let Some(height) = source.height {
                                        "{height}"
                                    }
                                }
                                td {
                                    if let Some(capture_rate) = source.capture_rate {
                                        "{capture_rate}"
                                    }
                                }
                                td {
                                    if let Some(fps) = view.configured_videos_fps {
                                        "{fps}"
                                    }
                                }
                            }
                        }
                    }
                }
                dl { class: "config-settings-meta",
                    div {
                        dt { "Active live output fps" }
                        dd { id: "video-live-output-fps", "{view.live_output_fps}" }
                    }
                    div {
                        dt { "Configured videos.fps" }
                        dd { id: "video-configured-fps", "{configured_fps}" }
                    }
                    div {
                        dt { "DVR retention" }
                        dd {
                            id: "video-dvr-retention",
                            "data-retention-secs": view.dvr_retention_secs.unwrap_or(0),
                            "{dvr_retention}"
                        }
                    }
                }
                form {
                    id: "video-live-fps-form",
                    class: "config-settings-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_fps();
                    },
                    label { r#for: "video-live-fps-input", "videos.fps (live output cadence)" }
                    input {
                        id: "video-live-fps-input",
                        name: "videos.fps",
                        r#type: "text",
                        inputmode: "numeric",
                        autocomplete: "off",
                        value: "{state.fps_input}",
                        disabled: state.saving,
                        oninput: move |event| {
                            set_ui(|state| state.fps_input = event.value());
                        },
                    }
                    button {
                        id: "video-live-fps-submit",
                        r#type: "submit",
                        disabled: state.saving,
                        "Apply live output fps"
                    }
                }
            } else {
                p { id: "config-settings-status", role: "status",
                    "Capture settings are unavailable on this agent."
                }
            }
        }
    }
}

fn set_ui(edit: impl FnOnce(&mut SettingsUi)) {
    UI.with(|slot| {
        let Some(mut ui) = *slot.borrow() else {
            return;
        };
        let mut next = ui.peek().clone();
        edit(&mut next);
        ui.set(next);
    });
}

fn start_load() {
    set_ui(|state| {
        state.loading = true;
        state.alert.clear();
    });
    spawn_local(async {
        match load_view().await {
            Ok(view) => set_ui(|state| {
                state.view = Some(view.clone());
                state.fps_input = view.live_output_fps.to_string();
                state.loading = false;
                state.saving = false;
                state.alert.clear();
            }),
            Err(message) if message == "missing" => set_ui(|state| {
                state.view = None;
                state.loading = false;
                state.saving = false;
            }),
            Err(message) => set_ui(|state| {
                state.alert = message;
                state.loading = false;
                state.saving = false;
            }),
        }
    });
}

fn submit_fps() {
    let text = UI
        .with(|slot| slot.borrow().as_ref().map(|ui| ui.peek().fps_input.clone()))
        .unwrap_or_default();
    let fps = match parse_fps_text(&text) {
        Ok(fps) => fps,
        Err(message) => {
            set_ui(|state| state.alert = message);
            return;
        }
    };
    set_ui(|state| {
        state.saving = true;
        state.alert.clear();
    });
    spawn_local(async move {
        match save_fps(fps).await {
            Ok(view) => set_ui(|state| {
                state.view = Some(view.clone());
                state.fps_input = view.live_output_fps.to_string();
                state.loading = false;
                state.saving = false;
                state.alert.clear();
            }),
            Err(message) if message == "missing" => set_ui(|state| {
                state.view = None;
                state.loading = false;
                state.saving = false;
            }),
            Err(message) => set_ui(|state| {
                state.alert = message;
                state.loading = false;
                state.saving = false;
            }),
        }
    });
}

fn parse_fps_text(text: &str) -> Result<u32, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("videos.fps must not be blank.".to_owned());
    }
    if trimmed.starts_with('-') || trimmed.contains('.') {
        return Err("videos.fps must be a positive whole number.".to_owned());
    }
    if !trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Err("videos.fps must be a positive whole number.".to_owned());
    }
    let fps = trimmed
        .parse::<u32>()
        .map_err(|_| "videos.fps must be a positive whole number.".to_owned())?;
    if fps == 0 {
        return Err("videos.fps must be greater than zero.".to_owned());
    }
    Ok(fps)
}

async fn load_view() -> Result<CaptureSettingsView, String> {
    let Some(transport) = session::transport() else {
        return Err("missing".to_owned());
    };
    let secret = take_secret(&[SETTINGS_PATH])
        .map_err(|()| "Could not reach capture settings.".to_owned())?;
    match transport.get_json(SETTINGS_PATH, secret.as_deref()).await {
        Ok(reply) if reply.status == 200 => serde_json::from_str(&reply.body)
            .map_err(|_| "Could not read capture settings.".to_owned()),
        Ok(reply) if reply.status == 404 => Err("missing".to_owned()),
        Ok(_) => Err("Could not load capture settings.".to_owned()),
        Err(()) => Err("Could not reach capture settings.".to_owned()),
    }
}

async fn save_fps(fps: u32) -> Result<CaptureSettingsView, String> {
    let Some(transport) = session::transport() else {
        return Err("missing".to_owned());
    };
    let body = serde_json::json!({ "videos": { "fps": fps } }).to_string();
    let secret = take_secret(&[SETTINGS_PATH])
        .map_err(|()| "Could not reach capture settings.".to_owned())?;
    match transport
        .patch_json(SETTINGS_PATH, &body, secret.as_deref())
        .await
    {
        Ok(reply) if (200..300).contains(&reply.status) => {
            serde_json::from_str::<serde_json::Value>(&reply.body)
                .ok()
                .and_then(|value| value.get("view").cloned())
                .and_then(|view| serde_json::from_value(view).ok())
                .ok_or_else(|| "Could not apply live output fps.".to_owned())
        }
        Ok(reply) if reply.status == 400 => {
            let message = serde_json::from_str::<serde_json::Value>(&reply.body)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/message")
                        .and_then(|text| text.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "videos.fps was rejected.".to_owned());
            Err(message)
        }
        Ok(_) => Err("Could not apply live output fps.".to_owned()),
        Err(()) => Err("Could not reach capture settings.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_fps_text;

    #[test]
    fn rejects_blank_zero_negative_and_non_numeric_fps() {
        for text in ["", "0", "-1", "1.5", "abc"] {
            assert!(parse_fps_text(text).is_err(), "{text}");
        }
    }

    #[test]
    fn accepts_positive_whole_numbers() {
        assert_eq!(parse_fps_text("5").unwrap(), 5);
        assert_eq!(parse_fps_text(" 12 ").unwrap(), 12);
    }
}
