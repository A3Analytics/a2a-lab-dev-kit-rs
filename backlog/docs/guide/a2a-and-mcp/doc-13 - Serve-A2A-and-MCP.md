---
id: doc-13
title: Serve A2A and MCP
type: guide
audience: public
created_date: "2026-10-06 20:11"
---

# Serve A2A and MCP

Serve one lab on Model Context Protocol (MCP), then call it from Agent2Agent (A2A) and from MCP.

## Steps

The steps in [a2a_and_mcp.rs](../../../../examples/a2a_and_mcp.rs) are:

1. Insert a log source with id `app` into `MemoryLogs`.
2. Insert an image source with id `bench` into `MemoryImages`, plus frames `earlier` and `current`. Select `current` with `set_current`.
3. Build an `A2aLabService` from those logs, `MemoryMetrics`, and `MemoryTasks`, then call `with_images`.
4. Bind an A2A listener with `bind_local`.
5. Bind an MCP listener with `bind_local`.
6. Serve the `A2aLabService` on the MCP listener with `McpServer::serve_http`.
7. Connect `McpLab` to that MCP server at `/mcp`.
8. Serve the `McpLab` on the A2A listener with `A2aServer::listen`.
9. Call `list_log_sources`, `list_image_sources`, `get_current_image`, and `get_image` with `A2aClient`.
10. Open a Streamable HTTP MCP client at protocol version `2026-07-28`.
11. Call the MCP tools `list_log_sources`, `list_image_sources`, `get_current_image`, and `get_image`.

The example uses the default 64 MiB decoded-byte limit. Its frames are a few bytes. A verified 3840x2160 payload at four bytes per pixel is about 31.6 MiB and fits that default. A large 24 megapixel PNG can exceed 64 MiB.

Optional: choose one larger maximum and pass it to `A2aLabService::with_image_transport`, `A2aClient::with_image_transport`, and `McpLab::connect_with` before `McpLab` connects. The [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>) define that limit.

`bind_local` listens on `127.0.0.1:0`.

With no listener, A2A binds `127.0.0.1:31000`.

With no listener, MCP binds `127.0.0.1:31001`.

The example aborts both servers after those calls. The retrieved images are inline base64 bytes.

## Output

Run this command:

```bash
mise exec -- cargo run --example a2a_and_mcp
```

The example prints:

```text
a2a list_log_sources: app
a2a list_image_sources: bench
a2a get_current_image: current 4
a2a get_image: earlier 3
mcp list_log_sources: app
mcp list_image_sources: bench
mcp get_current_image: current 4
mcp get_image: earlier 3
```

## Related

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [A2A](<../../reference/a2a/doc-4 - A2A.md>)
- [MCP](<../../reference/mcp/doc-5 - MCP.md>)
