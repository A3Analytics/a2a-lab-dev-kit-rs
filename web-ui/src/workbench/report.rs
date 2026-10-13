//! Structured command results and google.rpc.Status details.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use super::tree::{self, Node};
use super::{BROWSER_PREVIEW_MAX_BYTES, HistoryItem, Operation, PREVIEW_MEDIA_TYPES};
use crate::codec::{self, Reply};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Success,
    Rpc,
    TaskFailed,
    Unauthorized,
    Forbidden,
    Transport,
    Unavailable,
    Unsupported,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonLine {
    pub reason: String,
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLine {
    pub field: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correlation {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePreview {
    pub media_type: String,
    pub encoded_chars: usize,
    pub decoded_bytes: Option<usize>,
    pub preview_url: Option<String>,
    pub withheld: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub kind: Kind,
    pub operation: String,
    pub protocol_task_id: String,
    pub protocol_context_id: String,
    pub protocol_state: String,
    pub lab_run_id: String,
    pub lab_run_state: String,
    pub status: String,
    pub code: String,
    pub message: String,
    pub reasons: Vec<ReasonLine>,
    pub fields: Vec<FieldLine>,
    pub correlations: Vec<Correlation>,
    pub tree: Node,
    pub image: Option<ImagePreview>,
}

struct Payload {
    operation: String,
    result: Value,
}

struct DetailSet {
    reasons: Vec<ReasonLine>,
    fields: Vec<FieldLine>,
    correlations: Vec<Correlation>,
}

impl Kind {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Rpc => "rpc",
            Self::TaskFailed => "task-failed",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::Transport => "transport",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
            Self::Unexpected => "unexpected",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::Rpc => "Status error",
            Self::TaskFailed => "Task failed",
            Self::Unauthorized => "Unauthorized",
            Self::Forbidden => "Forbidden",
            Self::Transport => "Transport failure",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
            Self::Unexpected => "Unexpected result",
        }
    }
}

impl Report {
    fn blank(kind: Kind, operation: &str) -> Self {
        Self {
            kind,
            operation: operation.to_owned(),
            protocol_task_id: String::new(),
            protocol_context_id: String::new(),
            protocol_state: String::new(),
            lab_run_id: String::new(),
            lab_run_state: String::new(),
            status: String::new(),
            code: String::new(),
            message: String::new(),
            reasons: Vec::new(),
            fields: Vec::new(),
            correlations: Vec::new(),
            tree: Node::Text(String::new()),
            image: None,
        }
    }
}

pub fn transport_report(operation: Operation) -> Report {
    let mut report = Report::blank(Kind::Transport, operation.name());
    report.message = "The browser could not reach this origin.".to_owned();
    report
}

pub fn interpret(requested: Operation, reply: &Reply, secret: Option<&str>) -> Report {
    if reply.status == 401 {
        return auth_report(Kind::Unauthorized, requested, reply, secret);
    }
    if reply.status == 403 {
        return auth_report(Kind::Forbidden, requested, reply, secret);
    }
    let Ok(value) = serde_json::from_str::<Value>(&reply.body) else {
        return unexpected(requested, "The response could not be read.");
    };
    let report = if value.get("error").is_some() {
        rpc_report(requested, &value)
    } else if let Some(task) = task_value(&value) {
        task_report(requested, task)
    } else {
        unexpected(requested, "The response did not contain a lab result.")
    };
    redact_report(report, secret)
}

pub fn history_item(
    operation: Operation,
    params: &Value,
    report: &Report,
    secret: Option<&str>,
) -> HistoryItem {
    let params = redact_json(params, secret);
    let summary = codec::redact(&summary_line(operation, &params, report), secret);
    HistoryItem {
        operation,
        params,
        summary,
    }
}

fn auth_report(kind: Kind, requested: Operation, reply: &Reply, secret: Option<&str>) -> Report {
    let mut report = if let Ok(value) = serde_json::from_str::<Value>(&reply.body) {
        rpc_report(requested, &value)
    } else {
        let mut report = Report::blank(kind, requested.name());
        report.message = "The agent rejected the call.".to_owned();
        report
    };
    report.kind = kind;
    redact_report(report, secret)
}

fn rpc_report(requested: Operation, value: &Value) -> Report {
    let Some(error) = value.get("error").and_then(Value::as_object) else {
        return unexpected(requested, "The status error could not be read.");
    };
    let details = read_details(error);
    let status = text_of(error.get("status"));
    let mut report = Report::blank(classify_rpc(&status, &details.reasons), requested.name());
    report.status = status;
    report.code = text_of(error.get("code"));
    report.message = text_of(error.get("message"));
    report.reasons = details.reasons;
    report.fields = details.fields;
    report.correlations = details.correlations;
    report.tree = tree::from_value(value.get("error").unwrap_or(&Value::Null));
    report
}

fn task_report(requested: Operation, task: &Value) -> Report {
    let state = task
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let payloads = payloads_of(task);
    let mut report = Report::blank(Kind::Success, requested.name());
    report.protocol_task_id = text_of(task.get("id"));
    report.protocol_context_id = text_of(task.get("contextId"));
    report.protocol_state = state.clone();
    report.message = status_message(task.get("status"));
    match combine(&payloads) {
        Ok(payload) => fill_payload(&mut report, requested, &payload, &state),
        Err(()) => {
            report.kind = if failed_state(&state) {
                Kind::TaskFailed
            } else {
                Kind::Unexpected
            };
            report.tree =
                tree::from_value(&Value::Array(payloads.iter().map(payload_value).collect()));
            if report.message.is_empty() {
                report.message = "The lab result could not be combined.".to_owned();
            }
        }
    }
    report
}

fn fill_payload(report: &mut Report, requested: Operation, payload: &Payload, state: &str) {
    let (image, shown) = present_result(&payload.operation, &payload.result);
    report.operation = payload.operation.clone();
    report.tree = tree::from_value(&shown);
    report.image = image;
    if requested.is_task() {
        let (run_id, run_state) = lab_run(&payload.result);
        report.lab_run_id = run_id;
        report.lab_run_state = run_state;
    }
    report.kind = if failed_state(state) {
        Kind::TaskFailed
    } else if payload.operation != requested.name() {
        Kind::Unexpected
    } else {
        Kind::Success
    };
}

fn unexpected(requested: Operation, message: &str) -> Report {
    let mut report = Report::blank(Kind::Unexpected, requested.name());
    report.message = message.to_owned();
    report
}

fn classify_rpc(status: &str, reasons: &[ReasonLine]) -> Kind {
    let unavailable = status == "UNAVAILABLE"
        || status == "INTERNAL"
        || reasons.iter().any(|line| line.reason == "INTERNAL_ERROR");
    if unavailable {
        return Kind::Unavailable;
    }
    let unsupported = status == "UNIMPLEMENTED"
        || reasons.iter().any(|line| {
            line.reason == "CONTENT_TYPE_NOT_SUPPORTED" || line.reason == "UNIMPLEMENTED"
        });
    if unsupported {
        Kind::Unsupported
    } else {
        Kind::Rpc
    }
}

fn failed_state(state: &str) -> bool {
    matches!(state, "TASK_STATE_FAILED" | "TASK_STATE_REJECTED")
}

fn task_value(value: &Value) -> Option<&Value> {
    if let Some(task) = value.get("task") {
        return Some(task);
    }
    (value.get("id").is_some() && value.get("contextId").is_some()).then_some(value)
}

fn payloads_of(task: &Value) -> Vec<Payload> {
    let mut payloads = Vec::new();
    let Some(artifacts) = task.get("artifacts").and_then(Value::as_array) else {
        return payloads;
    };
    for artifact in artifacts {
        let Some(parts) = artifact.get("parts").and_then(Value::as_array) else {
            continue;
        };
        for part in parts {
            if let Some(payload) = part.get("data").and_then(read_payload) {
                payloads.push(payload);
            }
        }
    }
    payloads
}

fn read_payload(value: &Value) -> Option<Payload> {
    let object = value.as_object()?;
    let operation = object.get("operation")?.as_str()?.to_owned();
    let result = object.get("result")?.clone();
    result.as_object()?;
    Some(Payload { operation, result })
}

fn combine(payloads: &[Payload]) -> Result<Payload, ()> {
    if payloads.is_empty() || mixed(payloads) {
        return Err(());
    }
    if payloads.len() == 1 {
        return Ok(clone_payload(&payloads[0]));
    }
    if payloads.iter().all(has_items) {
        return Ok(merged_page(payloads));
    }
    Err(())
}

fn mixed(payloads: &[Payload]) -> bool {
    payloads
        .iter()
        .any(|payload| payload.operation != payloads[0].operation)
}

fn has_items(payload: &Payload) -> bool {
    payload
        .result
        .get("items")
        .and_then(Value::as_array)
        .is_some()
}

fn merged_page(payloads: &[Payload]) -> Payload {
    let mut items = Vec::new();
    for payload in payloads {
        if let Some(list) = payload.result.get("items").and_then(Value::as_array) {
            items.extend(list.iter().cloned());
        }
    }
    let mut result = Map::new();
    result.insert("items".to_owned(), Value::Array(items));
    if let Some(cursor) = payloads
        .last()
        .and_then(|payload| payload.result.get("next_cursor"))
        .filter(|cursor| !cursor.is_null())
    {
        result.insert("next_cursor".to_owned(), cursor.clone());
    }
    Payload {
        operation: payloads[0].operation.clone(),
        result: Value::Object(result),
    }
}

fn clone_payload(payload: &Payload) -> Payload {
    Payload {
        operation: payload.operation.clone(),
        result: payload.result.clone(),
    }
}

fn payload_value(payload: &Payload) -> Value {
    serde_json::json!({
        "operation": payload.operation,
        "result": payload.result
    })
}

fn lab_run(result: &Value) -> (String, String) {
    let Some(object) = result.as_object() else {
        return (String::new(), String::new());
    };
    (text_of(object.get("id")), text_of(object.get("state")))
}

fn present_result(operation: &str, result: &Value) -> (Option<ImagePreview>, Value) {
    let Some(preview) = image_preview(operation, result) else {
        return (None, result.clone());
    };
    let mut shown = result.clone();
    if let Some(object) = shown.as_object_mut() {
        object.insert(
            "data".to_owned(),
            serde_json::json!({
                "encoding": "base64",
                "characters": preview.encoded_chars,
                "decoded_bytes": preview.decoded_bytes,
                "withheld": preview.withheld
            }),
        );
    }
    (Some(preview), shown)
}

fn image_preview(operation: &str, result: &Value) -> Option<ImagePreview> {
    if !matches!(operation, "get_image" | "get_current_image") {
        return None;
    }
    let object = result.as_object()?;
    let encoded = object.get("data")?.as_str()?.to_owned();
    let media_type = object
        .get("descriptor")
        .and_then(|descriptor| descriptor.get("media_type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Some(preview_for(&media_type, &encoded))
}

fn preview_for(media_type: &str, encoded: &str) -> ImagePreview {
    let decoded_bytes = decoded_len(encoded);
    let allowed = preview_allowed(media_type);
    let (preview_url, withheld) = preview_decision(media_type, encoded, decoded_bytes, allowed);
    ImagePreview {
        media_type: media_type.to_owned(),
        encoded_chars: encoded.chars().count(),
        decoded_bytes,
        preview_url,
        withheld,
    }
}

fn preview_decision(
    media_type: &str,
    encoded: &str,
    decoded_bytes: Option<usize>,
    allowed: bool,
) -> (Option<String>, String) {
    if !allowed {
        return (
            None,
            "Media type is outside the raster preview allowlist.".to_owned(),
        );
    }
    let Some(size) = decoded_bytes else {
        return (None, "Payload is not standard base64.".to_owned());
    };
    if size > BROWSER_PREVIEW_MAX_BYTES {
        return (
            None,
            format!(
                "Payload exceeds the browser preview ceiling of {BROWSER_PREVIEW_MAX_BYTES} bytes."
            ),
        );
    }
    (
        Some(format!("data:{};base64,{encoded}", media_base(media_type))),
        String::new(),
    )
}

fn preview_allowed(media_type: &str) -> bool {
    let base = media_base(media_type);
    PREVIEW_MEDIA_TYPES
        .iter()
        .any(|allowed| base.eq_ignore_ascii_case(allowed))
}

fn media_base(media_type: &str) -> &str {
    media_type.split(';').next().unwrap_or("").trim()
}

fn decoded_len(encoded: &str) -> Option<usize> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return None;
    }
    if !encoded
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'='))
    {
        return None;
    }
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    if padding > 2 {
        return None;
    }
    Some(encoded.len() / 4 * 3 - padding)
}

