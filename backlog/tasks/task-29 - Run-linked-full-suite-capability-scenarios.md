---
id: TASK-29
title: Run linked full-suite capability scenarios
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 15:58'
updated_date: '2026-10-09 16:14'
labels:
  - compliance
  - backlog-implementer
dependencies:
  - TASK-28
references:
  - task-28
  - src/compliance_runner.rs
  - src/bin/a2a-lab-compliance.rs
priority: high
type: feature
ordinal: 29000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend the reusable runner and command to execute the profile 1.1.0 basic and full suites against deterministic A2A and MCP endpoints. Full must remain a capability-oriented superset of basic, carry values returned by earlier steps into later requests, and provide bounded cross-interface handoffs without turning the suite into exhaustive technical certification. LLM-enabled agent-message execution is reserved for a separate task.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The command accepts basic or full, defaults to full, rejects unknown suite names as configuration errors, and writes the selected suite into every completed report
- [x] #2 Selecting basic executes only the profile's existing operation-level cases, while selecting full executes every basic case plus all non-LLM linked scenarios
- [x] #3 For logs and metrics, each interface discovers an advertised identifier and successfully queries that exact identifier; the bounded handoff cases query an A2A-discovered log source through MCP and an MCP-discovered metric through A2A
- [x] #4 For tasks, each interface starts a discovered task and reads or polls the exact returned run until a terminal state or the configured timeout; the handoff case starts through A2A and reads the returned run through MCP
- [x] #5 For images, each interface discovers a source, lists or searches it, retrieves an exact returned image identifier, and requests the current image for that source; the handoff case discovers and lists through MCP and retrieves through A2A
- [x] #6 An empty discovery result, missing linked identifier, incompatible returned relationship, timeout, transport error, or unexpected lab error fails only the affected scenario with step-specific diagnostics and still permits a completed report
- [x] #7 Focused tests prove suite inclusion and defaulting, both interfaces, the bounded handoff matrix, task polling, empty-result failures, and that full assertions do not impose exhaustive ordering, hashing, or capture-byte equality
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add explicit basic/full selection to runner configuration and CLI, defaulting the command to full and rejecting unsupported suite names.
2. Execute every non-LLM full-suite scenario in profile order, carrying discovered source, metric, task/run, and image identifiers across the defined producer-consumer path with bounded task polling and step-specific failures.
3. Extend focused runner/command tests for suite inclusion, both interfaces, handoffs, polling, isolated failures, and minimal linkage assertions.
4. Run focused checks and mise run quality, verify each acceptance criterion, then record notes and completion metadata.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added typed basic/full suite selection to the reusable runner and `--suite basic|full` to the command; the command defaults to full and rejects unsupported names with configuration exit status 2.
- Full executes all 47 basic cases plus all 16 profile-defined non-LLM scenarios. Logs and metrics carry discovered identifiers into queries; tasks carry discovered task and returned run identifiers through bounded terminal polling; images carry discovered source and image identifiers through list/get/current operations.
- Executed same-interface A2A and MCP journeys plus the complete profile handoff matrix, including required A2A→MCP logs/tasks and MCP→A2A metrics/images paths. Scenario failures are isolated with operation-specific diagnostics and still produce contract-valid reports.
- Focused verification passed: mise run check, mise run clippy, mise run test, and 10 compliance runner tests covering selection/defaulting, handoffs, polling, empty discovery isolation, and non-exhaustive image assertions.
- Full repository quality gate passed: mise run quality.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented profile 1.1.0 full-suite execution in the reusable runner and command while preserving basic as operation-only scope. Full now runs all non-LLM linked scenarios across A2A, MCP, and every contract-defined handoff, carrying exact discovered identifiers and polling returned task runs with bounded, step-specific failure reporting.

Added command suite selection/defaulting and focused tests for inclusion, handoffs, polling, isolated empty discovery failures, and meaningful linkage without ordering, hashing, or capture-byte equality. `mise run quality` passes.
<!-- SECTION:FINAL_SUMMARY:END -->
