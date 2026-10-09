---
id: doc-1
title: A2A-LAB devkit architecture
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A-LAB devkit architecture

## Purpose

A2A-LAB devkit gives an automated lab agent one Rust API for logs, metrics, tasks, and images. Protocol adapters call that API. They do not own the data.

## Providers

Implement four traits:

- `LogProvider` lists sources and queries structured records.
- `MetricProvider` lists descriptors and queries finite samples.
- `TaskProvider` lists definitions, starts a run, and returns its status.
- `ImageProvider` lists image sources, lists and searches metadata, and returns inline frames.

`A2aLabService::new` still takes the log, metric, and task providers. Image operations stay unavailable until `with_images`. That keeps the three-provider constructor source-compatible.

`A2aLabService` validates page limits and UTC ranges.

It calls the matching provider.

It stores an Agent2Agent (A2A) task snapshot.

`MemoryLogs`, `MemoryMetrics`, `MemoryTasks`, and `MemoryImages` are in-memory implementations for examples and tests.

## Images

Field definitions, the five image operations, and transport limits are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Agent2Agent (A2A) and Model Context Protocol (MCP) wire details are in [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>).

This crate does not open an HTTP endpoint, a USB device, or an IP camera. It does not include a source helper, a capture helper, or a built-in source adapter. MCP exposes tools only: no image resources and no URI-only delivery. SiLA 2 `LabImages` lists sources and returns one image through binary download. `LabOperations` stays the seven log, metric, and task commands.

## Shared values

Identifiers are non-empty ASCII tokens. Timestamps are UTC. `TimeRange` is half-open: the start is included and the end is excluded. Pages use an opaque numeric cursor and a limit from 1 to 1000. Task input is a JSON object. `A2aLabError` uses the stable codes `invalid`, `not_found`, `unavailable`, `transport`, and `protocol`.

## Adapters

`A2aServer` speaks HTTP+JSON, JSON-RPC, and gRPC.

`A2aClient` speaks HTTP+JSON.

`McpServer` registers the same operations as Model Context Protocol (MCP) tools.

It serves standard input and output, or Streamable HTTP.

The default A2A agent uses `McpLab` to call that MCP server.

`A2aServer::new` accepts any `A2aLabApi`, including `A2aLabService`.

`SilaServer` is the inbound [SiLA 2](<../../reference/sila-2/doc-8 - SiLA-2.md>) Feature Provider.

`Ros2Tasks` implements `TaskProvider` for [ROS 2](<../../reference/ros-2/doc-9 - ROS-2.md>) actions.

See [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>) for the wire contracts.

See [Industrial equipment connectors](<../industrial/doc-3 - Industrial-equipment-connectors.md>) for the AAS, OPC UA, and SiLA 2 connectors.
