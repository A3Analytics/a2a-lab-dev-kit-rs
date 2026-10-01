# a2a-lab-dev-kit

Rust dev kit for lab logs, metrics, and tasks. Provider traits are the source of truth. `LabService` runs seven operations, and A2A HTTP+JSON and MCP both call that service.

[Wiki Home](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home)

## Operations

1. `list_log_sources` lists log sources.
2. `query_logs` reads structured records for one source.
3. `list_metrics` lists metric descriptors.
4. `query_metric` reads samples for one metric.
5. `list_tasks` lists task definitions.
6. `start_task` starts a task with a JSON object and waits until the run is terminal. Set `wait` to false to return as soon as the run is accepted.
7. `get_task_status` reads one run.

A2A skill ids are hyphenated (`list-log-sources`). MCP tool names match the Rust names (`list_log_sources`).

Time ranges are half-open UTC intervals, `[start, end)`. `start` must be strictly before `end`. A timestamp inside the range is greater than or equal to `start` and strictly less than `end`. `UtcTimestamp` accepts RFC 3339 text with offset `Z`, `+00:00`, or `-00:00`.

## Quickstart

```bash
mise install
mise exec -- cargo run --example memory_lab
mise exec -- cargo run --example a2a_and_mcp
```

[`examples/memory_lab.rs`](examples/memory_lab.rs) lists logs, metrics, and tasks. The log query keeps records in `[2024-01-01T00:00:00Z, 2024-01-01T01:00:00Z)`:

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

[`examples/a2a_and_mcp.rs`](examples/a2a_and_mcp.rs) serves MCP from `LabService` and points A2A at that MCP server through `McpLab`:

```rust
let service = LabService::new(logs, MemoryMetrics::new(), MemoryTasks::new()).share();
let mcp_lab = McpLab::connect(&format!("http://{mcp_address}/mcp")).await?;
```

`A2aServer::listen(None)` binds `127.0.0.1:31000`. `McpServer::serve_http(None)` binds `127.0.0.1:31001`. The default A2A agent calls `http://127.0.0.1:31001/mcp`. The example passes `TcpListener`s from `bind_local` (`127.0.0.1:0`) and runs both servers until each client call returns. `McpServer::serve_stdio` speaks MCP on standard input and output; keep diagnostics on stderr.

A2A publishes `/.well-known/agent-card.json` and accepts `POST /message:send` (A2A 1.0 HTTP+JSON). MCP Streamable HTTP is mounted at `/mcp`. Optional A2A push notifications, extended Agent Card, and security schemes are off unless configured on `A2aServer`.

## Standards

| Topic                      | Wiki                                                                |
| -------------------------- | ------------------------------------------------------------------- |
| Lab dev kit overview           | [Home](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home)     |
| A2A                        | [A2A](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/A2A)       |
| MCP                        | [MCP](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/MCP)       |
| Asset Administration Shell | [AAS](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/AAS)       |
| OPC UA                     | [OPC-UA](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/OPC-UA) |
| SiLA 2                     | [SiLA-2](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/SiLA-2) |
| ROS 2                      | [ROS-2](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/ROS-2)   |

## Features

`aas`, `opcua`, and `sila2` are default features. They compile `aas`, `opcua`, and `sila`. `ros2` is always compiled.

Live OPC UA and SiLA 2 integrations are partial. `OpcUaClient` and the SiLA helpers do not implement `LiveSource`. [`examples/industrial_scripted.rs`](examples/industrial_scripted.rs) uses `ScriptedLive`. [`examples/ros2_tasks.rs`](examples/ros2_tasks.rs) uses the in-process `MemoryRos2` graph.

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

`mise run quality` checks formatting, the Wiki stage, complexity, duplication, compilation, Clippy, tests, and the official A2A HTTP+JSON TCK.

Wiki pages are generated from public Backlog docs and published by the Wiki GitHub Action. Do not publish from a local checkout. GitHub's `GITHUB_TOKEN` cannot write Wikis, so add a `WIKI_TOKEN` repository secret with Wikis read/write. Create the first GitHub Wiki page once so `.wiki.git` exists, then re-run the Action.
