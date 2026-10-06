---
id: doc-5
title: MCP
type: reference
audience: public
created_date: "2026-09-30 17:38"
---

# MCP

Model Context Protocol (MCP) is an interface. `McpServer` registers the seven lab operations as tools on `A2aLabApi`.

## Role

The server name is `a2a-lab`.

The instructions are `Lab logs, metrics, and tasks`.

The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, and `get_task_status`. Those operations are on the [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>) page.

`McpLab::connect` implements `A2aLabApi` over Streamable HTTP. The [A2A](<../a2a/doc-4 - A2A.md>) page uses that client in the combined example.

## Transports

`McpServer` uses `rmcp` and protocol version `2026-07-28`.

`serve_stdio` uses standard input and output.

Those frames are newline-delimited JSON-RPC.

`serve_http` mounts `/mcp`.

With no listener, `serve_http` binds `127.0.0.1:31001`.

Both transports reach the same tools:

```mermaid
flowchart LR
  accTitle: MCP transports and A2aLabService
  accDescr: Standard input and output reach the MCP tools. Streamable HTTP reaches the same tools. The tools call A2aLabService.
  stdio["Standard input and output"] --> tools["MCP tools"]
  http["Streamable HTTP"] --> tools
  tools --> service["A2aLabService"]
```

In the preceding diagram, standard input and output reach the MCP tools.

Streamable HTTP reaches those same tools.

The MCP tools call `A2aLabService`.

## Host allowlist

`serve_http` accepts these `Host` values:

- `localhost`
- `127.0.0.1`
- `::1`
- `host.docker.internal`

## Errors

`invalid`, `protocol`, and `not_found` become MCP invalid-params errors.

`unavailable` and `transport` become internal errors.

Each error includes the `A2aLabError` code.

## Wait

`start_task` waits until the run is terminal unless `wait` is false.

`timeout_seconds` defaults to 60.

## Related

- [Interfaces and providers](<../interfaces-and-providers/doc-19 - Interfaces-and-providers.md>)
- [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>)
- [A2A](<../a2a/doc-4 - A2A.md>)
- [Serve A2A and MCP](<../../guide/a2a-and-mcp/doc-13 - Serve-A2A-and-MCP.md>)
- [a2a_and_mcp.rs](../../../../examples/a2a_and_mcp.rs)
