---
id: TASK-26
title: Package the compliance runner as a GitHub Action
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 03:28'
updated_date: '2026-10-09 04:08'
labels:
  - compliance
  - backlog-implementer
dependencies:
  - TASK-25
references:
  - task-25
  - .github/workflows/a2a-tck.yml
priority: high
type: feature
ordinal: 26000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Let consumer repositories invoke the devkit-owned compliance runner from their own GitHub Actions workflow at a pinned devkit revision. The action evaluates caller-started deterministic A2A and MCP endpoints, preserves the machine-readable report as workflow evidence, and leaves the badge claim attached to the consumer repository’s workflow status rather than issuing an unverifiable static badge.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A documented GitHub Action entry point accepts A2A URL, MCP URL, fixture declaration, profile selection, report path, and timeout inputs and runs the same suite as the local command
- [x] #2 The action exposes overall compliance and report-path outputs, fails its step for a noncompliant result, and retains the JSON report for a later artifact-upload step even when compliance fails
- [x] #3 Action tests cover input validation, a compliant fixture, a failing fixture, and use from a repository checkout at a pinned full commit SHA
- [x] #4 An example consumer workflow has the stable workflow name A2A-LAB Compliance, uploads the JSON report with if: always(), and demonstrates the standard GitHub Actions badge URL tied to that workflow on the default branch
- [x] #5 The action and example do not claim certification, do not use a separately hosted badge, and state that the badge proves only that the named repository commit passed the identified suite revision and profile
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a repository-root composite action and wrapper that validate action inputs, invoke the TASK-25 command at the pinned action source revision, publish compliance/report outputs, and preserve completed-run JSON before returning the command status.
2. Add an inert consumer workflow example with the stable workflow name, full-SHA action pin, always-uploaded report, standard workflow badge, and narrow non-certification semantics.
3. Add focused action tests for validation, compliant and failing endpoints, report/output preservation, and full-SHA consumer usage.
4. Run focused checks and mise run quality, verify each acceptance criterion, then record notes, summary, and Done status.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added the repository-root composite action with documented endpoint, fixture, profile, report, timeout, and implementation inputs plus compliant/report-path outputs.
- Added a validated wrapper that invokes the same a2a-lab-compliance binary as local use, creates the report directory, publishes outputs before returning noncompliant status, and leaves completed-run JSON on disk.
- Added an inert consumer workflow example with the stable A2A-LAB Compliance name, full-SHA pins, if: always() artifact upload, default-branch workflow badge URL, and explicit non-certification/commit-suite-profile semantics.
- Added five action tests covering metadata/delegation, invalid input, compliant and failing fixtures, output/report retention, and full-commit-SHA consumer usage. Focused action plus runner checks passed: 11 tests.
- Full repository quality gate passed: mise run quality, including 349 pinned SiLA communication checks.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Packaged the TASK-25 compliance command as a repository-root composite GitHub Action. It validates consumer inputs, runs the same versioned suite, exposes compliance and report path, preserves completed-run JSON on failure, and includes a full-SHA consumer workflow example with accurate badge semantics.

Added focused action coverage and passed mise run quality. The action is ready for downstream OT-2 TASK-19 to pin the devkit revision, start deterministic endpoints, supply its fixture declaration, and always upload the returned report path.
<!-- SECTION:FINAL_SUMMARY:END -->
