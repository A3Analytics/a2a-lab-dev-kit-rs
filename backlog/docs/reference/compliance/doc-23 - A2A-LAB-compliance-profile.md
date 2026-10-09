---
id: doc-23
title: A2A-LAB compliance profile
type: reference
audience: public
created_date: '2026-10-09 04:10'
---

# A2A-LAB compliance profile

The A2A-LAB compliance profile is a versioned behavioral contract for lab operations exposed through Agent2Agent (A2A) and Model Context Protocol (MCP). Profile `1.1.0` defines a `basic` suite and a `full` suite for the same deterministic implementation.

This profile is independent of the official A2A protocol Technology Compatibility Kit (TCK). The official TCK tests A2A protocol behavior. The A2A-LAB profile tests the lab operations and cross-interface behavior defined by this devkit. Passing either suite does not imply passing the other.

## Suites

The suites have these scopes:

- `basic` runs the 47 operation-level cases from profile `1.0.0`.
- `full` is a strict superset of `basic`. It runs every basic case, 16 linked capability scenarios, and the A2A agent-message scenario when the LLM check is enabled.

The command and GitHub Action default to `full`. The LLM check defaults to enabled for a full run. A basic run does not execute the linked or LLM scenarios, even if the caller does not specify the LLM opt-out.

## Basic operation cases

The basic suite covers all twelve A2A-LAB operations:

1. `list_log_sources`
2. `query_logs`
3. `list_metrics`
4. `query_metric`
5. `list_tasks`
6. `start_task`
7. `get_task_status`
8. `list_image_sources`
9. `list_images`
10. `search_images`
11. `get_image`
12. `get_current_image`

Every required case exercises its operation over both A2A and MCP. The fixture declaration marks each primitive—logs, metrics, tasks, and images—as `required` with deterministic identifiers or as `unavailable`.

The operation cases use these categories:

- `success` checks a representative successful request.
- `boundary` checks pagination or a half-open time-range boundary.
- `invalid` checks rejection of an invalid request with the `invalid` error code.
- `not_found` checks a declared missing identifier with the `not_found` error code.
- `unavailable` checks an unavailable primitive with the `unavailable` error code.

Each operation has a success case, an unavailable case, and at least one boundary, invalid, or not-found case. Cases run in stable profile order and have stable identifiers such as `logs.list.success` and `images.get.missing-image`.

## Required cases and skips

For a primitive declared `required`, its success, boundary, invalid, and not-found cases are required. Its unavailable cases are skipped.

For a primitive declared `unavailable`, each operation's unavailable case is required. Its other cases are skipped. This policy verifies that an omitted capability fails consistently instead of silently removing that primitive from the report.

A skipped case records `required: false`, outcome `skip`, and a diagnostic. A report can set `compliant` to `true` only when:

- both A2A and MCP were tested;
- every profile case appears exactly once;
- at least one case is required; and
- every required case has outcome `pass`.

A required case must not be skipped. A skipped or failed required case makes the run noncompliant.

## Full linked scenarios

The full suite adds four linked scenarios for each of logs, metrics, tasks, and images. Each capability runs across the same bounded producer-to-consumer matrix:

- A2A to A2A
- MCP to MCP
- A2A to MCP
- MCP to A2A

These four paths across four capabilities produce 16 linked scenarios:

- Logs: discover a log source through the producer, then query that returned source identifier through the consumer.
- Metrics: discover a metric through the producer, then query that returned metric identifier through the consumer.
- Tasks: discover and start a task through the producer, verify that the returned run refers to that task, then read or poll that exact run through the consumer until it reaches a terminal state or the timeout.
- Images: discover a source and list its images through the producer, retrieve an exact returned image through the consumer, then request the current image for the discovered source.

The assertions are deliberately bounded. They require meaningful linkage, compatible identifiers, and successful availability. Task scenarios also require terminal-state progress. Image scenarios verify identifier and source relationships, but do not require two captures to have equal bytes.

The linked scenarios do not establish exhaustive operation ordering, content hashes, byte equality between different captures, unrelated boundary behavior, hardware behavior, or application-level correctness beyond the stated handoffs.

## Agent-message scenario

For a full run with the LLM check enabled, the additional `agent-message.a2a` scenario:

