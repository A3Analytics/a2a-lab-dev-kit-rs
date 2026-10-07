---
id: TASK-13
title: Serve lab operations as a SiLA Feature Provider
status: Done
assignee:
  - '@me'
created_date: '2026-10-06 06:40'
updated_date: '2026-10-07 00:51'
labels:
  - sila
dependencies: []
priority: high
ordinal: 13000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The devkit represents equipment, so SiLA clients need to call that equipment through one stable feature instead of the devkit dialing a remote SiLA device.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SilaServer serves SiLAService, LabOperations, and CancelController over the same LabApi as A2A and MCP.
- [x] #2 StartTask is observable and CancelCommand cancels the underlying lab run, finishing the execution with an error.
- [x] #3 The encrypted listener uses a SiLA2 certificate and announce publishes _sila._tcp.local. with protocol version 1.1.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Serve SiLAService, LabOperations, and CancelController on Arc<dyn LabApi>.
2. Make StartTask observable and map CancelCommand onto LabApi::cancel.
3. Serve TLS by default and advertise _sila._tcp.local. with protocol version 1.1.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- SilaServer routes SiLAService, LabOperations, and CancelController through LabApi.
- StartTask is observable. CancelCommand calls LabApi::cancel and the execution finishes with an error.
- TLS is the default. announce publishes _sila._tcp.local. with version 1.1 using SO_REUSEADDR so the official client can share port 5353.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
SilaServer is the inbound Feature Provider for the same LabApi used by A2A and MCP. It serves SiLAService, LabOperations, and CancelController. StartTask is observable, and CancelCommand cancels the lab run so the execution finishes with an error. TLS with a SiLA2 certificate is the default, and announce publishes _sila._tcp.local. at protocol version 1.1.

Key files: src/sila/, proto/sila/, examples/sila_interface.rs. Verified by tests/sila_tests.rs and mise run quality.
<!-- SECTION:FINAL_SUMMARY:END -->
