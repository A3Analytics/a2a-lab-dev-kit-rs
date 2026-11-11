//! Opt-in structured application logging for operators.
//!
//! The dev kit uses the [`tracing`] ecosystem: events are emitted with stable
//! field names, filtered by level, and rendered as human-readable text or JSON on
//! stderr. Downstream applications install a subscriber through
//! [`init`](self::init) or [`init_from_loaded`](self::init_from_loaded); the
//! library does not force a global subscriber until the application opts in.
//!
//! Async context is preserved through Tokio tasks because subscribers install on
//! the process before servers start. Structured output uses JSON field rendering
//! from `tracing-subscriber`. Human-readable output keeps the same field names in
//! key=value form.

mod init;
mod lifecycle;
mod operation;
mod redact;

pub use init::{
    LoggingGuard, init, init_from_loaded, logging_installed, reset_logging_installed_for_tests,
};
pub use lifecycle::{log_effective_config, log_server_ready};
pub use operation::{operation_name, trace_operation};
pub use redact::{is_sensitive_field, redact_field, redact_free_text};
