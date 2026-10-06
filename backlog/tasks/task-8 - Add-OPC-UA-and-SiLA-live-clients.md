---
id: TASK-8
title: Add OPC UA and SiLA live clients
status: Done
assignee: []
created_date: '2026-09-30 16:51'
updated_date: '2026-10-06 06:40'
labels:
  - industrial
dependencies:
  - TASK-7
priority: high
ordinal: 8000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Live samples, events, and commands come from OPC UA and generated SiLA 2 clients rather than from the shell.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 OPC UA namespace lookup and half-open history filtering are tested
- [x] #2 SiLA discovery, certificate identity, execution status, and binary chunking are tested
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Superseded for SiLA: the devkit is now a Feature Provider. OPC UA remains an outbound client.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a secured async-opcua history client and a generated SiLA 2 client with discovery, identity, status, and binary-transfer checks.
<!-- SECTION:FINAL_SUMMARY:END -->
