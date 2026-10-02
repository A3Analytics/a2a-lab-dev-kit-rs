---
id: TASK-12
title: Serve A2A JSON-RPC and gRPC and lock compliance
status: Done
assignee:
  - '@me'
created_date: '2026-10-01 22:15'
updated_date: '2026-10-01 22:27'
labels:
  - a2a
dependencies: []
priority: high
ordinal: 12000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A2A clients need the same lab agent on HTTP+JSON, JSON-RPC, and gRPC, and the dev kit needs in-process wire tests plus a README badge for the official MUST suite.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The agent card advertises HTTP+JSON, JSONRPC, and GRPC at protocol version 1.0
- [x] #2 POST / JSON-RPC SendMessage returns a 2.0 result, and unknown methods and invalid JSON return JSON-RPC errors
- [x] #3 gRPC SendMessage returns a task and GetTask for an unknown id fails not found
- [x] #4 In-process tests cover the HTTP+JSON wire contract, SSE frames, and the five tck message-id profiles
- [x] #5 mise run tck runs the pinned MUST suite for http_json, jsonrpc, and grpc
- [x] #6 README links an Actions badge for the A2A TCK workflow
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Mount jsonrpc_router and advertise JSONRPC on the agent card.
2. Depend on a2a-grpc 0.3 and serve GrpcHandler on a second listener with a GRPC interface.
3. Add tests/a2a_compliance.rs for the HTTP+JSON wire contract, JSON-RPC, gRPC, SSE, and tck profiles.
4. Point mise run tck at http_json,jsonrpc,grpc and add the Actions badge.
5. Update the A2A docs and README, then run mise run test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Mounted JSON-RPC on POST / and gRPC on a second listener.
- Added tests/a2a_compliance.rs. mise run quality passed, including the pinned MUST TCK for http_json, jsonrpc, and grpc.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
A2aServer now serves HTTP+JSON, JSON-RPC, and gRPC from one lab handler. The agent card advertises all three. In-process wire tests cover the HTTP contract, JSON-RPC envelopes, gRPC calls, SSE, and the tck message-id profiles. mise run tck runs the pinned MUST suite for all three transports, and the README badge points at that GitHub Actions workflow.

Key files: src/a2a/server.rs, src/a2a/card.rs, tests/a2a_compliance.rs, .mise/scripts/a2a-tck.sh, .github/workflows/a2a-tck.yml.
Verified with mise run quality.
<!-- SECTION:FINAL_SUMMARY:END -->
