---
id: TASK-4
title: Expose lab operations over MCP
status: Done
assignee:
  - '@me'
created_date: '2026-09-29 23:41'
updated_date: '2026-10-09 20:00'
labels:
  - mcp
dependencies:
  - TASK-2
priority: high
ordinal: 4000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
MCP clients need the same seven operations over stdio and Streamable HTTP without a second business implementation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Both MCP transports list and call the seven lab tools
- [x] #2 Tool schemas come from the shared request and result types, and invalid input returns an MCP error
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Register seven MCP tools over LabApi.
2. Serve stdio and Streamable HTTP and test both.
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added McpServer with stdio and /mcp Streamable HTTP transports. Both transports list the seven tools, call them, and reject an invalid time range.
<!-- SECTION:FINAL_SUMMARY:END -->
