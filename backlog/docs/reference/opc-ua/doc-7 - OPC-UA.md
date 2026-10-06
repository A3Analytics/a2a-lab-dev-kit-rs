---
id: doc-7
title: OPC UA
type: reference
audience: public
created_date: "2026-09-30 17:38"
---

# OPC UA

## Role in A2A-LAB devkit

Open Platform Communications Unified Architecture (OPC UA) supplies the values for the [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>). A catalog binding names the OPC UA node. `LiveSource` reads that node.

The following diagram shows that mapping:

```mermaid
flowchart LR
  accTitle: OPC UA fulfills primitive values
  accDescr: An agent calls A2aLabService. A2aLabService calls industrial providers. Those providers read LiveSource. LiveSource returns log records, metric samples, and task runs for the bound OPC UA node.
  agent["Agent"] --> service["A2aLabService"]
  service --> providers["Industrial providers"]
  providers --> live["LiveSource"]
  live --> logs["Log records"]
  live --> metrics["Metric samples"]
  live --> tasks["Task runs"]
```

In the preceding diagram, an agent calls `A2aLabService`. `A2aLabService` calls the industrial providers. Those providers read `LiveSource`. `LiveSource` fulfills the primitive values for the bound node:

- `query_logs` returns log records.
- `query_metrics` returns metric samples.
- `start` and `status` return task runs.

`ScriptedLive` is the `LiveSource` in this crate. `OpcUaClient` reads those values from an OPC UA server. [Upcoming] `OpcUaClient::read_history` reads raw history for one node.

## What this crate implements

The `opcua` feature is on by default. It compiles `a2a_lab_dev_kit::opcua` against `async-opcua-client` 0.19.0.

`OpcUaClient::new` stores a PKI directory, a username, and a password. `read_history` takes an `Endpoint::OpcUa` value. It rejects a `security_policy` that contains `None`.

The session uses these settings:

- Automatic server trust is off (`trust_server_certs(false)`).
- Certificate verification is on (`verify_server_certs(true)`).
- Sample key pair creation is off (`create_sample_keypair(false)`).
- Authentication uses a username token.

The namespace index comes from the server namespace array for `namespace_uri`. The node id is `ns={index};{node_id}`.

The history read uses these settings:

- Raw data (`is_read_modified` is false)
- Source timestamps
- No bounds (`return_bounds` is false)
- No value cap (`num_values_per_node` is 0)

`BadHistoryOperationUnsupported` is `unavailable`. A history value without a source timestamp or a value is skipped. A value that is not a double is `invalid`. `filter_half_open` drops samples outside the lab range `[start, end)`. `namespace_index` returns the index of a URI in a namespace list, or `not_found`.

## Entry points

With the `opcua` feature:

- `opcua::OpcUaClient::read_history`
- `opcua::namespace_index`
- `opcua::filter_half_open`

These helpers stay in the `opcua` module. `Endpoint::OpcUa` is re-exported and carries these fields:

- `url`
- `security_policy`
- `security_mode`
- `identity`
- `node_id`
- `namespace_uri`
- `browse_path`

## Related

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>)
