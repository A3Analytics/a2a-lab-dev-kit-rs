---
id: TASK-9
title: Bridge industrial clients to lab providers
status: Done
assignee: []
created_date: '2026-09-30 16:51'
updated_date: '2026-10-06 06:40'
labels:
  - industrial
dependencies:
  - TASK-8
priority: high
ordinal: 9000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A2A and MCP agents should keep the same seven operations when the data comes from industrial equipment.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 IndustrialLabBuilder shares one catalog and live source across logs, metrics, and workflows
- [x] #2 An A2A client reads an OPC UA-bound metric and starts a SiLA-bound workflow
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Superseded for SiLA: catalog bindings no longer store an outbound SiLA endpoint. SiLA is served from LabApi.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Industrial providers resolve AAS bindings and serve the existing A2A operations for OPC UA metrics and SiLA workflows.
<!-- SECTION:FINAL_SUMMARY:END -->
