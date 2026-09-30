---
id: TASK-11
title: Add wait option to start_task
status: Done
assignee:
  - '@me'
created_date: '2026-09-30 21:58'
updated_date: '2026-09-30 22:02'
labels:
  - api
dependencies: []
priority: high
ordinal: 11000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Callers need to choose whether start_task returns as soon as a run is accepted or waits until that run is terminal, instead of always returning the first snapshot.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 StartTaskRequest includes wait (default false) and optional timeout_seconds
- [x] #2 wait=false returns the run as soon as TaskProvider::start accepts it
- [x] #3 wait=true returns only after the run is completed, failed, or canceled
- [x] #4 wait=true times out with SdkError unavailable when timeout_seconds elapses
- [x] #5 Omitted wait in JSON deserializes as false so existing start_task clients keep working
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add wait and timeout_seconds to StartTaskRequest with serde defaults.
2. LabService polls TaskProvider::status until terminal or timeout.
3. Tests for immediate, wait-until-complete, timeout, and JSON default.
4. Update A2A/MCP/overview docs, example CLI, and construction sites.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
StartTaskRequest.wait defaults to false; timeout_seconds optional (default 60). start_run polls TaskProvider::status until terminal. LabService, A2A, MCP, and a2a-lab-ot2 --wait share that helper.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
start_task now takes wait (default false) so callers can return as soon as a run is accepted or block until it is terminal.

Key files: src/tasks/model.rs, src/tasks/provider.rs (start_run), src/service.rs. Tests cover immediate, wait-until-complete, timeout, and omitted JSON wait. mise run quality passed.
<!-- SECTION:FINAL_SUMMARY:END -->
