---
id: TASK-24
title: Define the A2A-LAB compliance contract
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 03:28'
updated_date: '2026-10-09 03:36'
labels:
  - compliance
  - backlog-implementer
dependencies: []
references:
  - src/service.rs
  - backlog/docs/reference/primitives/doc-17 - A2A-LAB-primitives.md
  - backlog/docs/technical/protocol/doc-2 - A2A-and-MCP-protocols.md
priority: high
type: feature
ordinal: 24000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the versioned, reusable A2A-LAB behavioral contract in the devkit so implementations can be evaluated independently of the official A2A transport TCK. The contract covers the twelve A2A-LAB operations over A2A and MCP, deterministic fixture declarations, required versus unavailable capabilities, and a machine-readable result format. Live robot, camera, model-provider, OIDC, and SiLA checks are outside the badge profile; implementations provide deterministic fixtures for hardware-backed behavior. Authoritative starting points are src/service.rs, backlog/docs/reference/primitives/doc-17 - A2A-LAB-primitives.md, and backlog/docs/technical/protocol/doc-2 - A2A-and-MCP-protocols.md.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A versioned compliance profile identifies every required operation and defines representative success, pagination or boundary, and invalid or not-found cases for logs, metrics, tasks, and images
- [x] #2 A public fixture/adaptor contract lets a consumer name deterministic source, metric, task, run, and image fixtures without depending on physical hardware, external model services, OIDC, SiLA, or vendor simulators
- [x] #3 The contract defines which operations must pass on both A2A and MCP, which observable fields must agree across interfaces, and which protocol-envelope differences are intentional
- [x] #4 A versioned JSON result schema records suite version, implementation identity, tested interfaces, case identifiers, pass/fail/skip outcomes, diagnostics, and overall compliance; skipped required cases cannot produce a compliant result
- [x] #5 Contract validation rejects malformed fixture declarations and malformed result documents with actionable field-level errors
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a public versioned compliance module defining the twelve-operation profile, shared-interface observables, deterministic fixture declarations, and strict validation.
2. Add versioned result/report types and JSON Schema generation with consistency validation that forbids compliant reports with skipped required cases.
3. Add focused tests for profile completeness, fixture/result validation, schema shape, and JSON round trips.
4. Run focused checks and mise run quality, then record verified criteria and completion metadata.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added public compliance profile v1.0.0 covering all twelve operations, both interfaces, normalized observable fields, intentional envelope differences, and required/unavailable case paths.
- Added strict deterministic fixture declarations for logs, metrics, tasks/runs, and images with field-level validation.
- Added versioned compliance result/schema types and semantic validation; compliant reports reject skipped or failed required cases.
- Added six focused contract tests; cargo nextest run --test compliance_contract passes.

- Full repository quality gate passed: mise run quality (including format, SiLA matrix/communication, Wiki checks, complexity/duplication, compile, clippy, workspace tests, A2A TCK, and provider checks).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Defined compliance profile 1.0.0 as a public Rust contract for all twelve A2A-LAB operations across A2A and MCP, including deterministic required/unavailable fixtures, normalized observable behavior, intentional envelope differences, and strict result semantics.

Added src/compliance.rs, exports in src/lib.rs, and tests/compliance_contract.rs. Six focused tests and the full mise run quality gate pass. No live hardware, model, OIDC, SiLA, or vendor simulator dependency was added.
<!-- SECTION:FINAL_SUMMARY:END -->
