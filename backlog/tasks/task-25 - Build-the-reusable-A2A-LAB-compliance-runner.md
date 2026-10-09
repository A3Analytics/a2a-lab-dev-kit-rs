---
id: TASK-25
title: Build the reusable A2A-LAB compliance runner
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 03:28'
updated_date: '2026-10-09 04:02'
labels:
  - compliance
  - backlog-implementer
dependencies:
  - TASK-24
references:
  - task-24
  - src/a2a/client.rs
  - src/mcp/client.rs
priority: high
type: feature
ordinal: 25000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provide a reusable devkit runner that executes the versioned A2A-LAB contract against caller-supplied A2A and MCP endpoints and deterministic fixture declarations. It must be usable as a Rust library and as a local command, remain distinct from mise run tck and the official A2A protocol TCK, and write the contract result document even when cases fail.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A public library entry point and an a2a-lab-compliance command run the selected compliance profile against configurable A2A and MCP URLs plus a fixture declaration
- [x] #2 The runner executes the contract’s success, boundary, and failure cases for all twelve operations without requiring a robot, camera device, Docker simulator, model credentials, OIDC issuer, or SiLA server
- [x] #3 Every shared case runs against both A2A and MCP and reports an actionable difference containing the case id and normalized outputs when their required observable behavior diverges
- [x] #4 Intentional A2A/MCP envelope and error-transport differences are normalized without hiding differences in A2A-LAB result data, error code, pagination, task state, image metadata, or image bytes
- [x] #5 The command exits zero only for an overall compliant result, exits nonzero for case failures or invalid configuration, and always writes a JSON report for a completed test run
- [x] #6 Focused tests prove deterministic report ordering, required-case skip handling, timeout behavior, endpoint failures, and JSON-schema-valid compliant and noncompliant reports
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a public timeout-bounded compliance runner that builds every TASK-24 case, executes it over A2A and MCP, normalizes lab results/errors, compares shared observables, and returns a deterministically ordered validated report.
2. Add the a2a-lab-compliance binary with explicit endpoint, fixture, identity, timeout, profile, and report arguments; write reports for every completed run and map compliance/configuration to exit status.
3. Add focused runner and command tests for all operations, parity diagnostics, ordering, skips, timeouts, endpoint failures, report validity, and exit behavior.
4. Run focused checks and mise run quality, verify every acceptance criterion, then record notes and completion metadata.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added public ComplianceRunner, ComplianceEndpoint, ComplianceRunnerConfig, ComplianceRunError, and run_compliance APIs over the TASK-24 profile and report contract.
- Added a2a-lab-compliance with explicit A2A/MCP URL, fixture, profile, identity, timeout, and report arguments; completed runs always write JSON and compliance controls exit status.
- Runner executes every profile case in stable order over both interfaces, normalizes typed lab results/error codes, preserves pagination/task/image data and bytes, and reports case-id-scoped normalized differences.
- Added six focused tests covering all required case families, deterministic ordering, unavailable-capability skips, parity diagnostics, timeouts, endpoint failures, JSON round trips, and compliant/noncompliant command exit and report behavior.
- Focused compliance contract and runner suite passed: 12 tests. Full repository quality gate passed: mise run quality, including 349 pinned SiLA communication checks.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Built the reusable A2A-LAB compliance runner and a2a-lab-compliance command on the versioned TASK-24 contract. The runner covers all twelve operations over A2A and MCP, normalizes only protocol transport differences, emits deterministic actionable JSON reports, and enforces compliant-only success exits.

Added focused library/command coverage and passed the complete mise run quality gate. The public command/report surface is ready for TASK-26 GitHub Action packaging and configurable OT-2 fixture validation without embedding either downstream concern.
<!-- SECTION:FINAL_SUMMARY:END -->
