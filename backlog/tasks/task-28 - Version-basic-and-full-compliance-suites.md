---
id: TASK-28
title: Version basic and full compliance suites
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 15:58'
updated_date: '2026-10-09 16:06'
labels:
  - compliance
  - backlog-implementer
dependencies: []
references:
  - src/compliance.rs
  - tests/compliance_contract.rs
priority: high
type: feature
ordinal: 28000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Evolve the A2A-LAB compliance contract to profile 1.1.0 so consumers can distinguish operation-level capability checks from linked multi-step capability scenarios without treating either as exhaustive certification. The public model has two suites: basic preserves the existing operation cases, while full is a strict superset that adds bounded logs, metrics, tasks, images, and optional LLM-enabled scenarios. The result contract must make the selected scope and opt-outs recoverable from evidence.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Profile 1.1.0 exposes named basic and full suites; basic contains the existing operation-level case families and stable case identifiers, and full contains every basic case plus the added scenario cases
- [x] #2 The full contract defines linked scenarios for logs (discover source then query), metrics (discover metric then query), tasks (discover task, start it, then read or poll the returned run), and images (discover source, list or search, retrieve a returned image, then request the current image for that source)
- [x] #3 Each full scenario is defined for both A2A and MCP, and a bounded cross-interface matrix passes returned identifiers between interfaces while using each interface as both a producer and a consumer across the matrix
- [x] #4 Scenario assertions require only meaningful linkage, compatible identifiers, successful availability, and task terminal-state progress; they do not require exhaustive ordering, hashes, byte equality between different captures, or unrelated boundary behavior
- [x] #5 The contract represents the existing A2A agent-message capability as the only LLM-enabled compliance surface, records whether its checks were enabled, and does not invent a plain-text MCP capability
- [x] #6 The versioned result schema records selected suite, enabled checks, scenario outcomes, interfaces or handoff path, and actionable diagnostics; a full result cannot be compliant when any enabled required basic or scenario case fails or is absent
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Version the compliance profile and result schema at 1.1.0 with explicit basic/full suite and enabled-check metadata.
2. Define stable linked scenario contracts, per-interface and bounded cross-interface handoffs, minimal assertions, and the optional A2A agent-message surface.
3. Extend contract tests for suite composition, scenario coverage/matrix semantics, LLM opt-outs, and strict full-result validation.
4. Run focused tests and mise run quality, verify each acceptance criterion, then record notes and completion metadata.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Versioned the public contract and JSON schema to 1.1.0 with explicit basic/full suites; basic preserves all 47 existing operation case IDs and remains the runner default.
- Added bounded linked scenario contracts for logs, metrics, tasks, and images across A2A→A2A, MCP→MCP, A2A→MCP, and MCP→A2A handoffs with only linkage, identifier compatibility, availability, and task progress assertions.
- Added optional A2A-only agent-message evidence and strict result fields for selected suite, enabled checks, scenario outcomes, interfaces, handoff paths, and diagnostics.
- Updated fixed profile literals in the existing CLI/action/example from 1.0.0 to 1.1.0 without adding TASK-31 suite or LLM controls.
- Focused verification passed: mise run check, mise run clippy, and 23 compliance/action tests.

- Full repository quality gate passed: mise run quality, including formatting, Wiki validation, complexity/duplication, compile, clippy, workspace tests, A2A TCK, and SiLA checks.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Versioned the A2A-LAB compliance contract and result schema to profile 1.1.0. Basic preserves the 47 operation-level case identifiers and remains the current runner default; full is a strict contract superset with bounded linked logs, metrics, tasks, images, and optional A2A-only agent-message evidence.

Results now record selected suite, enabled checks, scenario outcomes, interfaces, handoff paths, and actionable diagnostics, with validation preventing compliant full evidence when enabled required outcomes fail or are absent. Updated fixed current-profile literals in the existing CLI/action/example. Focused checks and mise run quality pass.
<!-- SECTION:FINAL_SUMMARY:END -->
