---
id: doc-2
title: A2A and MCP protocols
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A and MCP protocols

## A2A

The server speaks A2A 1.0 over HTTP+JSON using the official `a2a-lf` models and `a2a-server-lf` request handler. It publishes an Agent Card at `/.well-known/agent-card.json` with seven skills:

- `list-log-sources`
- `query-logs`
- `list-metrics`
- `query-metric`
- `list-tasks`
- `start-task`
- `get-task-status`

Protocol version `1.0` is on the `HTTP+JSON` interface. The card advertises `streaming`. It advertises `pushNotifications` and `extendedAgentCard` only when those features are configured on `A2aServer`.

Clients send a `message` to `POST /message:send` or `POST /message:stream`. The lab command is a data part with media type `application/vnd.a2a-lab.v1+json`:

```json
{
  "operation": "query_logs",
  "params": {}
}
```

A successful send response is a ProtoJSON task envelope. Its artifact data part uses the same media type:

```json
{
  "operation": "query_logs",
  "result": {}
}
```

Task states use the protocol names `TASK_STATE_SUBMITTED`, `TASK_STATE_WORKING`, `TASK_STATE_COMPLETED`, `TASK_STATE_FAILED`, `TASK_STATE_CANCELED`, and the extra official states `INPUT_REQUIRED`, `AUTH_REQUIRED`, and `REJECTED` when a custom executor produces them. A lab `start_task` run id is not the A2A task id. `GET /tasks/{id}` loads the protocol task. Lab run status stays on the `get_task_status` skill.

`POST /tasks/{id}:subscribe` and `POST /message:stream` emit live `StreamResponse` frames. Query results are split into ordered artifact chunks. The last chunk sets `lastChunk` to true. Subscribe after a terminal task returns `unsupported_operation`.

Core conformance is the official HTTP+JSON TCK mandatory suite plus the advertised-capability tests for streaming. Optional push, extended-card, and security declarations are out of that core set until they are configured. The TCK pin still expects success `Content-Type: application/json`; this crate emits `application/a2a+json` and deselects that one content-type assertion.

This crate depends on `a2a-lf` 0.4.1, `a2a-server-lf` 0.5.1, and `a2a-client-lf` 0.2.7.

## MCP

`McpServer` targets MCP `2026-07-28` through `rmcp` 3.5. The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, and `get_task_status`. Input and output schemas come from the same request and result types used by A2A.

`serve_stdio` uses newline-delimited JSON-RPC on standard input and output. `serve_http` mounts Streamable HTTP at `/mcp`. With no listener, the MCP server binds `127.0.0.1:31001` and the A2A server binds `127.0.0.1:31000` after connecting to that MCP URL through `McpLab`. Starting a task waits until the run is completed, failed, or canceled. Set `wait` to false to return as soon as the run is accepted, then poll with `get_task_status`.

Provider failures become MCP invalid-params or internal errors. The same failures become A2A `google.rpc.Status` mappings (`invalid_params`, `task_not_found`, or `internal`).

The architecture is described in [Lab SDK architecture](<../architecture/doc-1 - Lab-SDK-architecture.md>).
