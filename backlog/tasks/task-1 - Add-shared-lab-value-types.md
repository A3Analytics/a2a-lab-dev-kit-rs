---
id: TASK-1
title: Add shared lab value types
status: Done
assignee:
  - '@me'
created_date: '2026-09-29 23:41'
updated_date: '2026-09-29 23:41'
labels:
  - sdk
dependencies: []
priority: high
ordinal: 1000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Providers and protocol adapters need one validated model for identifiers, UTC ranges, pages, JSON input, and errors.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Invalid identifiers, non-UTC timestamps, empty ranges, page limits, and non-object JSON are rejected
- [x] #2 SdkError exposes stable codes for invalid, not found, unavailable, transport, and protocol failures
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add validated IDs, UTC ranges, pages, JSON objects, and SdkError.
2. Cover the rejection and round-trip cases with tests.
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added the shared validated types used by every provider and adapter. Foundation tests reject malformed IDs, non-UTC timestamps, empty ranges, bad pages, and non-object JSON.
<!-- SECTION:FINAL_SUMMARY:END -->