fn read_details(error: &Map<String, Value>) -> DetailSet {
    let mut details = DetailSet {
        reasons: Vec::new(),
        fields: Vec::new(),
        correlations: Vec::new(),
    };
    for key in ["details", "data"] {
        let Some(items) = error.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items.iter().filter(|item| item.is_object()) {
            read_detail(item, &mut details);
        }
    }
    details
}

fn read_detail(detail: &Value, into: &mut DetailSet) {
    let Some(object) = detail.as_object() else {
        return;
    };
    if let Some(reason) = string_field(object, "reason") {
        into.reasons.push(ReasonLine {
            reason,
            domain: string_field(object, "domain").unwrap_or_default(),
        });
    }
    if let Some(fields) = object.get("fieldViolations").and_then(Value::as_array) {
        for field in fields {
            push_field(into, field);
        }
    }
    if let Some(value) = string_field(object, "requestId") {
        push_correlation(into, "Request id", value);
    }
    if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
        read_metadata(metadata, into);
    }
}

fn push_field(into: &mut DetailSet, field: &Value) {
    let Some(object) = field.as_object() else {
        return;
    };
    into.fields.push(FieldLine {
        field: string_field(object, "field").unwrap_or_default(),
        description: string_field(object, "description").unwrap_or_default(),
    });
}

