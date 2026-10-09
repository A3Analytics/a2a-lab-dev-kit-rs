---
id: TASK-34
title: Provide the legacy Action migration bridge
status: To Do
assignee:
  - '@backlog-implementer'
created_date: '2026-10-09 20:00'
updated_date: '2026-10-09 20:12'
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
- [ ] #1 The devkit v0.1.0 tag and its documented action contract remain unchanged and migration documentation identifies v0.1.0/9d5327868d96b3e800fd89f6debf434bcc12709d as the last embedded TCK release
- [ ] #2 The current repository-root legacy action emits a visible deprecation/migration notice and preserves existing inputs, outputs, default full suite, default-on LLM check, report retention, and exit behavior while invoking A3Analytics/a2a-lab-tck at one published full 40-character commit SHA
- [ ] #3 Automated action tests cover compliant, noncompliant, invalid-input, basic, default full/default-on LLM, and explicit opt-out behavior through the migration bridge
- [ ] #4 README and public compliance guidance link to the standalone TCK contract, runner, Action, immutable-pin instructions, and consumer-owned badge semantics and do not describe the devkit as the authoritative TCK
- [ ] #5 Cargo metadata and source inspection confirm a2a-lab-dev-kit has no dependency on a2a-lab-tck
- [ ] #6 The devkit quality gate passes and no source, tag, push, or release is performed without separate authorization
<!-- AC:END -->
