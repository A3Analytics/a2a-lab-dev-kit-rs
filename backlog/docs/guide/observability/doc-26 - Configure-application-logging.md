---
id: doc-26
title: Configure application logging
type: guide
audience: technical
created_date: '2026-11-11 03:44'
---

# Configure application logging

Application logging is opt in. The dev kit emits structured events with the Rust [`tracing`](https://docs.rs/tracing) crate and installs a subscriber only when the application calls `init_logging` or `init_logging_from_loaded`. Protocol adapters and `A2aLabService` share the same operation and lifecycle events.

## Why tracing

The crate is Tokio-based and had no application logging dependency. `tracing` keeps async context, supports stable structured fields, level filtering, and the same events as human-readable text or newline-delimited JSON through `tracing-subscriber`. The library does not set a global subscriber until the host application opts in, so downstream crates can keep their own subscriber when they do not call the dev kit initializer.

## Configuration file

Add an optional `logging` object to `a2a-lab.json`. When the section is present, **human-readable text on stderr is the default**; omit `format` to use it:

```json
{
  "schema_version": 1,
  "logging": {
    "level": "info",
    "filter": "tower_http=warn"
  }
}
```

| Field | Values | Default |
| --- | --- | --- |
| `level` | `trace`, `debug`, `info`, `warn`, `error`, `off` | `info` |
| `format` | `text`, `json` | `text` when omitted |
| `filter` | Extra `EnvFilter` directives | omitted |
| `logging.opentelemetry.endpoint` | OTLP gRPC URL | omitted (explicit opt-in) |

Set `"format": "json"` only when you want newline-delimited JSON events on stderr. Invalid `level`, `format`, or `filter` values fail config parsing with stable `invalid` errors. Error text does not echo secret field values.

## Initialize from Rust

After loading configuration:

```rust
use a2a_lab_dev_kit::{init_logging_from_loaded, LoadedLabConfig};

let loaded = LoadedLabConfig::load("a2a-lab.json")?;
let _logging = init_logging_from_loaded(&loaded)?;
```

Or pass an explicit section. `LoggingConfig::default()` selects human-readable text:

```rust
use a2a_lab_dev_kit::{init_logging, LoggingConfig};

let _logging = init_logging(Some(&LoggingConfig::default()))?;
```

Structured JSON is opt in:

```rust
use a2a_lab_dev_kit::{init_logging, LoggingConfig, LogOutputFormat};

let _logging = init_logging(Some(&LoggingConfig {
    level: "info".into(),
    format: LogOutputFormat::Json,
    filter: None,
    opentelemetry: None,
}))?;
```

Keep the returned `LoggingGuard` alive for the process lifetime. Dropping it emits an orderly shutdown event. Repeated initialization is ignored and does not duplicate lifecycle logs.

## Entry points

Use the same initializer before starting any adapter:

- **A2A** — call `init_logging_from_loaded`, then `A2aServer::listen`. Readiness logs use `adapter = "a2a"`.
- **MCP** — call the initializer, then `McpServer::serve_http` or `serve_stdio`. Readiness logs use `adapter = "mcp"`.
- **SiLA 2** — call the initializer, then `SilaServer::serve`. Readiness logs use `adapter = "sila"`.
- **Direct service** — call the initializer, then use `A2aLabService::execute` or `A2aLabService::share` from tests and custom hosts.

## Event fields

Stable fields help operators correlate logs:

| Field | Meaning |
| --- | --- |
| `event` | Lifecycle or operation phase, such as `operation_accepted` or `server_ready` |
| `operation` | Snake case lab command name shared by A2A and MCP |
| `correlation_id` | Monotonic operation identifier |
| `duration_ms` | Operation wall time |
| `status` | `ok`, `error`, or `degraded` |
| `error_code` | `A2aLabError::code()` without payload text |

Lifecycle configuration logs summarize which sections are present. They never print chat models, credentials, media bytes, embeddings, or request bodies.

## OpenTelemetry export

Build with the `otel` Cargo feature and set `logging.opentelemetry.endpoint`. The dev kit wires `tracing-opentelemetry` with the OpenTelemetry SDK OTLP gRPC exporter. Correlation fields on tracing events remain on exported spans. When the feature is disabled but configuration requests export, initialization fails with `invalid logging.opentelemetry`. Misconfigured endpoints fail during setup without requiring a reachable collector for default local operation.

```bash
mise exec -- cargo test --features otel --test application_logging
```

## Diagnostics policy

Video, semantic indexing, provider, and streaming code must not log per frame, per poll, or payload dumps. Operation boundaries log accept and complete events only. Use `debug!` for short-lived diagnostics; they are suppressed when `level` is above `debug`. Never log tokens, authorization headers, raw media, semantic text, or embeddings. Use `redact_field` and `redact_free_text` when adding new structured fields.
