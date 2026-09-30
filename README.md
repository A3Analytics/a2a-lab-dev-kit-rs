# a2a-lab-sdk

Rust SDK for lab logs, metrics, and workflows. Provider traits are the source of truth. The same seven operations are exposed over A2A HTTP+JSON and MCP.

## Operations

- List log sources and query structured logs for a UTC time range
- List metrics and query samples for a UTC time range
- List workflows, start a workflow with a JSON object, and read its status

Time ranges are half-open UTC intervals, `[start, end)`.

## Usage

```rust
use a2a_lab_sdk::{A2aServer, LabService, MemoryLogs, MemoryMetrics, MemoryWorkflows, McpServer};

let a2aLabService = LabService::new(
    MemoryLogs::new(),
    MemoryMetrics::new(),
    MemoryWorkflows::new(),
)
.share();

// A2A listens on 127.0.0.1:31000. MCP listens on 127.0.0.1:31001.
// Pass `Some(listener)` to either call to choose a different socket.
A2aServer::new(&a2aLabService).listen(None).await?;
McpServer::new(&a2aLabService).serve_http(None).await?;
```

`McpServer::serve_stdio` speaks MCP on standard input and output. Keep diagnostics on stderr.

The A2A server publishes `/.well-known/agent-card.json` and accepts `POST /message:send`. Workflow runs started over A2A can be followed with `GET /tasks/{id}` and `GET /tasks/{id}/subscribe`.

`Ros2Workflows` is a `WorkflowProvider`. Advertise each ROS 2 action server on a `Ros2Graph` and the provider lists it as a workflow. A JSON goal starts the action. Accepted and executing goals stay in progress; succeeded, aborted, and canceled goals become completed, failed, and canceled runs.

## Setup

Install the tools pinned in `mise.toml`:

```bash
mise install
```

## Development

```bash
mise run build
mise run test
mise run quality
```

`mise run quality` checks formatting, complexity, duplication, compilation, Clippy, and tests.
