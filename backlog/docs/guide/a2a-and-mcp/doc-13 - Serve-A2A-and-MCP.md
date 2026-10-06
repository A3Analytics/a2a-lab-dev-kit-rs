---
id: doc-13
title: Serve A2A and MCP
type: guide
audience: public
created_date: '2026-10-06 20:11'
---

# Serve A2A and MCP

Serve one lab on Model Context Protocol (MCP), then call it from Agent2Agent (A2A) and from MCP.

## Steps

The steps in [a2a_and_mcp.rs](<../../../../examples/a2a_and_mcp.rs>) are:

1. Insert a log source with id `app` into `MemoryLogs`.
2. Build a `LabService` from those logs, `MemoryMetrics`, and `MemoryTasks`.
3. Bind an A2A listener with `bind_local`.
4. Bind an MCP listener with `bind_local`.
5. Serve the `LabService` on the MCP listener with `McpServer::serve_http`.
6. Connect `McpLab` to that MCP server at `/mcp`.
7. Serve the `McpLab` on the A2A listener with `A2aServer::listen`.
8. Call `list_log_sources` with `A2aClient`.
9. Open a Streamable HTTP MCP client at protocol version `2026-07-28`.
10. Call the MCP tool `list_log_sources`.

`bind_local` listens on `127.0.0.1:0`.

With no listener, A2A binds `127.0.0.1:31000`.

With no listener, MCP binds `127.0.0.1:31001`.

The example aborts both servers after the two calls.

## Output

Run this command:

```bash
mise exec -- cargo run --example a2a_and_mcp
```

The example prints:

```text
a2a list_log_sources: app
mcp list_log_sources: app
```

## Related

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [A2A](<../../reference/a2a/doc-4 - A2A.md>)
- [MCP](<../../reference/mcp/doc-5 - MCP.md>)
