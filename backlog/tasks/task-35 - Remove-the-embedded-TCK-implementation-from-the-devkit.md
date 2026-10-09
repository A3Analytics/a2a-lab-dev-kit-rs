---
id: TASK-35
title: Remove the embedded TCK implementation from the devkit
status: To Do
assignee:
  - '@backlog-implementer'
created_date: '2026-10-09 20:12'
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
- [ ] #1 The compliance contract, runner, CLI binary, embedded action runner, TCK-only tests, schemas, and authoritative TCK docs no longer have duplicate implementations in the devkit; the legacy root action remains only as the migration bridge defined by TASK-34
- [ ] #2 The devkit continues to expose the A2A-LAB types, A2A/MCP clients, and typed malformed-image request seam required by the standalone TCK without depending on the TCK crate or repository
- [ ] #3 Public devkit docs and examples use standalone TCK terminology and coordinates while the official upstream A2A protocol TCK workflow and devkit quality coverage remain unchanged
- [ ] #4 Build metadata no longer declares the extracted compliance binary or TCK-only dependencies, and focused checks reject reintroduction of the removed implementation or a devkit-to-TCK dependency
- [ ] #5 Existing non-TCK devkit tests and mise run quality pass after cleanup
<!-- AC:END -->
