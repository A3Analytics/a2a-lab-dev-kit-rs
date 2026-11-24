//! Agent configuration on `/ui/config/settings`.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen_futures::spawn_local;

use crate::shell::session::{self, take_secret};

mod chrome;
mod document;
mod labels;
mod semantic;
mod videos;

use crate::settings_semantic::{
    SemanticDraft, semantic_draft_from_document, surgical_semantic_changes,
};
use chrome::{
    AlertKind, CheckField, SectionCard, SelectField, SettingsAlert, StickyActions, TextAreaField,
    TextField,
};
use document::{
    CHAT_MODEL, CHAT_PREAMBLE, CHAT_PROVIDER, CoreDraft, LOGGING_ENABLED, LOGGING_FILTER,
    LOGGING_FORMAT, LOGGING_LEVEL, LOGGING_OTEL, SettingsSnapshot, SettingsWriteError,
    apply_default_chat_preamble, classify_settings_write, draft_from_document, parse_snapshot,
    patch_envelope,
    persistence_message, restart_required, surgical_core_changes, writable,
};
use semantic::SemanticPanel;
use videos::{
    SourceDraft, VideoDraft, VideosPanel, surgical_video_changes, video_draft_from_document,
};

const SETTINGS_PATH: &str = "/config/settings";

#[derive(Clone, PartialEq)]
pub struct SettingsUi {
    pub alert: String,
    pub alert_kind: AlertKind,
    pub view: Option<document::CaptureSettingsView>,
    pub loading: bool,
    pub saving: bool,
    pub persistence: String,
    pub revision: Option<String>,
    pub pending_restart: Vec<String>,
    pub document: Value,
    pub draft: CoreDraft,
    pub default_chat_preamble: String,
    pub video: VideoDraft,
    pub semantic: SemanticDraft,
    pub errors: BTreeMap<String, String>,
}

