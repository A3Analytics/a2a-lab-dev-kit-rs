//! Subscriber installation for application logging.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::subscriber::set_global_default;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

use crate::config::{LoadedLabConfig, LogOutputFormat, LoggingConfig};
use crate::error::A2aLabError;

use super::lifecycle::{
    log_duplicate_init_attempt, log_effective_config, log_logging_ready, log_shutdown,
};

static LOGGING_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Guard that logs shutdown when dropped.
#[derive(Debug)]
pub struct LoggingGuard {
    #[cfg(feature = "otel")]
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for LoggingGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
        log_shutdown();
    }
}

/// Installs application logging from an optional config document section.
pub fn init(config: Option<&LoggingConfig>) -> Result<Option<LoggingGuard>, A2aLabError> {
    let Some(config) = config else {
        return Ok(None);
    };
    try_install(config)
}

/// Installs logging when a loaded config file includes `logging`.
pub fn init_from_loaded(loaded: &LoadedLabConfig) -> Result<Option<LoggingGuard>, A2aLabError> {
    let guard = init(loaded.config.logging.as_ref())?;
    if guard.is_some() {
        log_effective_config(loaded);
    }
    Ok(guard)
}

/// Returns whether application logging has been installed in this process.
#[must_use]
pub fn logging_installed() -> bool {
    LOGGING_INSTALLED.load(Ordering::SeqCst)
}

/// Resets the process-wide initialization guard. For tests only.
#[doc(hidden)]
pub fn reset_logging_installed_for_tests() {
    LOGGING_INSTALLED.store(false, Ordering::SeqCst);
}

fn try_install(config: &LoggingConfig) -> Result<Option<LoggingGuard>, A2aLabError> {
    if LOGGING_INSTALLED.swap(true, Ordering::SeqCst) {
        log_duplicate_init_attempt();
        return Ok(None);
    }
    let installed = match install_subscriber(config) {
        Ok(guard) => guard,
        Err(error) => {
            LOGGING_INSTALLED.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };
    log_logging_ready(config);
    Ok(Some(installed))
}

fn install_subscriber(config: &LoggingConfig) -> Result<LoggingGuard, A2aLabError> {
    #[cfg(not(feature = "otel"))]
    if config.opentelemetry.is_some() {
        return Err(A2aLabError::invalid(
            "logging.opentelemetry",
            "OpenTelemetry export requires the `otel` Cargo feature",
        ));
    }
    let filter = build_filter(config)?;
    #[cfg(feature = "otel")]
    if let Some((otel_layer, tracer_provider)) = build_otel_layer(config)? {
        let subscriber = Registry::default()
            .with(otel_layer)
            .with(stderr_layer(config.format, filter));
        set_global_default(subscriber)
            .map_err(|error| A2aLabError::invalid("logging", error.to_string()))?;
        return Ok(LoggingGuard {
            tracer_provider: Some(tracer_provider),
        });
    }
    let subscriber = Registry::default().with(stderr_layer(config.format, filter));
    set_global_default(subscriber)
        .map_err(|error| A2aLabError::invalid("logging", error.to_string()))?;
    Ok(LoggingGuard {
        #[cfg(feature = "otel")]
        tracer_provider: None,
    })
}

fn stderr_layer<S>(format: LogOutputFormat, filter: EnvFilter) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    match format {
        LogOutputFormat::Text => tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .with_target(true)
            .with_filter(filter)
            .boxed(),
        LogOutputFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_current_span(false)
            .with_span_list(false)
            .with_target(true)
            .with_filter(filter)
            .boxed(),
    }
}

fn build_filter(config: &LoggingConfig) -> Result<EnvFilter, A2aLabError> {
    let mut directives = format!("a2a_lab_dev_kit={}", config.level);
    if let Some(extra) = &config.filter {
        directives.push(',');
        directives.push_str(extra);
    }
    EnvFilter::try_new(directives)
        .map_err(|_| A2aLabError::invalid("logging.filter", "unsupported log filter"))
}

#[cfg(feature = "otel")]
type OtelLayer =
    tracing_opentelemetry::OpenTelemetryLayer<Registry, opentelemetry_sdk::trace::Tracer>;

#[cfg(feature = "otel")]
fn build_otel_layer(
    config: &LoggingConfig,
) -> Result<Option<(OtelLayer, opentelemetry_sdk::trace::SdkTracerProvider)>, A2aLabError> {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::WithExportConfig;

    let Some(otel) = &config.opentelemetry else {
        return Ok(None);
    };
    if otel.endpoint.trim().is_empty() {
        return Err(A2aLabError::invalid(
            "logging.opentelemetry.endpoint",
            "endpoint must contain non-whitespace text",
        ));
    }
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otel.endpoint.clone())
        .build()
        .map_err(|_| {
            A2aLabError::invalid(
                "logging.opentelemetry.endpoint",
                "invalid OpenTelemetry endpoint",
            )
        })?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer("a2a-lab-dev-kit");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    Ok(Some((layer, provider)))
}

#[cfg(test)]
mod init_tests {
    use super::*;
    use crate::config::LogOutputFormat;

    #[test]
    fn duplicate_init_is_idempotent() {
        reset_logging_installed_for_tests();
        let config = LoggingConfig {
            level: "info".to_owned(),
            format: LogOutputFormat::Text,
            filter: None,
            opentelemetry: None,
        };
        let first = init(Some(&config)).expect("first");
        let second = init(Some(&config)).expect("second");
        assert!(first.is_some());
        assert!(second.is_none());
        drop(first);
    }
}
