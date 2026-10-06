---
id: doc-1
title: A2A-LAB devkit architecture
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A-LAB devkit architecture

## Purpose

A2A-LAB devkit gives an automated lab agent one Rust API for logs, metrics, and tasks. Protocol adapters call that API. They do not own the data.

## Providers

Implement three traits:

- `LogProvider` lists sources and queries structured records.
- `MetricProvider` lists descriptors and queries finite samples.
- `TaskProvider` lists definitions, starts a run, and returns its status.

`LabService` validates page limits and UTC ranges.

It calls the matching provider.

It stores an Agent2Agent (A2A) task snapshot.

`MemoryLogs`, `MemoryMetrics`, and `MemoryTasks` are in-memory implementations for examples and tests.

## Shared values

Identifiers are non-empty ASCII tokens. Timestamps are UTC. `TimeRange` is half-open: the start is included and the end is excluded. Pages use an opaque numeric cursor and a limit from 1 to 1000. Task input is a JSON object. `A2aLabError` uses the stable codes `invalid`, `not_found`, `unavailable`, `transport`, and `protocol`.

## Adapters

`A2aServer` speaks HTTP+JSON, JSON-RPC, and gRPC.

`A2aClient` speaks HTTP+JSON.

`McpServer` registers the same operations as Model Context Protocol (MCP) tools.

It serves standard input and output, or Streamable HTTP.

The default A2A agent uses `McpLab` to call that MCP server.

`A2aServer::new` accepts any `LabApi`, including `LabService`.

`SilaServer` is the inbound [SiLA 2](<../../reference/sila-2/doc-8 - SiLA-2.md>) Feature Provider.

`Ros2Tasks` implements `TaskProvider` for [ROS 2](<../../reference/ros-2/doc-9 - ROS-2.md>) actions.

See [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>) for the wire contracts.

See [Industrial equipment connectors](<../industrial/doc-3 - Industrial-equipment-connectors.md>) for the AAS, OPC UA, and SiLA 2 connectors.
