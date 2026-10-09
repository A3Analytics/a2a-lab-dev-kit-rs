---
id: TASK-34
title: Provide the legacy Action migration bridge
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 20:00'
updated_date: '2026-10-09 21:05'
labels:
  - compliance
  - migration
  - backlog-implementer
dependencies:
  - TASK-33
references:
  - action.yml
  - 'https://github.com/A3Analytics/a2a-lab-dev-kit-rs/releases/tag/v0.1.0'
  - 'https://github.com/A3Analytics/a2a-lab-tck'
priority: high
type: enhancement
ordinal: 34000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Keep users of the former repository-root A2A-LAB compliance action on an explicit supported path after the canonical TCK is published. The immutable devkit v0.1.0 tag remains available unchanged; current devkit revisions must direct or forward callers to one published immutable standalone TCK revision without introducing a Rust dependency from the devkit to the TCK.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The devkit v0.1.0 tag and its documented action contract remain unchanged and migration documentation identifies v0.1.0/9d5327868d96b3e800fd89f6debf434bcc12709d as the last embedded TCK release
- [x] #2 The current repository-root legacy action emits a visible deprecation/migration notice and preserves existing inputs, outputs, default full suite, default-on LLM check, report retention, and exit behavior while invoking A3Analytics/a2a-lab-tck at one published full 40-character commit SHA
- [x] #3 Automated action tests cover compliant, noncompliant, invalid-input, basic, default full/default-on LLM, and explicit opt-out behavior through the migration bridge
- [x] #4 README and public compliance guidance link to the standalone TCK contract, runner, Action, immutable-pin instructions, and consumer-owned badge semantics and do not describe the devkit as the authoritative TCK
- [x] #5 Cargo metadata and source inspection confirm a2a-lab-dev-kit has no dependency on a2a-lab-tck
- [x] #6 The devkit quality gate passes and no source, tag, push, or release is performed without separate authorization
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace the repository-root composite implementation with a deprecated forwarding bridge pinned to standalone TCK commit 1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd while preserving the legacy action contract.
2. Adapt action contract tests to prove delegation and retained compliant, noncompliant, invalid-input, suite, default LLM, opt-out, report, output, and exit behavior.
3. Update README, public compliance docs, and consumer workflow example to make the standalone TCK authoritative, document v0.1.0/9d5327868d96b3e800fd89f6debf434bcc12709d as the last embedded release, and preserve consumer-owned badge semantics.
4. Run focused tests, inspect Cargo metadata/source for no TCK dependency, then run mise run quality and complete all acceptance criteria.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Verified focused migration bridge tests: `mise exec -- cargo test --test github_action` (12 passed).
- Verified `v0.1.0` resolves to `9d5327868d96b3e800fd89f6debf434bcc12709d` and its action contract remains unchanged.
- Verified tracked Cargo metadata and Rust source contain no `a2a-lab-tck` dependency.

- Full `mise run quality` passed (exit 0): formatting, SiLA matrix/Wiki/complexity/duplication, compile/Clippy/tests, upstream A2A TCK, and SiLA provider/conformance checks.
- IDE diagnostics and `git diff --check` are clean. Preserved the unrelated TASK-4 updated_date-only change. No commit, push, tag, or release was performed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the current repository-root compliance Action implementation with a visible deprecated forwarding bridge pinned to A3Analytics/a2a-lab-tck@1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd while retaining the legacy inputs, outputs, defaults, report, and exit contract. Updated action tests, consumer example, README, and public compliance guidance for standalone ownership, immutable pinning, v0.1.0 migration, and consumer-owned badge semantics.

Verification: focused github_action tests passed (12/12), v0.1.0 resolves to 9d5327868d96b3e800fd89f6debf434bcc12709d, Cargo/source inspection found no TCK dependency, IDE diagnostics and diff checks are clean, and `mise run quality` passed. No commit, push, tag, or release was performed; TASK-35 remains the separate embedded implementation cleanup.
<!-- SECTION:FINAL_SUMMARY:END -->
