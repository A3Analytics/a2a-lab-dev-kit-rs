---
id: TASK-3
title: Expose lab operations over A2A HTTP+JSON
status: Done
assignee:
  - '@me'
created_date: '2026-09-29 23:41'
updated_date: '2026-09-29 23:41'
labels:
  - a2a
dependencies:
  - TASK-2
priority: high
ordinal: 3000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Agents need to discover the lab skills and invoke them with A2A messages, tasks, artifacts, and streaming.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The agent card advertises the seven lab skills and HTTP+JSON
- [x] #2 A typed client can run every operation, follow workflow task state, and read ordered artifact chunks
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Publish an Agent Card and HTTP+JSON message, task, and SSE routes.
2. Add a typed client and loopback tests.
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added an A2A HTTP+JSON server and client for all seven operations. Loopback tests cover the agent card, workflow state changes, artifact chunk order, and invalid, missing, and unavailable requests.
<!-- SECTION:FINAL_SUMMARY:END -->