fn read_metadata(metadata: &Map<String, Value>, into: &mut DetailSet) {
    for (key, value) in metadata {
        let Some(text) = value.as_str() else {
            continue;
        };
        if let Some(label) = correlation_label(key) {
            push_correlation(into, label, text.to_owned());
        }
    }
}

fn correlation_label(key: &str) -> Option<&'static str> {
    match key {
        "requestId" | "request_id" => Some("Request id"),
        "correlationId" | "correlation_id" => Some("Correlation id"),
        "taskId" | "task_id" => Some("Task id"),
        _ => None,
    }
}

fn push_correlation(into: &mut DetailSet, label: &'static str, value: String) {
    if value.is_empty() || into.correlations.iter().any(|item| item.value == value) {
        return;
    }
    into.correlations.push(Correlation { label, value });
}

fn status_message(status: Option<&Value>) -> String {
    let Some(status) = status else {
        return String::new();
    };
    match status.get("message") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(object)) => object
            .get("parts")
            .and_then(Value::as_array)
            .and_then(|parts| {
                parts
                    .iter()
                    .find_map(|part| part.get("text").and_then(Value::as_str))
            })
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    }
}

fn summary_line(operation: Operation, params: &Value, report: &Report) -> String {
    let request = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_owned());
    let mut line = format!(
        "{} {} {} task {} context {}",
        operation.name(),
        truncate(&request, 120),
        report.kind.slug(),
        report.protocol_task_id,
        report.protocol_context_id
    );
    if !report.lab_run_id.is_empty() {
        line.push_str(&format!(
            " run {} {}",
            report.lab_run_id, report.lab_run_state
        ));
    }
    if !report.status.is_empty() {
        line.push_str(&format!(" status {}", report.status));
    }
    truncate(&line, 240)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit).collect()
}

