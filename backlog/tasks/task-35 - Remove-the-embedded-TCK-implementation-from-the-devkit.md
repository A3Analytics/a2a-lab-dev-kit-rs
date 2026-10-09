---
id: TASK-35
title: Remove the embedded TCK implementation from the devkit
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 20:12'
updated_date: '2026-10-09 21:11'
labels:
  - compliance
  - cleanup
  - backlog-implementer
dependencies:
  - TASK-34
references:
  - src/compliance.rs
  - src/compliance_runner.rs
  - src/bin/a2a-lab-compliance.rs
  - tests/compliance_contract.rs
  - tests/compliance_runner.rs
priority: high
type: chore
ordinal: 35000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Restore the devkit to its protocol library role after the standalone TCK and legacy migration bridge are available. Remove duplicated TCK ownership while retaining the public A2A-LAB domain types, clients, and narrowly scoped malformed-request seam required by the standalone TCK, and avoid any circular dependency.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The compliance contract, runner, CLI binary, embedded action runner, TCK-only tests, schemas, and authoritative TCK docs no longer have duplicate implementations in the devkit; the legacy root action remains only as the migration bridge defined by TASK-34
- [x] #2 The devkit continues to expose the A2A-LAB types, A2A/MCP clients, and typed malformed-image request seam required by the standalone TCK without depending on the TCK crate or repository
- [x] #3 Public devkit docs and examples use standalone TCK terminology and coordinates while the official upstream A2A protocol TCK workflow and devkit quality coverage remain unchanged
- [x] #4 Build metadata no longer declares the extracted compliance binary or TCK-only dependencies, and focused checks reject reintroduction of the removed implementation or a devkit-to-TCK dependency
- [x] #5 Existing non-TCK devkit tests and mise run quality pass after cleanup
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Remove the embedded compliance contract, runner, CLI, local action runner, TCK-only tests, and canonical compliance docs while preserving the root forwarding Action.
2. Remove compliance exports and stale documentation navigation, retaining domain APIs, A2A/MCP clients, and TckMalformedImageRequests.
3. Add focused boundary checks for removed assets, no devkit-to-TCK dependency, standalone coordinates, and retained upstream A2A TCK quality coverage.
4. Run focused checks and mise run quality, verify each acceptance criterion, then record notes, summary, and Done status.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Removed the embedded compliance contract, reusable runner, CLI binary, local composite-action runner, contract/runner tests, and devkit-owned compliance guide/reference; no compliance schemas were tracked in the devkit.
- Retained A2A-LAB domain types and A2A/MCP clients, plus TckMalformedImageRequests and its end-to-end malformed-request test. Retained the repository-root deprecated forwarding Action pinned to A3Analytics/a2a-lab-tck@1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd.
- Added focused ownership-boundary checks covering removed assets, standalone coordinates, absence of an a2a-lab-compliance target and devkit-to-TCK dependency, and retention of official upstream A2A TCK quality coverage.
- Focused tests passed (github_action 4/4; tck_malformed_images 1/1), wiki-check, fmt-check, check, and Clippy passed; Cargo metadata boundary and unchanged upstream TCK workflow/quality checks verified.
- Full mise run quality passed: 121/121 Rust tests, upstream A2A protocol TCK (206 passed, 59 skipped), SiLA communication tests (349 passed), and all other configured gates. IDE diagnostics and git diff --check are clean. Preserved the unrelated TASK-4 updated_date-only change. No commit, push, tag, or release was performed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed duplicate A2A-LAB TCK ownership from the devkit: the embedded contract, runner, CLI, local Action runner, implementation tests, and authoritative docs are gone, with focused checks preventing their return or a devkit-to-TCK dependency. The public domain/client API, typed malformed-image seam, root forwarding Action pinned to standalone commit 1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd, and official upstream A2A protocol TCK quality coverage remain.

Verification: focused tests, docs, formatting, compile, Clippy, metadata/diff checks, IDE diagnostics, and the complete mise run quality gate passed. TASK-4 was preserved; no commit, push, tag, or release was performed.
<!-- SECTION:FINAL_SUMMARY:END -->
