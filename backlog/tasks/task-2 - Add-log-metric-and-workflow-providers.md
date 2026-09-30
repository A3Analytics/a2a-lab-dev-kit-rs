---
id: TASK-2
title: 'Add log, metric, and workflow providers'
status: Done
assignee:
  - '@me'
created_date: '2026-09-29 23:41'
updated_date: '2026-09-29 23:41'
labels:
  - sdk
dependencies:
  - TASK-1
priority: high
ordinal: 2000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Lab backends need async interfaces for the seven observability and workflow operations, with an in-memory implementation for tests.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Log, metric, and workflow providers list catalogs and query or start the requested resources
- [x] #2 Queries honor half-open UTC ranges and cursor pagination, and unknown IDs return not found
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add one async trait per resource family.
2. Add in-memory providers and query tests.
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added LogProvider, MetricProvider, and WorkflowProvider plus memory implementations. Tests cover pagination, half-open ranges, workflow transitions, and not-found results.
<!-- SECTION:FINAL_SUMMARY:END -->