impl SettingsUi {
    pub fn new() -> Self {
        Self {
            alert: String::new(),
            alert_kind: AlertKind::Destructive,
            view: None,
            loading: false,
            saving: false,
            persistence: "unconfigured".to_owned(),
            revision: None,
            pending_restart: Vec::new(),
            document: Value::Object(Default::default()),
            draft: CoreDraft::empty(),
            default_chat_preamble: String::new(),
            video: VideoDraft::empty(),
            semantic: SemanticDraft::empty(),
            errors: BTreeMap::new(),
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
    let locked = !writable(&state.persistence);
    let persist_copy = persistence_message(&state.persistence).unwrap_or("");
    let pending = state.pending_restart.join(", ");
    let pending_label = if pending.is_empty() {
        "none".to_owned()
    } else {
        pending.clone()
    };
    let alert_hidden = state.alert.is_empty();
    rsx! {
        section {
            id: "config-settings-panel",
            class: "config-settings",
            hidden: !visible,
            SettingsAlert {
                id: "config-settings-alert".to_owned(),
                kind: state.alert_kind,
                title: alert_title(state.alert_kind),
                body: state.alert.clone(),
                hidden: alert_hidden,
            }
            if state.loading {
                p { id: "config-settings-status", role: "status", "Loading capture settings." }
            } else {
                SettingsAlert {
                    id: "settings-persistence".to_owned(),
                    kind: AlertKind::Info,
                    title: "Persistence".to_owned(),
                    body: persist_copy.to_owned(),
                    hidden: persist_copy.is_empty(),
                }
                dl { class: "config-settings-meta",
                    div {
                        dt { "Pending restart" }
                        dd { id: "settings-pending-restart", "{pending_label}" }
                    }
                }
                SettingsAlert {
                    id: "settings-restart-alert".to_owned(),
                    kind: AlertKind::Warning,
                    title: "Restart required".to_owned(),
                    body: format!("Saved values are not live until the agent restarts: {pending}"),
                    hidden: pending.is_empty(),
                }
                form {
                    id: "settings-core-form",
                    class: "settings-core-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_core();
                    },
                    onreset: move |event| {
                        event.prevent_default();
                        cancel_core();
                    },
                    ChatSection {
                        draft: state.draft.clone(),
                        default_preamble: state.default_chat_preamble.clone(),
                        errors: state.errors.clone(),
                        locked,
                    }
                    LoggingSection { draft: state.draft.clone(), errors: state.errors.clone(), locked }
                    VideosPanel {
                        draft: state.video.clone(),
                        view: state.view.clone(),
                        errors: state.errors.clone(),
                        persistence: state.persistence.clone(),
                        saving: state.saving,
                    }
                    SemanticPanel {
                        draft: state.semantic.clone(),
                        errors: state.errors.clone(),
                        persistence: state.persistence.clone(),
                        saving: state.saving,
                    }
                    StickyActions { locked, saving: state.saving }
                }
            }
        }
    }
}

#[component]
fn ChatSection(
    draft: CoreDraft,
    default_preamble: String,
    errors: BTreeMap<String, String>,
    locked: bool,
) -> Element {
    let reset_to = default_preamble.clone();
    let show_reset = !default_preamble.is_empty();
    let already_default = draft.chat_preamble == default_preamble;
    rsx! {
        SectionCard {
            title: "Chat",
            description: "Chat provider, model, and preamble. Saved changes require an agent restart and are never applied live.",
            SelectField {
                id: "settings-chat-provider".to_string(),
                label: "chat.provider".to_string(),
                help: "Optional. One of bedrock, openai, or anthropic.".to_string(),
                error: error_text(&errors, CHAT_PROVIDER),
                restart: restart_required(CHAT_PROVIDER),
                locked,
                value: draft.chat_provider.clone(),
                options: vec![
                    (String::new(), "Not set (optional)".to_string()),
                    ("bedrock".to_string(), "bedrock".to_string()),
                    ("openai".to_string(), "openai".to_string()),
                    ("anthropic".to_string(), "anthropic".to_string()),
                ],
                oninput: move |value| edit_draft(|draft| draft.chat_provider = value),
            }
            TextField {
                id: "settings-chat-model".to_string(),
                label: "chat.model".to_string(),
                help: "Optional model identifier. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, CHAT_MODEL),
                restart: restart_required(CHAT_MODEL),
                locked,
                value: draft.chat_model.clone(),
                oninput: move |value| edit_draft(|draft| draft.chat_model = value),
            }
            TextAreaField {
                id: "settings-chat-preamble".to_string(),
                label: "chat.preamble".to_string(),
                help: "Exact preamble text. An omitted value starts as the built-in preamble. Blank and whitespace are kept. Saved changes require an agent restart.".to_string(),
                error: error_text(&errors, CHAT_PREAMBLE),
                restart: restart_required(CHAT_PREAMBLE),
                locked,
                value: draft.chat_preamble.clone(),
                oninput: move |value| edit_draft(|draft| {
                    draft.chat_preamble_set = true;
                    draft.chat_preamble = value;
                }),
            }
            if show_reset {
                button {
                    id: "settings-chat-preamble-reset",
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked || already_default,
                    onclick: move |_| {
                        let preamble = reset_to.clone();
                        edit_draft(move |draft| {
                            draft.chat_preamble_set = true;
                            draft.chat_preamble = preamble;
                        });
                    },
                    "Reset to default"
                }
            }
        }
    }
}

#[component]
fn LoggingSection(draft: CoreDraft, errors: BTreeMap<String, String>, locked: bool) -> Element {
    rsx! {
        SectionCard {
            title: "Logging",
            description: "Application logging stays off when omitted. Saved logging changes require an agent restart and are never applied live.",
            CheckField {
                id: "settings-logging-enabled".to_string(),
                label: "Enable logging".to_string(),
                help: "Application logging stays off when disabled.".to_string(),
                error: error_text(&errors, LOGGING_ENABLED),
                restart: restart_required(LOGGING_ENABLED),
                locked,
                checked: draft.logging_enabled,
                oninput: move |value| edit_draft(|draft| draft.logging_enabled = value),
            }
            SelectField {
                id: "settings-logging-level".to_string(),
                label: "logging.level".to_string(),
                help: "Default is info when logging is present and level is omitted.".to_string(),
                error: error_text(&errors, LOGGING_LEVEL),
                restart: restart_required(LOGGING_LEVEL),
                locked: locked || !draft.logging_enabled,
                value: draft.logging_level.clone(),
                options: level_options(),
                oninput: move |value| edit_draft(|draft| draft.logging_level = value),
            }
            SelectField {
                id: "settings-logging-format".to_string(),
                label: "logging.format".to_string(),
                help: "text is human-readable stderr. json is newline-delimited events.".to_string(),
                error: error_text(&errors, LOGGING_FORMAT),
                restart: restart_required(LOGGING_FORMAT),
                locked: locked || !draft.logging_enabled,
                value: draft.logging_format.clone(),
                options: vec![("text".to_string(), "text".to_string()), ("json".to_string(), "json".to_string())],
                oninput: move |value| edit_draft(|draft| draft.logging_format = value),
            }
            TextField {
                id: "settings-logging-filter".to_string(),
                label: "logging.filter".to_string(),
                help: "Optional tracing-subscriber directives appended after the crate default.".to_string(),
                error: error_text(&errors, LOGGING_FILTER),
                restart: restart_required(LOGGING_FILTER),
                locked: locked || !draft.logging_enabled,
                value: draft.logging_filter.clone(),
                oninput: move |value| edit_draft(|draft| draft.logging_filter = value),
            }
            TextField {
                id: "settings-logging-otel".to_string(),
                label: "logging.opentelemetry.endpoint".to_string(),
                help: "Optional OTLP gRPC endpoint. Leave blank to omit OpenTelemetry export.".to_string(),
                error: error_text(&errors, LOGGING_OTEL),
                restart: restart_required(LOGGING_OTEL),
                locked: locked || !draft.logging_enabled,
                value: draft.logging_otel.clone(),
                oninput: move |value| edit_draft(|draft| draft.logging_otel = value),
            }
        }
    }
}

fn level_options() -> Vec<(String, String)> {
    ["trace", "debug", "info", "warn", "error", "off"]
        .into_iter()
        .map(|level| (level.to_owned(), level.to_owned()))
        .collect()
}

fn error_text(errors: &BTreeMap<String, String>, field: &str) -> String {
    errors.get(field).cloned().unwrap_or_default()
}

fn alert_title(kind: AlertKind) -> String {
    match kind {
        AlertKind::Info => "Notice".to_owned(),
        AlertKind::Warning => "Review needed".to_owned(),
        AlertKind::Destructive => "Something failed".to_owned(),
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

fn edit_draft(edit: impl FnOnce(&mut CoreDraft)) {
    set_ui(|state| {
        edit(&mut state.draft);
        state.errors.clear();
    });
}

pub(super) fn edit_video(edit: impl FnOnce(&mut VideoDraft)) {
    set_ui(|state| {
        edit(&mut state.video);
        state.errors.clear();
    });
}

pub(super) fn edit_source(index: usize, edit: impl FnOnce(&mut SourceDraft)) {
    set_ui(|state| {
        if let Some(source) = state.video.sources.get_mut(index) {
            edit(source);
        }
        state.errors.clear();
    });
}

pub(super) fn edit_semantic(edit: impl FnOnce(&mut SemanticDraft)) {
    set_ui(|state| {
        edit(&mut state.semantic);
        state.errors.clear();
    });
}

fn cancel_core() {
    set_ui(|state| {
        state.draft = draft_from_document(&state.document);
        state.video = video_draft_from_document(&state.document);
        state.semantic = semantic_draft_from_document(&state.document);
        state.errors.clear();
        state.alert.clear();
    });
}

fn start_load() {
    set_ui(|state| {
        state.loading = true;
        state.alert.clear();
    });
    spawn_local(async {
        match load_snapshot().await {
            Ok(snapshot) => set_ui(|state| apply_snapshot(state, snapshot)),
            Err(message) if message == "missing" => set_ui(|state| {
                state.view = None;
                state.loading = false;
                state.saving = false;
            }),
            Err(message) => set_ui(|state| fail(state, message)),
        }
    });
}

fn submit_core() {
    let (revision, document, draft, video, semantic) = UI
        .with(|slot| {
            slot.borrow().as_ref().map(|ui| {
                let state = ui.peek();
                (
                    state.revision.clone(),
                    state.document.clone(),
                    state.draft.clone(),
                    state.video.clone(),
                    state.semantic.clone(),
                )
            })
        })
        .unwrap_or((
            None,
            Value::Null,
            CoreDraft::empty(),
            VideoDraft::empty(),
            SemanticDraft::empty(),
        ));
    let changes = match merge_settings_changes(&document, &draft, &video, &semantic) {
        Ok(changes) => changes,
        Err(errors) => {
            set_ui(|state| {
                state.errors = errors;
                state.alert.clear();
            });
            return;
        }
    };
    let Some(revision) = revision else {
        set_ui(|state| {
            fail(
                state,
                persistence_message(&state.persistence)
                    .unwrap_or("Settings cannot be saved.")
                    .to_owned(),
            );
        });
        return;
    };
    if changes.as_object().is_some_and(serde_json::Map::is_empty) {
        set_ui(|state| {
            state.errors.clear();
            state.alert.clear();
        });
        return;
    }
    set_ui(|state| {
        state.saving = true;
        state.errors.clear();
        state.alert.clear();
    });
    spawn_local(async move {
        match save_changes(&revision, &changes).await {
            Ok(snapshot) => set_ui(|state| {
                apply_snapshot(state, snapshot);
                state.alert_kind = AlertKind::Warning;
                state.alert =
                    "Saved. Changes requiring restart will not apply until the service restarts."
                        .to_owned();
            }),
            Err(SettingsWriteError::Stale(message)) => reload_after_conflict(message).await,
            Err(SettingsWriteError::Field { field, message }) => set_ui(|state| {
                state.errors.insert(field, message);
                state.saving = false;
                state.loading = false;
            }),
            Err(SettingsWriteError::Message(message)) => set_ui(|state| fail(state, message)),
        }
    });
}

fn merge_settings_changes(
    document: &Value,
    draft: &CoreDraft,
    video: &VideoDraft,
    semantic: &SemanticDraft,
) -> Result<Value, document::FieldErrors> {
    let core = surgical_core_changes(document, draft);
    let video = surgical_video_changes(document, video, semantic);
    let semantic = surgical_semantic_changes(document, semantic);
    let mut errors = document::FieldErrors::new();
    let mut changes = json!({});
    match core {
        Ok(value) => merge_patch(&mut changes, value),
        Err(field_errors) => errors.extend(field_errors),
    }
    match video {
        Ok(value) => merge_patch(&mut changes, value),
        Err(field_errors) => errors.extend(field_errors),
    }
    match semantic {
        Ok(value) => merge_patch(&mut changes, value),
        Err(field_errors) => errors.extend(field_errors),
    }
    if errors.is_empty() {
        Ok(changes)
    } else {
        Err(errors)
    }
}

fn merge_patch(target: &mut Value, patch: Value) {
    let Some(patch) = patch.as_object() else {
        return;
    };
    let Some(object) = target.as_object_mut() else {
        return;
    };
    for (key, value) in patch {
        object.insert(key.clone(), value.clone());
    }
}

async fn reload_after_conflict(message: String) {
    match load_snapshot().await {
        Ok(snapshot) => set_ui(|state| {
            apply_snapshot(state, snapshot);
            fail(state, message);
        }),
        Err(load_error) => set_ui(|state| fail(state, load_error)),
    }
}

fn apply_snapshot(state: &mut SettingsUi, snapshot: SettingsSnapshot) {
    state.view = snapshot.live.clone();
    state.document = snapshot.document;
    state.default_chat_preamble = snapshot.default_chat_preamble.clone().unwrap_or_default();
    state.draft = draft_from_document(&state.document);
    apply_default_chat_preamble(&mut state.draft, snapshot.default_chat_preamble.as_deref());
    state.video = video_draft_from_document(&state.document);
    state.semantic = semantic_draft_from_document(&state.document);
    state.revision = snapshot.revision;
    state.persistence = snapshot.persistence;
    state.pending_restart = snapshot.pending_restart;
    state.loading = false;
    state.saving = false;
    state.errors.clear();
    state.alert.clear();
}

fn fail(state: &mut SettingsUi, message: String) {
    state.alert_kind = AlertKind::Destructive;
    state.alert = message;
    state.loading = false;
    state.saving = false;
}

async fn load_snapshot() -> Result<SettingsSnapshot, String> {
    let Some(transport) = session::transport() else {
        return Err("missing".to_owned());
    };
    let secret = take_secret(&[SETTINGS_PATH])
        .map_err(|()| "Could not reach capture settings.".to_owned())?;
    match transport.get_json(SETTINGS_PATH, secret.as_deref()).await {
        Ok(reply) if reply.status == 200 => parse_snapshot(&reply.body),
        Ok(reply) if reply.status == 404 => Err("missing".to_owned()),
        Ok(_) => Err("Could not load capture settings.".to_owned()),
        Err(()) => Err("Could not reach capture settings.".to_owned()),
    }
}

async fn save_changes(
    revision: &str,
    changes: &Value,
) -> Result<SettingsSnapshot, SettingsWriteError> {
    let Some(transport) = session::transport() else {
        return Err(SettingsWriteError::Message("missing".to_owned()));
    };
    let body = patch_envelope(revision, changes).to_string();
    let secret = take_secret(&[SETTINGS_PATH]).map_err(|()| {
        SettingsWriteError::Message("Could not reach capture settings.".to_owned())
    })?;
    match transport
        .patch_json(SETTINGS_PATH, &body, secret.as_deref())
        .await
    {
        Ok(reply) => classify_settings_write(reply.status, &reply.body),
        Err(()) => Err(SettingsWriteError::Message(
            "Could not reach capture settings.".to_owned(),
        )),
    }
}
