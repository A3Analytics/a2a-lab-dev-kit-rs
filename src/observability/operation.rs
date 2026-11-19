//! Shared operation boundary events for lab commands.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tracing::{debug, error, info, warn};

use crate::error::A2aLabError;
use crate::service::A2aLabCommand;

static CORRELATION: AtomicU64 = AtomicU64::new(1);

/// Stable snake_case operation name shared by A2A, MCP, and SiLA adapters.
#[must_use]
pub fn operation_name(command: &A2aLabCommand) -> &'static str {
    match serde_json::to_value(command).ok().and_then(|value| {
        value
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }) {
        Some(operation) => operation_name_from_tag(&operation),
        None => "unknown_operation",
    }
}

fn operation_name_from_tag(operation: &str) -> &'static str {
    match operation.chars().next() {
        Some('l') => list_operation(operation),
        Some('q') => query_operation(operation),
        Some('s') => start_or_search_operation(operation),
        Some('g') => get_operation(operation),
        Some('r') => read_operation(operation),
        Some('a') => append_operation(operation),
        Some('f') => follow_operation(operation),
        Some('d') => decide_operation(operation),
        _ => "unknown_operation",
    }
}

fn decide_operation(operation: &str) -> &'static str {
    match operation {
        "decide_image" => "decide_image",
        _ => "unknown_operation",
    }
}

fn list_operation(operation: &str) -> &'static str {
    match operation {
        "list_log_sources" => "list_log_sources",
        "list_metrics" => "list_metrics",
        "list_tasks" => "list_tasks",
        "list_image_sources" => "list_image_sources",
        "list_images" => "list_images",
        "list_video_sources" => "list_video_sources",
        _ => "unknown_operation",
    }
}

fn query_operation(operation: &str) -> &'static str {
    match operation {
        "query_logs" => "query_logs",
        "query_metric" => "query_metric",
        _ => "unknown_operation",
    }
}

fn start_or_search_operation(operation: &str) -> &'static str {
    match operation {
        "start_task" => "start_task",
        "search_images" => "search_images",
        "search_semantic_frames" => "search_semantic_frames",
        _ => "unknown_operation",
    }
}

fn get_operation(operation: &str) -> &'static str {
    match operation {
        "get_task_status" => "get_task_status",
        "get_image" => "get_image",
        "get_current_image" => "get_current_image",
        "get_video_frame_at" => "get_video_frame_at",
        "get_video_clip" => "get_video_clip",
        "get_video_timeline" => "get_video_timeline",
        "get_frame_decision_capabilities" => "get_frame_decision_capabilities",
        "get_semantic_search_capabilities" => "get_semantic_search_capabilities",
        "get_semantic_indexing_coverage" => "get_semantic_indexing_coverage",
        _ => "unknown_operation",
    }
}

fn read_operation(operation: &str) -> &'static str {
    match operation {
        "read_frame_decision_history" => "read_frame_decision_history",
        _ => "unknown_operation",
    }
}

fn append_operation(operation: &str) -> &'static str {
    match operation {
        "append_frame_decision" => "append_frame_decision",
        _ => "unknown_operation",
    }
}

fn follow_operation(operation: &str) -> &'static str {
    match operation {
        "follow_video" => "follow_video",
        _ => "unknown_operation",
    }
}

fn log_operation_accepted(operation: &'static str, correlation_id: u64) {
    info!(
        target: "a2a_lab_dev_kit::operation",
        operation,
        correlation_id,
        event = "operation_accepted",
        "operation accepted"
    );
    debug!(
        target: "a2a_lab_dev_kit::operation",
        operation,
        correlation_id,
        event = "operation_debug",
        "operation diagnostic"
    );
}

fn next_correlation_id() -> u64 {
    CORRELATION.fetch_add(1, Ordering::Relaxed)
}

/// Runs `future` and emits accept/complete application log events.
pub async fn trace_operation<T, F>(operation: &'static str, future: F) -> Result<T, A2aLabError>
where
    F: Future<Output = Result<T, A2aLabError>>,
{
    let correlation_id = next_correlation_id();
    let started = Instant::now();
    log_operation_accepted(operation, correlation_id);
    let result = future.await;
    record_operation_outcome(
        operation,
        correlation_id,
        started.elapsed().as_millis(),
        &result,
    );
    result
}

fn record_operation_outcome<T>(
    operation: &'static str,
    correlation_id: u64,
    duration_ms: u128,
    result: &Result<T, A2aLabError>,
) {
    if let Ok(()) = result.as_ref().map(|_| ()) {
        info!(
            target: "a2a_lab_dev_kit::operation",
            operation,
            correlation_id,
            duration_ms,
            status = "ok",
            event = "operation_completed",
            "operation completed"
        );
        return;
    }
    let error_code = result.as_ref().err().map_or("error", A2aLabError::code);
    record_operation_failure(operation, correlation_id, duration_ms, error_code);
}

#[allow(clippy::cognitive_complexity)]
fn record_operation_failure(
    operation: &'static str,
    correlation_id: u64,
    duration_ms: u128,
    error_code: &'static str,
) {
    if matches!(error_code, "unavailable") {
        warn!(
            target: "a2a_lab_dev_kit::operation",
            operation,
            correlation_id,
            duration_ms,
            status = "degraded",
            error_code,
            event = "operation_degraded",
            "operation rejected"
        );
        return;
    }
    error!(
        target: "a2a_lab_dev_kit::operation",
        operation,
        correlation_id,
        duration_ms,
        status = "error",
        error_code,
        event = "operation_failed",
        "operation failed"
    );
}