1. Confirms that the A2A Agent Card advertises `agent-message`.
2. Calls MCP `list_tasks` and selects a returned task identifier.
3. Sends a deterministic plain-text A2A request that names that identifier.
4. Requires a nonblank A2A context identifier and a reply that contains the exact MCP-derived task identifier.

This check uses the existing A2A agent-message endpoint and the MCP task surface. It does not define or require a plain-text MCP operation.

The check does not inspect the model's internal tool-call trace. A passing result therefore does not prove that a live model invoked a particular tool, chose a good plan, produced a generally useful answer, or behaves reliably for other prompts. It proves only the observable advertisement, MCP discovery, A2A reply grounding, and context assertions listed here.

The LLM check is enabled by default for `full` and can be explicitly omitted. Opting out removes `a2a_agent_message` from `enabled_checks` and omits `agent-message.a2a` from `scenarios`; it does not create a skipped scenario result. The `basic` suite always omits this check.

## A2A and MCP parity

The runner sends the equivalent typed A2A-LAB command to each interface. It removes protocol-specific transport envelopes, then compares the shared lab result or error code.

The shared observable fields are:

- Page operations: `items` and `next_cursor`.
- Task-run operations: `id`, `task_id`, `state`, `input`, `message`, `result`, `progress`, `error_kind`, and `error_identifier`.
- Image-returning operations: `descriptor` and `data`.

The following interface differences do not affect parity:

- A2A returns a task artifact; MCP returns structured tool content.
- A2A and MCP use their native error envelopes and status transport.
- MCP can repeat image base64 in text and structured content.
- Protocol task and context identifiers are not A2A-LAB result fields.

After those envelope differences are removed, both interfaces must return equal lab values or the same expected lab error code. A timeout, transport error, unexpected result, unexpected error code, or normalized value difference fails the case and appears in its diagnostics.

## Report fields

The runner writes one JSON object with these top-level fields:

- `suite_version`: the profile version. Profile `1.1.0` requires the exact value `"1.1.0"`.
- `selected_suite`: `basic` or `full`.
- `enabled_checks`: the checks included in the run. `basic_operations` is always present. Full runs also contain `linked_scenarios`, and default full runs contain `a2a_agent_message`.
- `implementation`: an object containing the nonblank implementation `name` and `version`.
- `tested_interfaces`: exactly `a2a` and `mcp`.
- `cases`: all profile case results in stable order.
- `scenarios`: enabled linked and agent-message scenario results in stable order.
- `compliant`: `true` only when all required enabled cases and scenarios pass.

Each entry in `cases` contains:

- `case_id`: the stable profile case identifier.
- `required`: whether the fixture declaration makes the case mandatory.
- `interfaces`: exactly `a2a` and `mcp`.
- `outcome`: `pass`, `fail`, or `skip`.
- `diagnostics`: actionable messages or normalized differences. Failed and skipped cases must include at least one diagnostic.

Each entry in `scenarios` contains:

- `scenario_id`: the stable scenario identifier.
- `required`: whether the fixture declaration makes the scenario mandatory.
- `interfaces`: the interfaces used by the scenario.
- `handoff_path`: the ordered producer and consumer interfaces.
- `outcome`: `pass`, `fail`, or `skip`.
- `diagnostics`: linkage or execution failures. Failed and skipped scenarios include at least one diagnostic.

The versioned result schema identifier is `https://a2a-lab.dev/schemas/compliance-result-1.1.0.json`. `ComplianceResult::validate` enforces the report invariants, and `compliance_result_schema` returns the JSON Schema.

## Scope

The profile covers the deterministic behavior represented by the selected suite and enabled checks. It does not test physical robots, physical cameras, Docker simulators, live model-provider quality, OpenID Connect (OIDC), Standardization in Lab Automation (SiLA), or vendor-specific integration behavior. It does not establish certification, exhaustive correctness, or official A2A TCK status.

Use the [adoption guide](<../../guide/compliance/doc-22 - Adopt-A2A-LAB-compliance.md>) to declare fixtures, run the local command, configure GitHub Actions, retain reports, and display a workflow-status badge.

## Related

- [Adopt A2A-LAB compliance](<../../guide/compliance/doc-22 - Adopt-A2A-LAB-compliance.md>)
- [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>)
- [A2A and MCP protocols](<../../technical/protocol/doc-2 - A2A-and-MCP-protocols.md>)

