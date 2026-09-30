# a2a-lab-sdk

Rust SDK for lab logs, metrics, and workflows. Provider traits are the source of truth. `LabService` runs seven operations, and A2A HTTP+JSON and MCP both call that service.

[Wiki Home](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/Home)

## Operations

1. `list_log_sources` lists log sources.
2. `query_logs` reads structured records for one source.
3. `list_metrics` lists metric descriptors.
4. `query_metric` reads samples for one metric.
5. `list_workflows` lists workflow definitions.
6. `start_workflow` starts a workflow with a JSON object and returns the run.
7. `get_workflow_status` reads one run.

A2A skill ids are hyphenated (`list-log-sources`). MCP tool names match the Rust names (`list_log_sources`).

Time ranges are half-open UTC intervals, `[start, end)`. `start` must be strictly before `end`. A timestamp inside the range is greater than or equal to `start` and strictly less than `end`. `UtcTimestamp` accepts RFC 3339 text with offset `Z`, `+00:00`, or `-00:00`.

## Quickstart

```bash
mise install
mise exec -- cargo run --example memory_lab
mise exec -- cargo run --example a2a_and_mcp
```

[`examples/memory_lab.rs`](examples/memory_lab.rs) lists logs, metrics, and workflows. The log query keeps records in `[2024-01-01T00:00:00Z, 2024-01-01T01:00:00Z)`:

```rust
let queried = logs
    .query(QueryLogsRequest {
        source_id,
        range: TimeRange::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
            UtcTimestamp::parse("2024-01-01T01:00:00Z")?,
        )?,
        page: PageRequest::new(None, 10)?,
    })
    .await?;
```

[`examples/a2a_and_mcp.rs`](examples/a2a_and_mcp.rs) shares one service between the two protocols:

```rust
let service = LabService::new(logs, MemoryMetrics::new(), MemoryWorkflows::new()).share();
```

`A2aServer::listen(None)` binds `127.0.0.1:31000`. `McpServer::serve_http(None)` binds `127.0.0.1:31001`. The example passes `TcpListener`s from `bind_local` (`127.0.0.1:0`) and runs both servers until each client call returns. `McpServer::serve_stdio` speaks MCP on standard input and output; keep diagnostics on stderr.

A2A publishes `/.well-known/agent-card.json` and accepts `POST /message:send`. MCP Streamable HTTP is mounted at `/mcp`.

## Standards

| Topic | Wiki |
| --- | --- |
| Lab SDK overview | [Home](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/Home) |
| A2A | [A2A](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/A2A) |
| MCP | [MCP](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/MCP) |
| Asset Administration Shell | [AAS](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/AAS) |
| OPC UA | [OPC-UA](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/OPC-UA) |
| SiLA 2 | [SiLA-2](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/SiLA-2) |
| ROS 2 | [ROS-2](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/ROS-2) |

## Features

`aas`, `opcua`, and `sila2` are default features. They compile `aas`, `opcua`, and `sila`. `ros2` is always compiled.

Live OPC UA and SiLA 2 integrations are partial. `OpcUaClient` and the SiLA helpers do not implement `LiveSource`. [`examples/industrial_scripted.rs`](examples/industrial_scripted.rs) uses `ScriptedLive`. [`examples/ros2_workflows.rs`](examples/ros2_workflows.rs) uses the in-process `MemoryRos2` graph.

## API documentation

```bash
mise exec -- cargo doc --no-deps --open
```

## Setup

```bash
mise install
```

## Development

```bash
mise run build
mise run test
mise run quality
```

`mise run quality` checks formatting, the Wiki stage, complexity, duplication, compilation, Clippy, and tests.
