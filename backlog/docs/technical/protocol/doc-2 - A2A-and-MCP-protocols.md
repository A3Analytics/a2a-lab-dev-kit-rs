---
id: doc-2
title: A2A and MCP protocols
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A and MCP protocols

## A2A

The server speaks A2A 1.0 over HTTP+JSON. It publishes an Agent Card at `/.well-known/agent-card.json` with seven skills:

- `list-log-sources`
- `query-logs`
- `list-metrics`
- `query-metric`
- `list-tasks`
- `start-task`
- `get-task-status`

Clients send a `message` to `POST /message:send` or `POST /message/send`. The command is a data part with media type `application/vnd.a2a-lab.v1+json`:

```json
{
  "operation": "query_logs",
  "params": {}
}
```

A successful response is an A2A task. Its artifact data part uses the same media type:

```json
{
  "operation": "query_logs",
  "result": {}
}
```

Task states use the protocol names `TASK_STATE_SUBMITTED`, `TASK_STATE_WORKING`, `TASK_STATE_COMPLETED`, `TASK_STATE_FAILED`, and `TASK_STATE_CANCELED`. A task started through A2A uses the run ID as the task ID, so `GET /tasks/{id}` follows that run. `GET /tasks/{id}/subscribe` emits SSE events named `task`, `statusUpdate`, and `artifactUpdate`. Query results are split into ordered artifact chunks. The last chunk sets `lastChunk` to true.

This crate implements that HTTP+JSON profile directly and does not depend on a separate A2A crate.

## MCP

`McpServer` targets MCP `2026-07-28` through `rmcp` 3.5. The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, and `get_task_status`. Input and output schemas come from the same request and result types used by A2A.

`serve_stdio` uses newline-delimited JSON-RPC on standard input and output. `serve_http` mounts Streamable HTTP at `/mcp`. With no listener, the A2A server binds `127.0.0.1:31000` and the MCP server binds `127.0.0.1:31001`. Starting a task waits until the run is completed, failed, or canceled. Set `wait` to false to return as soon as the run is accepted, then poll with `get_task_status`.

Provider failures become MCP invalid-params or internal errors. The same failures become HTTP 400, 404, 502, or 503 on the A2A adapter, with a body of `{ "code", "message" }`.

The architecture is described in [Lab SDK architecture](<../architecture/doc-1 - Lab-SDK-architecture.md>).
