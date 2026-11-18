//! Video source, cadence, retention, and DVR path editors.

mod source_semantic;

use dioxus::prelude::*;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wasm_bindgen_futures::spawn_local;

use super::chrome::{SectionCard, SelectField, TextField};
use super::document::{
    CaptureSettingsView, FieldErrors, parse_fps_text, reject_blank, restart_required, writable,
};
use crate::settings_semantic::SemanticDraft;
use crate::settings_semantic::source::{
    SourceSemanticDraft, from_value as source_semantic_from_value, patch as source_semantic_patch,
    validate as validate_source_semantic,
};
use crate::shell::session;
use source_semantic::SourceSemanticFields;

const CAMERA_DISCOVERY_PATH: &str = "/config/cameras";

pub const VIDEOS_FPS: &str = "videos.fps";
pub const VIDEOS_LIVE_RETENTION: &str = "videos.live_retention";
pub const VIDEOS_DVR_SAVE_INTERVAL: &str = "videos.dvr.save_interval";
pub const VIDEOS_DVR_RETENTION: &str = "videos.dvr.retention";
pub const VIDEOS_DVR_STORAGE_PATH: &str = "videos.dvr.storage_path";

const FORMATS: &[&str] = &["auto", "mjpeg", "yuy2", "yuyv"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDraft {
    pub present: bool,
    pub fps: String,
    pub live_retention: String,
    pub dvr_save_interval: String,
    pub dvr_retention: String,
    pub dvr_storage_path: String,
    pub sources: Vec<SourceDraft>,
    pub discovered: Vec<DiscoveredCamera>,
    pub discovering: bool,
    pub discovery_error: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DiscoveredCamera {
    pub index: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDraft {
    pub id: String,
    pub name: String,
    pub description: String,
    pub format: String,
    pub camera_kind: CameraKind,
    pub camera: String,
    pub width: String,
    pub height: String,
    pub capture_rate: String,
    pub semantic: SourceSemanticDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraKind {
    Omitted,
    DeviceIndex,
    CameraIdInt,
    CameraIdText,
}

impl VideoDraft {
    pub fn empty() -> Self {
        Self {
            present: false,
            fps: String::new(),
            live_retention: String::new(),
            dvr_save_interval: String::new(),
            dvr_retention: String::new(),
            dvr_storage_path: String::new(),
            sources: Vec::new(),
            discovered: Vec::new(),
            discovering: false,
            discovery_error: String::new(),
        }
    }
}

pub fn video_draft_from_document(document: &Value) -> VideoDraft {
    let Some(videos) = document.get("videos") else {
        return VideoDraft::empty();
    };
    let dvr = videos.get("dvr");
    VideoDraft {
        present: true,
        fps: number_or_empty(videos.get("fps")),
        live_retention: string_or_empty(videos.get("live_retention")),
        dvr_save_interval: string_or_empty(dvr.and_then(|value| value.get("save_interval"))),
        dvr_retention: string_or_empty(dvr.and_then(|value| value.get("retention"))),
        dvr_storage_path: string_or_empty(dvr.and_then(|value| value.get("storage_path"))),
        sources: videos
            .get("sources")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(source_from_value).collect())
            .unwrap_or_default(),
        discovered: Vec::new(),
        discovering: false,
        discovery_error: String::new(),
    }
}

pub fn surgical_video_changes(
    original: &Value,
    draft: &VideoDraft,
    semantic: &SemanticDraft,
) -> Result<Value, FieldErrors> {
    let errors = validate_video_draft(draft, semantic);
    if !errors.is_empty() {
        return Err(errors);
    }
    if !draft.present {
        return Ok(json!({}));
    }
    let videos = original.get("videos");
    let mut nested = Map::new();
    push_optional_u32(
        &mut nested,
        "fps",
        videos.and_then(|value| value.get("fps")),
        &draft.fps,
    );
    push_optional_string(
        &mut nested,
        "live_retention",
        videos.and_then(|value| value.get("live_retention")),
        &draft.live_retention,
    );
    dvr_change(
        videos.and_then(|value| value.get("dvr")),
        draft,
        &mut nested,
    );
    sources_change(
        videos.and_then(|value| value.get("sources")),
        &draft.sources,
        &mut nested,
    );
    let mut changes = Map::new();
    if !nested.is_empty() {
        changes.insert("videos".to_owned(), Value::Object(nested));
    }
    Ok(Value::Object(changes))
}

pub fn validate_video_draft(draft: &VideoDraft, semantic: &SemanticDraft) -> FieldErrors {
    let mut errors = FieldErrors::new();
    if !draft.present {
        return errors;
    }
    optional_fps(&mut errors, VIDEOS_FPS, &draft.fps);
    optional_whole_duration(&mut errors, VIDEOS_LIVE_RETENTION, &draft.live_retention);
    optional_positive_duration(
        &mut errors,
        VIDEOS_DVR_SAVE_INTERVAL,
        &draft.dvr_save_interval,
    );
    optional_whole_duration(&mut errors, VIDEOS_DVR_RETENTION, &draft.dvr_retention);
    reject_blank(
        &mut errors,
        VIDEOS_DVR_STORAGE_PATH,
        &draft.dvr_storage_path,
        "videos.dvr.storage_path must contain non-whitespace text.",
    );
    for (index, source) in draft.sources.iter().enumerate() {
        validate_source(&mut errors, index, source, semantic);
    }
    errors
}

#[component]
fn CameraDiscovery(draft: VideoDraft, locked: bool, saving: bool) -> Element {
    rsx! {
        div { class: "settings-source-discovery",
            button {
                id: "settings-videos-discover",
                class: "settings-button-outline",
                r#type: "button",
                disabled: locked || saving || draft.discovering,
                onclick: move |_| discover_cameras(),
                if draft.discovering { "Discovering cameras…" } else { "Add camera" }
            }
            if !draft.discovery_error.is_empty() {
                p { class: "settings-error", "{draft.discovery_error}" }
            }
            for camera in draft.discovered.iter() {
                {
                    let already_added = draft.sources.iter().any(|source| {
                        source.camera_kind == CameraKind::DeviceIndex
                            && source.camera == camera.index.to_string()
                    });
                    let index = camera.index;
                    let name = camera.name.clone();
                    rsx! {
                        div { class: "settings-source-discovery-row",
                            span { "{name} (device {index})" }
                            button {
                                id: "settings-videos-add-{index}",
                                class: "settings-button-outline",
                                r#type: "button",
                                disabled: locked || saving || already_added,
                                    onclick: move |_| add_discovered_camera(index),
                                if already_added { "Added" } else { "Add video" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn VideosPanel(
    draft: VideoDraft,
    view: Option<CaptureSettingsView>,
    errors: FieldErrors,
    persistence: String,
    saving: bool,
) -> Element {
    let locked = !writable(&persistence);
    if !draft.present {
        return rsx! {
            SectionCard {
                title: "Videos",
                description: "Add a video source from Nokhwa camera discovery.",
                CameraDiscovery { draft: draft.clone(), locked, saving }
            }
        };
    }
    let live_fps = view
        .as_ref()
        .map(|live| live.live_output_fps.to_string())
        .unwrap_or_else(|| "unavailable".to_owned());
    rsx! {
        SectionCard {
            title: "Videos",
            description: "Cadence, retention, storage path, and per-source capture identity. Save writes only changed keys.",
            CameraDiscovery { draft: draft.clone(), locked, saving }
            dl { class: "config-settings-meta",
                div {
                    dt { "Active live output fps" }
                    dd { id: "video-live-output-fps", "{live_fps}" }
                }
            }
            TextField {
                id: "settings-videos-fps".to_string(),
                label: "videos.fps".to_string(),
                help: "Live output cadence. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, VIDEOS_FPS),
                restart: restart_required(VIDEOS_FPS),
                live: true,
                locked,
                value: draft.fps.clone(),
                oninput: move |value| super::edit_video(move |video| video.fps = value),
            }
            TextField {
                id: "settings-videos-live-retention".to_string(),
                label: "videos.live_retention".to_string(),
                help: "Optional in-memory live window. Whole-second duration such as 30s.".to_string(),
                error: error_text(&errors, VIDEOS_LIVE_RETENTION),
                restart: restart_required(VIDEOS_LIVE_RETENTION),
                locked,
                value: draft.live_retention.clone(),
                oninput: move |value| super::edit_video(move |video| video.live_retention = value),
            }
            TextField {
                id: "settings-videos-dvr-save-interval".to_string(),
                label: "videos.dvr.save_interval".to_string(),
                help: "DVR persistence cadence. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, VIDEOS_DVR_SAVE_INTERVAL),
                restart: restart_required(VIDEOS_DVR_SAVE_INTERVAL),
                live: true,
                locked,
                value: draft.dvr_save_interval.clone(),
                oninput: move |value| super::edit_video(move |video| video.dvr_save_interval = value),
            }
            TextField {
                id: "settings-videos-dvr-retention".to_string(),
                label: "videos.dvr.retention".to_string(),
                help: "Optional disk retention. Whole-second duration such as 1h.".to_string(),
                error: error_text(&errors, VIDEOS_DVR_RETENTION),
                restart: restart_required(VIDEOS_DVR_RETENTION),
                locked,
                value: draft.dvr_retention.clone(),
                oninput: move |value| super::edit_video(move |video| video.dvr_retention = value),
            }
            TextField {
                id: "settings-videos-dvr-storage-path".to_string(),
                label: "videos.dvr.storage_path".to_string(),
                help: "Storage directory text as written. Relative and absolute paths are not resolved.".to_string(),
                error: error_text(&errors, VIDEOS_DVR_STORAGE_PATH),
                restart: restart_required(VIDEOS_DVR_STORAGE_PATH),
                locked,
                value: draft.dvr_storage_path.clone(),
                oninput: move |value| super::edit_video(move |video| video.dvr_storage_path = value),
            }
        }
        for (index , source) in draft.sources.iter().enumerate() {
            SourceCard {
                index,
                source: source.clone(),
                errors: errors.clone(),
                locked: locked || saving,
            }
        }
    }
}

#[component]
fn SourceCard(index: usize, source: SourceDraft, errors: FieldErrors, locked: bool) -> Element {
    let id = source.id.clone();
    rsx! {
        SectionCard {
            title: format!("Video source {id}"),
            description: "Capture identity and profile. The source id is not edited here.".to_string(),
            TextField {
                id: field_id(index, "id"),
                label: format!("videos.sources.{index}.id"),
                help: "Stable operator id. Identity is preserved across surgical array patches.".to_string(),
                error: error_text(&errors, &source_path(index, "id")),
                restart: true,
                locked: true,
                value: source.id.clone(),
                oninput: move |_| {},
            }
            TextField {
                id: field_id(index, "name"),
                label: format!("videos.sources.{index}.name"),
                help: "Optional display name. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "name")),
                restart: true,
                locked,
                value: source.name.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.name = value),
            }
            TextField {
                id: field_id(index, "description"),
                label: format!("videos.sources.{index}.description"),
                help: "Optional description. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "description")),
                restart: true,
                locked,
                value: source.description.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.description = value),
            }
            SelectField {
                id: field_id(index, "format"),
                label: format!("videos.sources.{index}.format"),
                help: "Camera input. Not set keeps the omitted auto default. yuyv is preserved until changed.".to_string(),
                error: error_text(&errors, &source_path(index, "format")),
                restart: true,
                locked,
                value: source.format.clone(),
                options: format_options(&source.format),
                oninput: move |value| super::edit_source(index, move |source| source.format = value),
            }
            TextField {
                id: field_id(index, "camera"),
                label: camera_label(index, source.camera_kind),
                help: "Index or camera name/UID. Leave blank to omit. device_index aliases stay device_index.".to_string(),
                error: error_text(&errors, &source_path(index, "camera")),
                restart: true,
                locked,
                value: source.camera.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.camera = value),
            }
            TextField {
                id: field_id(index, "width"),
                label: format!("videos.sources.{index}.width"),
                help: "Optional frame width. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "width")),
                restart: true,
                locked,
                value: source.width.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.width = value),
            }
            TextField {
                id: field_id(index, "height"),
                label: format!("videos.sources.{index}.height"),
                help: "Optional frame height. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "height")),
                restart: true,
                locked,
                value: source.height.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.height = value),
            }
            TextField {
                id: field_id(index, "capture_rate"),
                label: format!("videos.sources.{index}.capture_rate"),
                help: "Optional capture frame rate. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "capture_rate")),
                restart: true,
                locked,
                value: source.capture_rate.clone(),
                oninput: move |value| super::edit_source(index, move |source| source.capture_rate = value),
            }
            SourceSemanticFields {
                index,
                source: source.clone(),
                errors: errors.clone(),
                locked,
            }
        }
    }
}

fn source_from_value(value: &Value) -> SourceDraft {
    let (camera_kind, camera) = camera_from_value(value);
    SourceDraft {
        id: string_or_empty(value.get("id")),
        name: string_or_empty(value.get("name")),
        description: string_or_empty(value.get("description")),
        format: string_or_empty(value.get("format")),
        camera_kind,
        camera,
        width: number_or_empty(value.get("width")),
        height: number_or_empty(value.get("height")),
        capture_rate: number_or_empty(value.get("capture_rate")),
        semantic: source_semantic_from_value(value),
    }
}

pub(super) fn discover_cameras() {
    super::edit_video(|video| {
        video.discovering = true;
        video.discovery_error.clear();
    });
    spawn_local(async {
        let result = load_discovered_cameras().await;
        super::edit_video(|video| {
            video.discovering = false;
            match result {
                Ok(cameras) => {
                    video.discovered = cameras;
                    video.discovery_error.clear();
                }
                Err(message) => video.discovery_error = message,
            }
        });
    });
}

pub(super) fn add_discovered_camera(index: u32) {
    super::edit_video(|video| {
        video.present = true;
        if video.sources.iter().any(|source| {
            source.camera_kind == CameraKind::DeviceIndex && source.camera == index.to_string()
        }) {
            return;
        }
        let base_id = format!("camera-{index}");
        let mut id = base_id.clone();
        let mut suffix = 2;
        while video.sources.iter().any(|source| source.id == id) {
            id = format!("{base_id}-{suffix}");
            suffix += 1;
        }
        let name = video
            .discovered
            .iter()
            .find(|camera| camera.index == index)
            .map(|camera| camera.name.clone())
            .unwrap_or_else(|| id.clone());
        video.sources.push(SourceDraft {
            id,
            name,
            description: String::new(),
            format: "auto".to_owned(),
            camera_kind: CameraKind::DeviceIndex,
            camera: index.to_string(),
            width: String::new(),
            height: String::new(),
            capture_rate: String::new(),
            semantic: SourceSemanticDraft::empty(),
        });
    });
}

async fn load_discovered_cameras() -> Result<Vec<DiscoveredCamera>, String> {
    let Some(transport) = session::transport() else {
        return Err("Camera discovery is unavailable.".to_owned());
    };
    let secret = session::take_secret(&[CAMERA_DISCOVERY_PATH])
        .map_err(|()| "Could not reach camera discovery.".to_owned())?;
    let reply = transport
        .get_json(CAMERA_DISCOVERY_PATH, secret.as_deref())
        .await
        .map_err(|()| "Could not reach camera discovery.".to_owned())?;
    if reply.status != 200 {
        return Err("Camera discovery is unavailable.".to_owned());
    }
    serde_json::from_str::<serde_json::Value>(&reply.body)
        .ok()
        .and_then(|body| body.get("cameras").cloned())
        .and_then(|cameras| serde_json::from_value(cameras).ok())
        .ok_or_else(|| "Camera discovery returned an invalid response.".to_owned())
}

fn camera_from_value(value: &Value) -> (CameraKind, String) {
    if let Some(index) = value.get("device_index") {
        return (CameraKind::DeviceIndex, json_number_or_text(index));
    }
    match value.get("camera_id") {
        Some(Value::Number(number)) => (CameraKind::CameraIdInt, number.to_string()),
        Some(Value::String(text)) => (CameraKind::CameraIdText, text.clone()),
        Some(other) => (CameraKind::CameraIdText, other.to_string()),
        None => (CameraKind::Omitted, String::new()),
    }
}

fn dvr_change(dvr: Option<&Value>, draft: &VideoDraft, videos: &mut Map<String, Value>) {
    let mut nested = Map::new();
    push_optional_string(
        &mut nested,
        "save_interval",
        dvr.and_then(|value| value.get("save_interval")),
        &draft.dvr_save_interval,
    );
    push_optional_string(
        &mut nested,
        "retention",
        dvr.and_then(|value| value.get("retention")),
        &draft.dvr_retention,
    );
    push_optional_string(
        &mut nested,
        "storage_path",
        dvr.and_then(|value| value.get("storage_path")),
        &draft.dvr_storage_path,
    );
    if nested.is_empty() {
        return;
    }
    videos.insert("dvr".to_owned(), Value::Object(nested));
}

fn sources_change(
    original: Option<&Value>,
    drafts: &[SourceDraft],
    videos: &mut Map<String, Value>,
) {
    let items = original.and_then(Value::as_array);
    if items.is_none() && drafts.is_empty() {
        return;
    }
    let mut patch = Vec::new();
    let mut last_changed = None;
    for (index, draft) in drafts.iter().enumerate() {
        let nested = source_patch(items.and_then(|items| items.get(index)), draft);
        if nested.is_empty() {
            patch.push(json!({}));
        } else {
            last_changed = Some(index);
            patch.push(Value::Object(nested));
        }
    }
    let Some(last) = last_changed else {
        return;
    };
    patch.truncate(last + 1);
    videos.insert("sources".to_owned(), Value::Array(patch));
}

fn source_patch(original: Option<&Value>, draft: &SourceDraft) -> Map<String, Value> {
    let mut nested = Map::new();
    if original.is_none() {
        nested.insert("id".to_owned(), Value::String(draft.id.clone()));
    }
    push_optional_string(
        &mut nested,
        "name",
        original.and_then(|value| value.get("name")),
        &draft.name,
    );
    push_optional_string(
        &mut nested,
        "description",
        original.and_then(|value| value.get("description")),
        &draft.description,
    );
    push_optional_string(
        &mut nested,
        "format",
        original.and_then(|value| value.get("format")),
        &draft.format,
    );
    camera_patch(original, draft, &mut nested);
    push_optional_u32(
        &mut nested,
        "width",
        original.and_then(|value| value.get("width")),
        &draft.width,
    );
    push_optional_u32(
        &mut nested,
        "height",
        original.and_then(|value| value.get("height")),
        &draft.height,
    );
    push_optional_u32(
        &mut nested,
        "capture_rate",
        original.and_then(|value| value.get("capture_rate")),
        &draft.capture_rate,
    );
    nested.extend(source_semantic_patch(original, &draft.semantic));
    nested
}

fn camera_patch(original: Option<&Value>, draft: &SourceDraft, nested: &mut Map<String, Value>) {
    let next = draft.camera.trim();
    match draft.camera_kind {
        CameraKind::Omitted if next.is_empty() => {}
        CameraKind::Omitted => insert_camera_id(nested, next),
        CameraKind::DeviceIndex if next.is_empty() => {
            nested.insert("device_index".to_owned(), Value::Null);
        }
        CameraKind::DeviceIndex => {
            if let Ok(index) = next.parse::<u32>() {
                let current = original.and_then(|value| value.get("device_index"));
                if !number_matches(current, index) {
                    nested.insert("device_index".to_owned(), json!(index));
                }
            } else {
                nested.insert("device_index".to_owned(), Value::Null);
                insert_camera_id(nested, next);
            }
        }
        CameraKind::CameraIdInt if next.is_empty() => {
            nested.insert("camera_id".to_owned(), Value::Null);
        }
        CameraKind::CameraIdInt => {
            if let Ok(index) = next.parse::<u32>() {
                let current = original.and_then(|value| value.get("camera_id"));
                if !number_matches(current, index) {
                    nested.insert("camera_id".to_owned(), json!(index));
                }
            } else {
                insert_camera_id(nested, next);
            }
        }
        CameraKind::CameraIdText if next.is_empty() => {
            nested.insert("camera_id".to_owned(), Value::Null);
        }
        CameraKind::CameraIdText => {
            let current = original
                .and_then(|value| value.get("camera_id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if current != next {
                nested.insert("camera_id".to_owned(), json!(next));
            }
        }
    }
}

fn insert_camera_id(nested: &mut Map<String, Value>, next: &str) {
    if let Ok(index) = next.parse::<u32>() {
        nested.insert("camera_id".to_owned(), json!(index));
    } else {
        nested.insert("camera_id".to_owned(), json!(next));
    }
}

fn validate_source(
    errors: &mut FieldErrors,
    index: usize,
    source: &SourceDraft,
    semantic: &SemanticDraft,
) {
    if source.id.is_empty() {
        errors.insert(
            source_path(index, "id"),
            "videos.sources.id is required.".to_owned(),
        );
    }
    reject_blank(
        errors,
        &source_path(index, "name"),
        &source.name,
        "videos.sources.name must contain non-whitespace text.",
    );
    reject_blank(
        errors,
        &source_path(index, "description"),
        &source.description,
        "videos.sources.description must contain non-whitespace text.",
    );
    if !source.format.is_empty() && !FORMATS.contains(&source.format.as_str()) {
        errors.insert(
            source_path(index, "format"),
            "videos.sources.format is not a supported camera input.".to_owned(),
        );
    }
    if source.camera_kind == CameraKind::DeviceIndex
        && !source.camera.trim().is_empty()
        && source.camera.trim().parse::<u32>().is_err()
        && !source
            .camera
            .chars()
            .any(|character| !character.is_whitespace())
    {
        errors.insert(
            source_path(index, "camera"),
            "videos.sources.device_index must be a non-negative integer.".to_owned(),
        );
    }
    optional_positive_u32(errors, &source_path(index, "width"), &source.width);
    optional_positive_u32(errors, &source_path(index, "height"), &source.height);
    optional_positive_u32(
        errors,
        &source_path(index, "capture_rate"),
        &source.capture_rate,
    );
    validate_source_semantic(errors, index, &source.semantic, semantic);
}

fn optional_fps(errors: &mut FieldErrors, field: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    if let Err(message) = parse_fps_text(value) {
        errors.insert(field.to_owned(), message);
    }
}

fn optional_positive_u32(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match parse_fps_text(trimmed) {
        Ok(_) => {}
        Err(_) => {
            errors.insert(
                field.to_owned(),
                format!("{field} must be a positive whole number."),
            );
        }
    }
}

fn optional_whole_duration(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if !is_whole_second_duration(trimmed) {
        errors.insert(
            field.to_owned(),
            format!("{field} must be a positive whole-second duration."),
        );
    }
}

fn optional_positive_duration(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if !is_positive_duration(trimmed) {
        errors.insert(
            field.to_owned(),
            format!("{field} must be a positive duration."),
        );
    }
}

fn is_whole_second_duration(text: &str) -> bool {
    parse_duration_units(text, false).is_some_and(|nanos| nanos.is_multiple_of(1_000_000_000))
}

pub(super) fn is_positive_duration(text: &str) -> bool {
    parse_duration_units(text, true).is_some()
}

fn parse_duration_units(text: &str, subsecond: bool) -> Option<u128> {
    if text.starts_with('-') {
        return None;
    }
    let mut rest = text;
    let mut nanos: u128 = 0;
    let mut saw = false;
    while !rest.is_empty() {
        let (amount, unit, scale, next) = parse_duration_amount(rest)?;
        if !subsecond && matches!(unit, "ms" | "us" | "ns") {
            return None;
        }
        rest = next;
        nanos = nanos.checked_add(fractional_nanos(amount, scale)?)?;
        saw = true;
    }
    if saw && nanos > 0 { Some(nanos) } else { None }
}

fn parse_duration_amount(text: &str) -> Option<(f64, &'static str, u128, &str)> {
    let mut end = 0;
    let mut dot = false;
    for (index, character) in text.char_indices() {
        if character.is_ascii_digit() {
            end = index + 1;
            continue;
        }
        if character == '.' && !dot {
            dot = true;
            end = index + 1;
            continue;
        }
        break;
    }
    if end == 0 || text[..end].ends_with('.') {
        return None;
    }
    let amount: f64 = text[..end].parse().ok()?;
    let rest = &text[end..];
    let (unit, scale) = duration_unit(rest)?;
    Some((amount, unit, scale, &rest[unit.len()..]))
}

fn fractional_nanos(amount: f64, scale: u128) -> Option<u128> {
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let nanos = amount * scale as f64;
    if !nanos.is_finite() || nanos <= 0.0 {
        return None;
    }
    Some(nanos.round() as u128)
}

fn duration_unit(text: &str) -> Option<(&'static str, u128)> {
    const SECOND: u128 = 1_000_000_000;
    [
        ("ms", 1_000_000u128),
        ("us", 1_000u128),
        ("ns", 1u128),
        ("s", SECOND),
        ("m", 60 * SECOND),
        ("h", 60 * 60 * SECOND),
        ("d", 24 * 60 * 60 * SECOND),
        ("w", 7 * 24 * 60 * 60 * SECOND),
    ]
    .into_iter()
    .find(|(unit, _)| text.starts_with(unit))
}

fn push_optional_string(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&Value>,
    next: &str,
) {
    let next = next.trim();
    let current = current.and_then(Value::as_str);
    match (current, next) {
        (None, "") => {}
        (Some(current), next) if current == next => {}
        (Some(_), "") => {
            target.insert(key.to_owned(), Value::Null);
        }
        (_, next) => {
            target.insert(key.to_owned(), json!(next));
        }
    }
}

fn push_optional_u32(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&Value>,
    next: &str,
) {
    let next = next.trim();
    if next.is_empty() {
        if current.is_some() {
            target.insert(key.to_owned(), Value::Null);
        }
        return;
    }
    let Ok(parsed) = parse_fps_text(next) else {
        return;
    };
    if number_matches(current, parsed) {
        return;
    }
    target.insert(key.to_owned(), json!(parsed));
}

fn number_matches(current: Option<&Value>, next: u32) -> bool {
    current
        .and_then(Value::as_u64)
        .is_some_and(|value| value == u64::from(next))
}

pub(super) fn error_text(errors: &FieldErrors, field: &str) -> String {
    errors.get(field).cloned().unwrap_or_default()
}

pub(super) fn field_id(index: usize, field: &str) -> String {
    format!("settings-videos-source-{index}-{field}")
}

pub(super) fn source_path(index: usize, field: &str) -> String {
    format!("videos.sources.{index}.{field}")
}

fn camera_label(index: usize, kind: CameraKind) -> String {
    match kind {
        CameraKind::DeviceIndex => format!("videos.sources.{index}.device_index"),
        _ => format!("videos.sources.{index}.camera_id"),
    }
}

fn format_options(current: &str) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), "Not set (auto)".to_owned())];
    for format in FORMATS {
        options.push(((*format).to_owned(), (*format).to_owned()));
    }
    if !current.is_empty() && !FORMATS.contains(&current) {
        options.push((current.to_owned(), current.to_owned()));
    }
    options
}

pub(super) fn string_or_empty(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn number_or_empty(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn json_number_or_text(value: &Value) -> String {
    match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = r#"{
        "schema_version": 1,
        "videos": {
            "sources": [
                {"id": "camera", "format": "yuyv", "device_index": 6},
                {"id": "detail", "name": "Detail", "camera_id": "emeet", "width": 1280, "height": 720}
            ],
            "fps": 5,
            "dvr": {"storage_path": "./dvr"}
        }
    }"#;

    fn original() -> Value {
        serde_json::from_str(ORIGINAL).unwrap()
    }

    #[test]
    fn omitted_videos_emits_no_patch() {
        let document = json!({ "schema_version": 1 });
        let draft = video_draft_from_document(&document);
        assert!(!draft.present);
        assert_eq!(
            surgical_video_changes(&document, &draft, &SemanticDraft::empty()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn surgical_source_patch_keeps_id_alias_and_omitted_keys() {
        let original = original();
        let mut draft = video_draft_from_document(&original);
        draft.sources[1].name = "Closer".to_owned();
        let changes = surgical_video_changes(&original, &draft, &SemanticDraft::empty()).unwrap();
        assert_eq!(
            changes,
            json!({ "videos": { "sources": [{}, { "name": "Closer" }] } })
        );
        assert!(
            changes["videos"]["sources"][0]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(changes["videos"].get("fps").is_none());
        assert!(changes["videos"]["sources"][1].get("id").is_none());
        assert!(changes["videos"]["sources"][1].get("camera_id").is_none());
    }

    #[test]
    fn clearing_optional_keys_emits_null_and_paths_stay_literal() {
        let original = original();
        let mut draft = video_draft_from_document(&original);
        draft.fps.clear();
        draft.dvr_storage_path = "/var/dvr".to_owned();
        draft.live_retention = "45s".to_owned();
        let changes = surgical_video_changes(&original, &draft, &SemanticDraft::empty()).unwrap();
        assert_eq!(changes["videos"]["fps"], Value::Null);
        assert_eq!(changes["videos"]["live_retention"], json!("45s"));
        assert_eq!(changes["videos"]["dvr"]["storage_path"], json!("/var/dvr"));
        assert!(changes["videos"]["dvr"].get("retention").is_none());
        assert!(changes["videos"]["dvr"].get("save_interval").is_none());
    }

    #[test]
    fn device_index_and_yuyv_are_not_rewritten() {
        let original = original();
        let mut draft = video_draft_from_document(&original);
        draft.sources[0].camera = "7".to_owned();
        let changes = surgical_video_changes(&original, &draft, &SemanticDraft::empty()).unwrap();
        assert_eq!(
            changes,
            json!({ "videos": { "sources": [{ "device_index": 7 }] } })
        );
        assert!(changes["videos"]["sources"][0].get("format").is_none());
        assert!(changes["videos"]["sources"][0].get("id").is_none());
    }

    #[test]
    fn live_fields_do_not_require_restart() {
        assert!(!restart_required(VIDEOS_FPS));
        assert!(!restart_required(VIDEOS_DVR_SAVE_INTERVAL));
        assert!(restart_required(VIDEOS_DVR_STORAGE_PATH));
        assert!(restart_required(VIDEOS_LIVE_RETENTION));
        assert!(restart_required("videos.sources.0.name"));
        assert!(restart_required(
            "videos.sources.0.semantic_indexing.description"
        ));
        assert!(restart_required(
            "videos.sources.0.semantic_indexing.embeddings"
        ));
        assert!(restart_required(VIDEOS_DVR_RETENTION));
    }

    #[test]
    fn invalid_video_values_do_not_build_a_patch() {
        let original = original();
        let mut draft = video_draft_from_document(&original);
        draft.fps = "0".to_owned();
        draft.dvr_save_interval = "nope".to_owned();
        draft.dvr_storage_path = "   ".to_owned();
        let errors =
            surgical_video_changes(&original, &draft, &SemanticDraft::empty()).unwrap_err();
        assert!(errors.contains_key(VIDEOS_FPS));
        assert!(errors.contains_key(VIDEOS_DVR_SAVE_INTERVAL));
        assert!(errors.contains_key(VIDEOS_DVR_STORAGE_PATH));
    }
}
