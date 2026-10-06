---
id: doc-19
title: Interfaces and providers
type: reference
audience: public
created_date: "2026-10-06 22:51"
---

# Interfaces and providers

An interface exposes the A2A-LAB primitives to a client. A provider fulfills one or more of those primitives. `A2aLabService` connects the interfaces to the providers.

## How they connect

The following diagram shows that connection:

```mermaid
flowchart LR
  accTitle: Interfaces and providers
  accDescr: Interfaces call A2aLabService. A2aLabService uses built-in providers and custom providers.
  interfaces["Interfaces"] --> lab["A2aLabService"]
  lab --> builtin["Built-in providers"]
  lab --> custom["Custom providers"]
```

In the preceding diagram, interfaces call `A2aLabService`. `A2aLabService` uses built-in providers and custom providers.

## Interfaces

These interfaces are supported:

- [A2A](<../a2a/doc-4 - A2A.md>)
- [MCP](<../mcp/doc-5 - MCP.md>)
- [SiLA 2](<../sila-2/doc-8 - SiLA-2.md>)

## Providers

These providers are supported:

- [Asset Administration Shell](<../aas/doc-6 - Asset-Administration-Shell.md>)
- [OPC UA](<../opc-ua/doc-7 - OPC-UA.md>)
- [ROS 2](<../ros-2/doc-9 - ROS-2.md>)

## Custom providers

A machine without a supported standard uses a custom provider. Implement `LogProvider`, `MetricProvider`, `TaskProvider`, or a combination of those traits. Pass the implementations to `A2aLabService`. A service can mix built-in providers and custom providers.

The [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>) define that contract. [Implementations](<../implementations/doc-18 - Implementations.md>) lists the built-in providers.

## Related

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>)
- [Implementations](<../implementations/doc-18 - Implementations.md>)
