---
id: doc-3
title: Industrial equipment connectors
type: technical
audience: technical
created_date: "2026-09-30"
---

# Industrial equipment connectors

## Roles

Agent2Agent (A2A) and Model Context Protocol (MCP) expose twelve operations. `IndustrialLogs`, `IndustrialMetrics`, and `IndustrialTasks` cover the seven non-image operations over one `AssetCatalogProvider` and one `LiveSource`. A separate `ImageProvider` enables the five image operations. These industrial connectors do not implement `ImageProvider`.

`AasClient` implements `AssetCatalogProvider`. It reads `description`, `shells`, and `submodels/{id}` and rejects a description whose profiles lack a string that includes both `3.2` and `AssetAdministrationShellRepositoryServiceSpecification`.

`OpcUaClient` exposes `read_history`, `namespace_index`, and `filter_half_open`. `read_history` reads raw history for one node and drops samples outside the half-open range. `read_history` rejects a security policy containing `None`, verifies server certificates, and authenticates with a username and password. Industrial providers read live OPC UA samples through `LiveSource`, and `OpcUaClient` implements that source. [Upcoming]

`sila::SilaServer` is the inbound Feature Provider. It serves `LabOperations` for the seven log, metric, and task operations, and `LabImages` for named-source images. Image bytes use SiLA binary download. `sila::SilaProvider` is the outbound Feature Consumer and does not implement `ImageProvider`. It connects to one configured remote server. Catalog bindings do not store a SiLA endpoint.

## Bindings

Agent-facing identifiers stay short lab tokens. A shell id is stored on `AssetKey`. `SemanticId` is an IRI, an IRDI, or a custom value. A `Binding` connects one lab token to one OPC UA node.

`IndustrialLabBuilder` shares one catalog and one live source across `IndustrialLogs`, `IndustrialMetrics`, and `IndustrialTasks`. Those providers implement the traits `A2aLabService::new` accepts. The `LiveSource` implemented in this crate is `ScriptedLive`.

## Features

The `aas`, `opcua`, and `sila2` features are part of the default build. `sila2` serves the lab. `Ros2Tasks` is a separate in-process task provider and is not one of these clients.

See [A2A-LAB devkit architecture](<../architecture/doc-1 - Lab-dev-kit-architecture.md>).
