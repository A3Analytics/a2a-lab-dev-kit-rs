---
id: doc-4
title: A2A
type: reference
audience: public
created_date: '2026-09-30 17:38'
---

# A2A

## Role in this SDK

A2A is one of the two agent-facing protocols. `A2aServer` and `A2aClient` speak HTTP+JSON to `LabApi`. `LabService` runs the same seven operations it exposes over MCP.

## What this crate implements

The adapter lives in `src/a2a` and uses Axum and reqwest. `A2A_PROTOCOL_VERSION` is `"1.0"`. Cargo.toml does not depend on a separate A2A crate.

`GET /.well-known/agent-card.json` returns a card named `a2a-lab`. The card sets `protocolVersion` and the `HTTP+JSON` interface to `1.0`, `streaming` to true, and `pushNotifications` to false. Its skills are `list-log-sources`, `query-logs`, `list-metrics`, `query-metric`, `list-workflows`, `start-workflow`, and `get-workflow-status`. Input and output modes are `application/vnd.a2a-lab.v1+json` (`LAB_MEDIA_TYPE`).

`POST /message:send` and `POST /message/send` accept a message whose data part deserializes as `LabCommand`. A present `mediaType` must equal `LAB_MEDIA_TYPE`. A success body is a task: `id`, `contextId`, `status.state`, and one artifact data part of type `LabResult`. Protocol state names are `TASK_STATE_SUBMITTED`, `TASK_STATE_WORKING`, `TASK_STATE_COMPLETED`, `TASK_STATE_FAILED`, and `TASK_STATE_CANCELED`.

A started workflow stores the run id as the task id. `GET /tasks/{id}` reloads that run from the workflow provider and maps `RunState` onto `TaskState`. Other commands allocate `task-{n}` and finish in `TASK_STATE_COMPLETED`.

`GET /tasks/{id}/subscribe` emits SSE events named `task`, `statusUpdate`, and `artifactUpdate`. Query-log and query-metric pages become one artifact chunk per item. Later chunks set `append`; the last chunk sets `lastChunk`. Other results are a single chunk. `statusUpdate` sets `final` when the task state is terminal.

Failures return `{ "code", "message" }`. `invalid` and `protocol` are HTTP 400, `not_found` is 404, `unavailable` is 503, and `transport` is 502.

With no listener, `A2aServer::listen` binds `127.0.0.1:31000`.

## Entry points

Re-exported from the crate root:

- `A2aServer::new` and `A2aServer::listen`
- `A2aClient::new`, `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_workflows`, `start_workflow`, `workflow_status`, `task`, and `subscribe`
- `bind_local`, `A2A_PROTOCOL_VERSION`, `LAB_MEDIA_TYPE`, and `StreamEvent`

`A2aClient` posts to `message:send` with role `ROLE_USER`. `subscribe` reads the SSE body after the HTTP response completes.

## Related

- [Lab SDK overview](<../../overview/doc-10 - Lab-SDK-overview.md>)
- [MCP](<../mcp/doc-5 - MCP.md>)
- [Lab SDK architecture](<../../technical/architecture/doc-1 - Lab-SDK-architecture.md>)
- [A2A and MCP protocols](<../../technical/protocol/doc-2 - A2A-and-MCP-protocols.md>)
