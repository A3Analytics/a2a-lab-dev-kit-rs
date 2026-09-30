//! Log sources and structured log records.

mod model;
mod provider;

pub use model::{ListLogSourcesRequest, LogLevel, LogRecord, LogSource, QueryLogsRequest};
pub use provider::LogProvider;
