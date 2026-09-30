---
id: doc-5
title: MCP
type: reference
audience: public
created_date: '2026-09-30 17:38'
---

# MCP

## Role in this SDK

MCP is the other agent-facing protocol. `McpServer` registers the seven lab operations as tools on the same `LabApi` that `A2aServer` uses.

## What this crate implements

`src/mcp` builds the server with `rmcp` 3.5. `get_info` advertises protocol version `2026-07-28` (`ProtocolVersion::V_2026_07_28`), server name `a2a-lab`, instructions `Lab logs, metrics, and workflows`, and the tools capability.

The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_workflows`, `start_workflow`, and `get_workflow_status`. Each tool takes the same request type `LabService` accepts and returns the matching page or `WorkflowRun`.

`serve_stdio` speaks newline-delimited JSON-RPC on standard input and output. `serve_http` mounts Streamable HTTP at `/mcp`. With no listener it binds `127.0.0.1:31001`.

`invalid`, `protocol`, and `not_found` become MCP invalid-params errors. `unavailable` and `transport` become internal errors. `start_workflow` returns the run from `LabService` immediately. `get_workflow_status` is a separate call.

## Entry points

`McpServer` is re-exported from the crate root. Construct it with `McpServer::new` and serve with `serve_stdio` or `serve_http`.

## Related

- [Lab SDK overview](<../../overview/doc-10 - Lab-SDK-overview.md>)
- [A2A](<../a2a/doc-4 - A2A.md>)
- [Lab SDK architecture](<../../technical/architecture/doc-1 - Lab-SDK-architecture.md>)
- [A2A and MCP protocols](<../../technical/protocol/doc-2 - A2A-and-MCP-protocols.md>)
