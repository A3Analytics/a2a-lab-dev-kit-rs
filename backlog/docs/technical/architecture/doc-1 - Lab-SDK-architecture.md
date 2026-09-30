---
id: doc-1
title: Lab SDK architecture
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# Lab SDK architecture

## Purpose

`a2a-lab-sdk` gives an automated lab agent one Rust API for logs, metrics, and tasks. Protocol adapters call that API. They do not own the data.

## Providers

Implement three traits:

- `LogProvider` lists sources and queries structured records.
- `MetricProvider` lists descriptors and queries finite samples.
- `TaskProvider` lists definitions, starts a run, and returns its status.

`LabService` validates page limits and UTC ranges, calls the matching provider, and stores an A2A task snapshot. `MemoryLogs`, `MemoryMetrics`, and `MemoryTasks` are in-memory implementations for examples and tests.

## Shared values

Identifiers are non-empty ASCII tokens. Timestamps are UTC. `TimeRange` is half-open: the start is included and the end is excluded. Pages use an opaque numeric cursor and a limit from 1 to 1000. Task input is a JSON object. `SdkError` uses the stable codes `invalid`, `not_found`, `unavailable`, `transport`, and `protocol`.

## Adapters

`A2aServer` and `A2aClient` speak A2A HTTP+JSON. `McpServer` registers the same operations as MCP tools and can serve stdio or Streamable HTTP. Both adapters take `Arc<dyn LabApi>`, so one provider implementation can be exposed on both protocols.

See [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>) for the wire contracts and [Industrial equipment connectors](<../industrial/doc-3 - Industrial-equipment-connectors.md>) for the AAS, OPC UA, and SiLA 2 connectors.
