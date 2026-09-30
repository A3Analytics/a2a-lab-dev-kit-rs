---
id: TASK-6
title: Add industrial asset catalog
status: Done
assignee: []
created_date: '2026-09-30 16:51'
updated_date: '2026-09-30 16:51'
labels:
  - industrial
dependencies: []
priority: high
ordinal: 6000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
AAS identifiers cannot fit in the agent-facing lab tokens, so the SDK needs a separate semantic catalog.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 IRI and IRDI identifiers validate without weakening lab tokens
- [x] #2 An in-memory catalog pages assets and bindings and reports unknown assets
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added the semantic catalog, bindings, and memory catalog without changing agent-facing identifier rules.
<!-- SECTION:FINAL_SUMMARY:END -->
