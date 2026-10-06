---
id: TASK-14
title: Prove the SiLA provider with the official C# client
status: Done
assignee:
  - '@me'
created_date: '2026-10-06 06:40'
updated_date: '2026-10-06 07:16'
labels:
  - sila
dependencies:
  - TASK-13
priority: high
ordinal: 14000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Local evidence has to come from an official SiLA client driving the devkit server, not from the devkit calling an official server.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 mise run sila2-interop runs sila_csharp v.10.3.2 commit 2625cce6541c501cb951f2eea95d490a2efd12c0 dynamic client against the devkit server.
- [x] #2 The client discovers the server, reads SiLAService and LabOperations, executes the lab commands, observes StartTask, and cancels a run.
- [x] #3 The JSON and JUnit reports use role feature_provider, include image ids, and fail when a required capability fails.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Run the devkit server against sila_csharp v.10.3.2 commit 2625cce.
2. Cover discovery, SiLAService, LabOperations, observable StartTask, and cancellation.
3. Write feature_provider JSON and JUnit reports that fail when a required capability fails.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- sila2-interop runs the devkit server and sila_csharp v.10.3.2 commit 2625cce6541c501cb951f2eea95d490a2efd12c0 on one container network.
- The dynamic client discovered the server, called all seven operations, observed StartTask completion, and cancelled a run.
- target/sila2-interop-reports/report.json records role feature_provider, image ids, and 10 passes with 0 failures.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
mise run sila2-interop runs the devkit server against the official sila_csharp v.10.3.2 dynamic client at commit 2625cce6541c501cb951f2eea95d490a2efd12c0. The client discovers the server, reads SiLAService and LabOperations, executes the lab commands, observes StartTask, and cancels a run.

Reports in target/sila2-interop-reports use role feature_provider, include image ids, and fail the task when a required capability fails. The latest run passed 10 and failed 0.
<!-- SECTION:FINAL_SUMMARY:END -->