fn redact_report(mut report: Report, secret: Option<&str>) -> Report {
    report.operation = codec::redact(&report.operation, secret);
    report.protocol_task_id = codec::redact(&report.protocol_task_id, secret);
    report.protocol_context_id = codec::redact(&report.protocol_context_id, secret);
    report.protocol_state = codec::redact(&report.protocol_state, secret);
    report.lab_run_id = codec::redact(&report.lab_run_id, secret);
    report.lab_run_state = codec::redact(&report.lab_run_state, secret);
    report.status = codec::redact(&report.status, secret);
    report.code = codec::redact(&report.code, secret);
    report.message = codec::redact(&report.message, secret);
    for line in &mut report.reasons {
        line.reason = codec::redact(&line.reason, secret);
        line.domain = codec::redact(&line.domain, secret);
    }
    for line in &mut report.fields {
        line.field = codec::redact(&line.field, secret);
        line.description = codec::redact(&line.description, secret);
    }
    for line in &mut report.correlations {
        line.value = codec::redact(&line.value, secret);
    }
    report.tree = tree::redact(&report.tree, secret);
    if let Some(image) = &mut report.image {
        image.media_type = codec::redact(&image.media_type, secret);
        image.withheld = codec::redact(&image.withheld, secret);
        image.preview_url = image
            .preview_url
            .as_ref()
            .map(|url| codec::redact(url, secret));
    }
    report
}

fn redact_json(value: &Value, secret: Option<&str>) -> Value {
    match value {
        Value::String(text) => Value::String(codec::redact(text, secret)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| redact_json(item, secret)).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (codec::redact(key, secret), redact_json(item, secret)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}
