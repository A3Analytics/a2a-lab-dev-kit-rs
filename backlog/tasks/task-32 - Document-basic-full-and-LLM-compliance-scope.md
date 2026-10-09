---
id: TASK-32
title: 'Document basic, full, and LLM compliance scope'
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 15:58'
updated_date: '2026-10-09 16:32'
labels:
  - compliance
  - documentation-writer
dependencies:
  - TASK-31
references:
  - task-31
  - backlog/docs/reference/compliance/doc-23 - A2A-LAB-compliance-profile.md
  - backlog/docs/guide/compliance/doc-22 - Adopt-A2A-LAB-compliance.md
priority: medium
type: docs
ordinal: 32000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Update the public compliance reference and adoption guidance for profile 1.1.0 so implementation authors can choose basic or full, understand that full includes basic, configure the default-on LLM check, and interpret linked scenario evidence without reading it as exhaustive certification.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The compliance reference states that basic is the operation-level suite, full is a strict superset with linked capability scenarios, and full is the default
- [x] #2 The reference lists the logs, metrics, tasks, images, bounded cross-interface handoffs, and A2A agent-message to MCP-tool LLM scenario, including the meaningful linkage assertions and explicit non-goals
- [x] #3 The adoption guide provides copyable local and action examples for the defaults and for opting out of LLM checks, and explains how each choice appears in the JSON report
- [x] #4 The fixture guidance explains how to use a deterministic fake model without external credentials and distinguishes that substitute from live Bedrock, OpenAI, Anthropic, or other provider validation
- [x] #5 Badge and evidence language identifies profile 1.1.0, suite, LLM-check state, implementation revision, and suite revision while avoiding certification or exhaustive-correctness claims
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Verify profile 1.1.0 suite, scenario, LLM, CLI, action, and report behavior against implementation and tests.
2. Update the canonical compliance reference and adoption guide with accurate scope, examples, evidence interpretation, and non-goals.
3. Run the repository quality gate, verify every acceptance criterion, record notes and summary, and mark Done only when complete.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Updated the public compliance reference for profile 1.1.0 with basic/full scope, the 16 linked capability scenarios, bounded handoff assertions, default-on agent-message evidence, report fields, and explicit non-goals.
- Updated the adoption guide with valid profile 1.1.0 fixtures, copyable local and immutable-SHA action examples for default and LLM-opt-out runs, deterministic fake-model guidance, evidence provenance, badge limits, and the unpublished-action caveat.
- Verified claims against src/compliance.rs, src/compliance_runner.rs, the CLI, action metadata/runner, focused tests, and the downstream OT-2 deterministic fake-model fixture.
- mise run quality passed after the final documentation edit; edited docs have no IDE linter errors and git diff --check passed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented A2A-LAB compliance profile 1.1.0 in the canonical public reference and adoption guide. Readers can now distinguish the operation-level basic suite from default full scope, interpret all 16 bounded linked scenarios and the optional agent-message evidence, configure local and action LLM opt-outs, and identify implementation and suite revisions without treating the workflow badge as certification.

Changed backlog/docs/reference/compliance/doc-23 and backlog/docs/guide/compliance/doc-22. Verified implementation, CLI, action, tests, and deterministic OT-2 fixture behavior; mise run quality passes. The action remains unpublished, so examples retain an illustrative SHA that consumers must replace with a published full commit SHA.
<!-- SECTION:FINAL_SUMMARY:END -->
