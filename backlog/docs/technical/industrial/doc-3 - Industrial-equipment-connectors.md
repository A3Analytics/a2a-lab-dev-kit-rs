---
id: doc-3
title: Industrial equipment connectors
type: specification
audience: technical
created_date: "2026-09-30"
---

# Industrial equipment connectors

## Roles

The seven A2A and MCP operations stay the agent-facing contract. Industrial protocols are outbound clients behind the existing provider traits.

- The Asset Administration Shell HTTP repository is the asset and semantic catalog. The client targets IDTA API 3.2 and rejects repositories that do not advertise that shell-repository profile.
- OPC UA supplies current values, historical samples, events, and method calls. Sessions require a signed security policy, certificate verification, and an explicit username or certificate identity. Namespace indexes are resolved from namespace URIs.
- SiLA 2 calls are generated from pinned Feature definitions. The checked-in lab feature follows the sila_base v1.2 observable-command statuses. Discovery accepts `_sila._tcp.local.` only, and certificate checks require the common name `SiLA2` plus a matching server UUID. Binary uploads are split at 2 MiB.

## Bindings

Agent-facing identifiers stay short lab tokens. AAS IRIs and IRDIs live on `AssetKey` and `SemanticId`. A `Binding` connects one lab token to one OPC UA node or SiLA feature member.

`IndustrialLabBuilder` shares one catalog and one live source with `IndustrialLogs`, `IndustrialMetrics`, and `IndustrialWorkflows`. Those providers are passed to `LabService::new`, so both protocol servers use the same service instance.

## Features

The AAS, OPC UA, and SiLA 2 clients are part of the default build. AASX packages and OPC 30270 are not the catalog source: OPC 30270 still maps an older AAS metamodel.

See doc-1 for the provider boundary.
