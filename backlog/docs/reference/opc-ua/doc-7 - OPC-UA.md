---
id: doc-7
title: OPC UA
type: reference
audience: public
created_date: "2026-09-30 17:38"
---

# OPC UA

## Role in A2A-LAB devkit

Open Platform Communications Unified Architecture (OPC UA) is an outbound equipment client. Industrial providers read live samples through `LiveSource`. `OpcUaClient` implements that source and returns samples inside the half-open range. [Upcoming]

`ScriptedLive` is the `LiveSource` in this crate. `OpcUaClient::read_history` reads raw history for one node.

A catalog binding reaches the industrial providers through this path:

```mermaid
flowchart TD
  accTitle: OPC UA live readings
  accDescr: A catalog binding reaches the industrial providers. Those providers call LiveSource. OpcUaClient implements LiveSource and returns metric points. That connection is upcoming.
  binding["Catalog binding"] --> providers["Industrial providers"]
  providers --> live["LiveSource"]
  live --> client["OpcUaClient"]
  client --> points["Metric points in the half-open range"]
```

A catalog binding reaches the industrial providers. Those providers call `LiveSource`. `OpcUaClient` implements `LiveSource` and returns the metric points. [Upcoming]

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
- [Asset Administration Shell](<../aas/doc-6 - Asset-Administration-Shell.md>)
- [Standardization in Lab Automation (SiLA) 2](<../sila-2/doc-8 - SiLA-2.md>)
- [Run a scripted industrial lab](<../../guide/scripted-industrial/doc-14 - Run-a-scripted-industrial-lab.md>)
