---
id: TASK-31
title: Expose suite and LLM controls in the compliance action
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 15:58'
updated_date: '2026-10-09 16:27'
labels:
  - compliance
  - backlog-implementer
dependencies:
  - TASK-30
references:
  - task-30
  - action.yml
  - .github/actions/a2a-lab-compliance/run.sh
priority: high
type: feature
ordinal: 31000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Keep local and GitHub Action compliance execution equivalent by exposing the profile 1.1.0 suite and LLM-check controls through the reusable action. Preserve caller-owned evidence, immutable-SHA consumption semantics, and non-certification language; publishing the action or choosing a downstream immutable SHA is outside this task.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The action accepts validated suite and LLM-check inputs, defaults them to full and enabled, and passes them unchanged to the local compliance command
- [x] #2 Action outputs or the retained report make the selected profile, suite, LLM-check state, overall compliance, and report path recoverable on success and noncompliance
- [x] #3 Invalid suite or boolean values fail before runner execution with actionable input diagnostics, while an explicit LLM opt-out runs without model credentials
- [x] #4 The consumer workflow example selects or inherits full with default-on LLM checks, retains the report with if: always(), and continues to demonstrate a full immutable commit SHA rather than a branch or tag
- [x] #5 Focused action tests cover defaults, explicit basic selection, explicit LLM opt-out, invalid values, and retained reports for compliant and noncompliant runs
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add validated `suite` and `llm-check` composite-action inputs, defaulting to `full` and `true`, and forward them exactly to the local command semantics.
2. Emit recoverable profile, suite, LLM-check, compliance, and report-path outputs while retaining report evidence across compliant and noncompliant exits.
3. Update the immutable-SHA consumer example for full/default-on behavior and always-retained evidence without publication claims.
4. Expand focused action tests for defaults, basic, opt-out, invalid controls, and both report outcomes.
5. Run focused checks and `mise run quality`, verify all ACs, then record notes and completion metadata.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added `suite` (`full` default) and `llm-check` (`true` default) composite-action inputs with strict pre-run validation; basic/full is forwarded through `--suite`, while false maps to the local command's explicit `--no-llm-check` opt-out.
- Added profile, suite, LLM-check, compliant, and report-path outputs. Completed reports retain profile 1.1.0, effective selected suite, enabled-check evidence, and compliance for both compliant and noncompliant exits.
- Updated the consumer workflow example to select full/default-on LLM behavior explicitly, retain evidence with `if: always()`, and preserve the full 40-hex immutable action reference plus non-certification language. No publication or concrete downstream SHA was chosen.
- Expanded focused action coverage for defaults, basic selection, credential-free LLM opt-out forwarding, invalid suite/boolean rejection before execution, output provenance, and retained compliant/noncompliant reports. `mise run clippy` and 10 focused action tests pass.
- Full repository quality gate passed: `mise run quality`.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Exposed profile 1.1.0 basic/full suite selection and default-on LLM checks through the reusable GitHub Action, including strict validation, explicit opt-out forwarding, provenance outputs, and retained machine-readable evidence on compliant and noncompliant runs.

Updated the immutable-SHA consumer example without solving publication or choosing a downstream SHA, and added focused tests for all control and evidence paths. `mise run quality` passes.
<!-- SECTION:FINAL_SUMMARY:END -->
