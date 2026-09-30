---
id: TASK-7
title: Read AAS repositories into the catalog
status: Done
assignee: []
created_date: '2026-09-30 16:51'
updated_date: '2026-09-30 16:51'
labels:
  - industrial
dependencies:
  - TASK-6
priority: high
ordinal: 7000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The Asset Administration Shell is the authoritative list of assets and protocol bindings.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 An AAS 3.2 repository profile loads shell bindings into the catalog
- [x] #2 Missing 3.2 profiles and rejected tokens return protocol errors
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added an IDTA AAS 3.2 HTTP client that maps binding submodels and rejects bad profiles and tokens.
<!-- SECTION:FINAL_SUMMARY:END -->
