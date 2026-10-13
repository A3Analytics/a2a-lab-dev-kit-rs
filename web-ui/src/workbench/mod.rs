//! Primitive command workbench.
//!
//! Image previews are limited to [`PREVIEW_MEDIA_TYPES`] and
//! [`BROWSER_PREVIEW_MAX_BYTES`] decoded bytes. SVG and other media types are
//! summarized. Command history stays in page memory and is not written to storage.
//! Lab commands omit the protocol `contextId`.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub(crate) mod report;
pub(crate) mod tree;

#[cfg(test)]
mod tests;

use serde_json::{Map, Value};

use crate::inventory::Primitive;

/// Decoded-byte ceiling for a browser image preview.
pub const BROWSER_PREVIEW_MAX_BYTES: usize = 256 * 1024;

/// In-memory command history kept for the current page.
pub const HISTORY_LIMIT: usize = 20;

/// Raster media types that may be shown in an image element.
pub const PREVIEW_MEDIA_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

const PAGE_LIMIT: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    ListLogSources,
    QueryLogs,
    ListMetrics,
    QueryMetric,
    ListTasks,
    StartTask,
    GetTaskStatus,
    ListImageSources,
    ListImages,
    SearchImages,
    GetImage,
    GetCurrentImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorFault {
    Malformed,
    NotObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    pub operation: Operation,
    pub params: Value,
    pub summary: String,
}

impl Operation {
    pub const ALL: [Self; 12] = [
        Self::ListLogSources,
        Self::QueryLogs,
        Self::ListMetrics,
        Self::QueryMetric,
        Self::ListTasks,
        Self::StartTask,
        Self::GetTaskStatus,
        Self::ListImageSources,
        Self::ListImages,
        Self::SearchImages,
        Self::GetImage,
        Self::GetCurrentImage,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::ListLogSources => "list_log_sources",
            Self::QueryLogs => "query_logs",
            Self::ListMetrics => "list_metrics",
            Self::QueryMetric => "query_metric",
            Self::ListTasks => "list_tasks",
            Self::StartTask => "start_task",
            Self::GetTaskStatus => "get_task_status",
            Self::ListImageSources => "list_image_sources",
            Self::ListImages => "list_images",
            Self::SearchImages => "search_images",
            Self::GetImage => "get_image",
            Self::GetCurrentImage => "get_current_image",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ListLogSources => "List log sources",
            Self::QueryLogs => "Query logs",
            Self::ListMetrics => "List metrics",
            Self::QueryMetric => "Query metric",
            Self::ListTasks => "List tasks",
            Self::StartTask => "Start task",
            Self::GetTaskStatus => "Get task status",
            Self::ListImageSources => "List image sources",
            Self::ListImages => "List images",
            Self::SearchImages => "Search images",
            Self::GetImage => "Get image",
            Self::GetCurrentImage => "Current image",
        }
    }

    pub const fn action(self) -> &'static str {
        match self {
            Self::QueryLogs => "Query logs",
            Self::QueryMetric => "Query metric",
            Self::StartTask => "Start task",
            Self::ListImages => "List images",
            Self::SearchImages => "Search images",
            Self::GetCurrentImage => "Current image",
            _ => self.label(),
        }
    }

    pub const fn is_task(self) -> bool {
        matches!(self, Self::StartTask | Self::GetTaskStatus)
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.name() == name)
    }

    pub fn template(self) -> Value {
        match self {
            Self::ListLogSources | Self::ListMetrics | Self::ListTasks | Self::ListImageSources => {
                page_params()
            }
            Self::QueryLogs => identified("source_id", "", query_body()),
            Self::QueryMetric => identified("metric_id", "", query_body()),
            Self::StartTask => json_object(&[("task_id", ""), ("input", "")]),
            Self::GetTaskStatus | Self::GetImage => json_object(&[("id", "")]),
            Self::ListImages => identified("source_id", "", page_params()),
            Self::SearchImages => page_params(),
            Self::GetCurrentImage => json_object(&[("source_id", "")]),
        }
    }

    pub fn prefill(self, identifier: &str) -> Value {
        let mut params = self.template();
        if let Some(field) = self.id_field()
            && let Some(object) = params.as_object_mut()
        {
            object.insert(field.to_owned(), Value::String(identifier.to_owned()));
        }
        params
    }

    const fn id_field(self) -> Option<&'static str> {
        match self {
            Self::QueryLogs | Self::ListImages | Self::SearchImages | Self::GetCurrentImage => {
                Some("source_id")
            }
            Self::QueryMetric => Some("metric_id"),
            Self::StartTask => Some("task_id"),
            Self::GetTaskStatus | Self::GetImage => Some("id"),
            Self::ListLogSources | Self::ListMetrics | Self::ListTasks | Self::ListImageSources => {
                None
            }
        }
    }

    pub const fn target_field(self) -> Option<&'static str> {
        match self {
            Self::QueryLogs | Self::ListImages | Self::SearchImages | Self::GetCurrentImage => {
                Some("source_id")
            }
            Self::QueryMetric => Some("metric_id"),
            Self::StartTask => Some("task_id"),
            _ => None,
        }
    }

    pub const fn target_label(self) -> Option<&'static str> {
        match self {
            Self::QueryLogs => Some("Log source"),
            Self::QueryMetric => Some("Metric"),
            Self::StartTask => Some("Task"),
            Self::ListImages | Self::SearchImages | Self::GetCurrentImage => Some("Image source"),
            _ => None,
        }
    }
}

