---
id: TASK-27
title: Document A2A-LAB compliance adoption and badge semantics
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 03:28'
updated_date: '2026-10-09 04:15'
labels:
  - compliance
  - documentation-writer
dependencies:
  - TASK-26
references:
  - task-26
  - README.md
  - backlog/docs/reference/primitives/doc-17 - A2A-LAB-primitives.md
priority: medium
type: docs
ordinal: 27000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Explain how implementation authors run the A2A-LAB compliance suite locally and in GitHub Actions, provide deterministic fixtures, retain machine-readable evidence, and display a truthful workflow-status badge. Keep the A2A-LAB profile distinct from the official A2A protocol TCK and describe what the badge does and does not establish.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Public documentation gives a copyable local runner example with A2A and MCP endpoints, fixture declaration, profile, exit behavior, and JSON report location
- [x] #2 Public documentation gives a copyable consumer-owned GitHub workflow pinned to a full devkit commit SHA, uploads the report on success or failure, and shows the corresponding standard Actions badge Markdown
- [x] #3 The fixture guide explains deterministic hardware substitutes and states that physical robots, cameras, Docker simulators, model providers, OIDC, and SiLA are outside the badge profile
- [x] #4 The compliance reference lists the profile version, required operations and case categories, A2A/MCP parity rules, report fields, and policy for required-case skips
- [x] #5 README and public docs distinguish A2A-LAB compliance from A2A protocol TCK status and avoid certification language
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Create a public adoption guide with a deterministic fixture declaration, verified local command, consumer-owned full-SHA GitHub workflow, artifact retention, and native badge semantics.
2. Create a public compliance reference for profile 1.0.0 operations, case categories, A2A/MCP parity, report fields, and required-skip policy.
3. Link both pages from README and the public overview while distinguishing A2A-LAB compliance from the official A2A protocol TCK.
4. Run documentation and repository quality checks, verify links/examples against implementation, then check acceptance criteria and complete the task.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Created public doc-22 with a validated fixture shape, copyable local runner invocation, exit/report behavior, deterministic substitute guidance, a consumer-owned workflow using an explicitly illustrative full-SHA pin, always-uploaded evidence, and native GitHub Actions badge semantics.
- Created public doc-23 defining profile 1.0.0 operations, case categories, required/unavailable skip policy, A2A/MCP parity, report fields, scope, and separation from the official A2A protocol TCK.
- Added both pages to the Wiki map and linked them from README and the public overview; the unpublished action caveat and post-publication full-SHA replacement are explicit.
- Verification passed: mise run wiki-check, git diff --check, IDE lint scan, and mise run quality (including 349 SiLA communication checks).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented end-to-end A2A-LAB compliance adoption in public guide doc-22 and profile reference doc-23. The docs cover deterministic fixtures, local and GitHub Actions execution, machine-readable evidence, immutable full-SHA pinning after publication, native badge semantics, profile 1.0.0 behavior, and the boundary from the official A2A protocol TCK.

Linked the pages from README and the public overview and registered them for Wiki publishing. mise run wiki-check, git diff --check, the lint scan, and the full mise run quality gate passed.
<!-- SECTION:FINAL_SUMMARY:END -->
