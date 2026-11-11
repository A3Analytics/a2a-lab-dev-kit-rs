//! Application logging configuration and emission checks.

#![allow(clippy::cognitive_complexity, clippy::large_futures)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabConfig, A2aLabService, ListLogSourcesRequest, LogOutputFormat,
    LoggingConfig, MemoryLogs, MemoryMetrics, MemoryTasks, OpenTelemetryLoggingConfig, PageRequest,
    init_logging, operation_name, redact_field, redact_free_text,
    reset_logging_installed_for_tests, trace_operation,
};
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};

#[derive(Clone, Default)]
struct SharedCapture(Arc<Mutex<Vec<u8>>>);

impl Write for SharedCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn install_capture(filter: &str, json: bool) -> (SharedCapture, tracing::Dispatch) {
    let capture = SharedCapture::default();
    let writer = capture.clone();
    let filter = EnvFilter::try_new(filter).expect("filter");
    let layer = if json {
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_filter(filter)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_filter(filter)
            .boxed()
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    (capture, tracing::Dispatch::new(subscriber))
}

fn output_text(capture: &SharedCapture) -> String {
    String::from_utf8(capture.0.lock().expect("capture lock").clone()).expect("utf8")
}

#[test]
fn invalid_logging_level_fails_predictably() {
    let error =
        A2aLabConfig::parse(r#"{"schema_version":1,"logging":{"level":"verbose"}}"#).unwrap_err();
    assert_eq!(error.code(), "invalid");
    assert!(!format!("{error}").contains("verbose"));
}

#[test]
fn logging_section_defaults_to_info_text() {
    let config = A2aLabConfig::parse(r#"{"schema_version":1,"logging":{}}"#).unwrap();
    let logging = config.logging.expect("logging");
    assert_eq!(logging.level, "info");
    assert_eq!(logging.format, LogOutputFormat::Text);
}

#[test]
fn explicit_json_format_is_unchanged() {
    let config =
        A2aLabConfig::parse(r#"{"schema_version":1,"logging":{"format":"json"}}"#).unwrap();
    assert_eq!(
        config.logging.expect("logging").format,
        LogOutputFormat::Json
    );
}

#[test]
fn invalid_logging_format_fails_predictably() {
    let error =
        A2aLabConfig::parse(r#"{"schema_version":1,"logging":{"format":"yaml"}}"#).unwrap_err();
    assert_eq!(error.code(), "invalid");
    assert!(!format!("{error}").contains("yaml"));
}

#[cfg(not(feature = "otel"))]
#[test]
fn opentelemetry_without_feature_fails() {
    reset_logging_installed_for_tests();
    let config = LoggingConfig {
        level: "info".to_owned(),
        format: LogOutputFormat::Text,
        filter: None,
        opentelemetry: Some(OpenTelemetryLoggingConfig {
            endpoint: "http://127.0.0.1:4317".to_owned(),
        }),
    };
    let error = init_logging(Some(&config)).expect_err("feature");
    assert_eq!(error.code(), "invalid");
}

#[test]
fn redaction_hides_auth_material() {
    assert_eq!(redact_free_text("Bearer secret-value"), "<redacted>");
    assert_eq!(redact_field("embedding", "[0.1,0.2]"), "<redacted>");
    assert_eq!(redact_field("operation", "list_tasks"), "list_tasks");
}

#[test]
fn text_mode_is_human_readable() {
    let (capture, dispatch) = install_capture("a2a_lab_dev_kit=info", false);
    tracing::dispatcher::with_default(&dispatch, || {
        info!(target: "a2a_lab_dev_kit::lifecycle", event = "test", "ready");
    });
    let output = output_text(&capture);
    assert!(output.contains("ready"));
    assert!(!output.starts_with('{'));
}

#[test]
fn json_mode_emits_parseable_objects() {
    let (capture, dispatch) = install_capture("a2a_lab_dev_kit=info", true);
    tracing::dispatcher::with_default(&dispatch, || {
        info!(target: "a2a_lab_dev_kit::lifecycle", event = "test", "ready");
    });
    let output = output_text(&capture);
    let line = output.lines().next().expect("line");
    let _: serde_json::Value = serde_json::from_str(line).expect("json line");
}

#[test]
fn filtering_suppresses_debug() {
    let (capture, dispatch) = install_capture("a2a_lab_dev_kit=info", false);
    tracing::dispatcher::with_default(&dispatch, || {
        debug!(target: "a2a_lab_dev_kit", event = "hidden", "debug");
        info!(target: "a2a_lab_dev_kit", event = "visible", "info");
        warn!(target: "a2a_lab_dev_kit", event = "warned", "warn");
        error!(target: "a2a_lab_dev_kit", event = "failed", "error");
    });
    let output = output_text(&capture);
    assert!(!output.contains("hidden"));
    assert!(output.contains("visible"));
    assert!(output.contains("warned"));
    assert!(output.contains("failed"));
}

#[tokio::test]
async fn operation_events_include_names_and_status() {
    let (capture, dispatch) = install_capture("a2a_lab_dev_kit=info", true);
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new());
    let command = A2aLabCommand::ListLogSources(ListLogSourcesRequest {
        page: PageRequest::new(None, 5).expect("page"),
    });
    let operation = operation_name(&command);
    let _default = tracing::dispatcher::set_default(&dispatch);
    let _ = trace_operation(operation, service.execute(command)).await;
    let output = output_text(&capture);
    assert!(output.contains("list_log_sources"));
    assert!(output.contains("operation_accepted"));
    assert!(output.contains("operation_completed"));
    assert!(!output.contains("params"));
}

#[tokio::test]
async fn list_log_sources_emits_no_per_request_flood() {
    let (capture, dispatch) = install_capture("a2a_lab_dev_kit=info", true);
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let _default = tracing::dispatcher::set_default(&dispatch);
    let _ = service
        .execute(A2aLabCommand::ListLogSources(ListLogSourcesRequest {
            page: PageRequest::new(None, 5).expect("page"),
        }))
        .await;
    let lines = output_text(&capture).lines().count();
    assert!(
        lines <= 4,
        "expected bounded operation events, got {lines} lines"
    );
}

#[cfg(feature = "otel")]
#[tokio::test]
async fn opentelemetry_endpoint_installs_without_live_collector() {
    reset_logging_installed_for_tests();
    let config = LoggingConfig {
        level: "info".to_owned(),
        format: LogOutputFormat::Text,
        filter: None,
        opentelemetry: Some(OpenTelemetryLoggingConfig {
            endpoint: "http://127.0.0.1:4317".to_owned(),
        }),
    };
    let guard = init_logging(Some(&config)).expect("install");
    assert!(guard.is_some());
    drop(guard);
}

#[cfg(feature = "otel")]
#[test]
fn opentelemetry_rejects_blank_endpoint_in_config() {
    let error = A2aLabConfig::parse(
        r#"{"schema_version":1,"logging":{"opentelemetry":{"endpoint":"   "}}}"#,
    )
    .unwrap_err();
    assert_eq!(error.code(), "invalid");
}