impl EditorFault {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Malformed => "The parameters are not valid JSON.",
            Self::NotObject => "Parameters must be a JSON object.",
        }
    }
}

pub fn openings(primitive: Primitive) -> &'static [Operation] {
    match primitive {
        Primitive::Logs => &[Operation::QueryLogs],
        Primitive::Metrics => &[Operation::QueryMetric],
        Primitive::Tasks => &[Operation::StartTask],
        Primitive::Images => &[
            Operation::ListImages,
            Operation::SearchImages,
            Operation::GetCurrentImage,
        ],
    }
}

pub fn commands(primitive: Primitive) -> &'static [Operation] {
    match primitive {
        Primitive::Logs => &[Operation::ListLogSources, Operation::QueryLogs],
        Primitive::Metrics => &[Operation::ListMetrics, Operation::QueryMetric],
        Primitive::Tasks => &[
            Operation::ListTasks,
            Operation::StartTask,
            Operation::GetTaskStatus,
        ],
        Primitive::Images => &[
            Operation::ListImageSources,
            Operation::ListImages,
            Operation::SearchImages,
            Operation::GetImage,
            Operation::GetCurrentImage,
        ],
    }
}

pub fn preview_note() -> String {
    format!(
        "Image previews are limited to {} and {} decoded bytes.",
        PREVIEW_MEDIA_TYPES.join(", "),
        BROWSER_PREVIEW_MAX_BYTES
    )
}

pub fn parse_params(text: &str) -> Result<Value, EditorFault> {
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| EditorFault::Malformed)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(EditorFault::NotObject)
    }
}

pub fn command_value(operation: Operation, params: &Value) -> Value {
    serde_json::json!({
        "operation": operation.name(),
        "params": params
    })
}

pub fn envelope(id: &str, operation: Operation, params: &Value) -> Result<String, ()> {
    crate::codec::command_message(id, &command_value(operation, params), "")
}

fn page_params() -> Value {
    serde_json::json!({ "page": { "limit": PAGE_LIMIT } })
}

fn query_body() -> Value {
    let mut params = page_params();
    params["range"] = serde_json::json!({
        "start": "2020-01-01T00:00:00Z",
        "end": "2020-01-01T01:00:00Z"
    });
    params
}

fn identified(field: &str, identifier: &str, mut params: Value) -> Value {
    if let Some(object) = params.as_object_mut() {
        object.insert(field.to_owned(), Value::String(identifier.to_owned()));
    }
    params
}

fn json_object(fields: &[(&str, &str)]) -> Value {
    let mut object = Map::new();
    for (key, value) in fields {
        if *key == "input" {
            object.insert((*key).to_owned(), Value::Object(Map::new()));
        } else {
            object.insert((*key).to_owned(), Value::String((*value).to_owned()));
        }
    }
    Value::Object(object)
}
