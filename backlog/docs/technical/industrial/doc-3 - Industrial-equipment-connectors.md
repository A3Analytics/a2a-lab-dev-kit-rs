---
id: doc-3
title: Industrial equipment connectors
type: technical
audience: technical
created_date: "2026-09-30"
---

# Industrial equipment connectors

## Roles

The seven A2A and MCP operations stay the agent-facing contract. `IndustrialLogs`, `IndustrialMetrics`, and `IndustrialTasks` implement the provider traits over one `AssetCatalogProvider` and one `LiveSource`.

`AasClient` implements `AssetCatalogProvider`. It reads `description`, `shells`, and `submodels/{id}` and rejects a description whose profiles do not include both `3.2` and `AssetAdministrationShellRepositoryServiceSpecification`.

`OpcUaClient` exposes `read_history`, `namespace_index`, and `filter_half_open`. `read_history` reads raw history for one node and drops samples outside the half-open range. The session rejects a security policy containing `None`, verifies server certificates, and authenticates with a username and password. `OpcUaClient` does not implement `LiveSource`.

SiLA support is the checked-in `proto/sila/lab.proto` plus `execution_state`, `parse_discovery`, `certificate_accepted`, `chunk_binary`, and `start_task`. Those helpers are not a Feature XML importer, a discovery browser, or a TLS stack, and they do not implement `LiveSource`.

## Bindings

Agent-facing identifiers stay short lab tokens. A shell id is stored on `AssetKey`. `SemanticId` is an IRI, an IRDI, or a custom value. A `Binding` connects one lab token to one OPC UA node or one SiLA feature member.

`IndustrialLabBuilder` shares one catalog and one live source across `IndustrialLogs`, `IndustrialMetrics`, and `IndustrialTasks`. Those providers implement the traits `LabService::new` accepts. The `LiveSource` implemented in this crate is `ScriptedLive`.

## Features

The `aas`, `opcua`, and `sila2` features are part of the default build. `Ros2Tasks` is a separate in-process task provider and is not one of these clients.

See [Lab dev kit architecture](<../architecture/doc-1 - Lab-dev-kit-architecture.md>).
