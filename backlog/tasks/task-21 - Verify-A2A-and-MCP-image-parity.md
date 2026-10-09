---
id: TASK-21
title: Verify A2A and MCP image parity
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 22:03'
labels:
  - a2a
  - mcp
dependencies:
  - TASK-19
  - TASK-20
references:
  - tests/a2a_tests.rs
  - tests/mcp_tests.rs
priority: high
type: task
ordinal: 21000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A2A and MCP use different wire mechanisms but promise downstream callers the same image operation contract. A bounded conformance check must detect semantic drift without forcing their intentional naming and framing differences to match.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A shared representative matrix compares both protocols for source listing, source-scoped image listing, range-only search, text-only search, combined search, specific retrieval, and current retrieval using the same provider fixtures
- [x] #2 The matrix covers empty and final pages, half-open timestamp boundaries, unknown source and image IDs, unavailable providers, malformed requests, and decoded payloads immediately below, at, and above the configured maximum
- [x] #3 Equivalent successful calls produce the same ordered source or descriptor data and byte-identical retrieved images, including a generated 3840x2160 RGBA-sized inline payload
- [x] #4 Equivalent failures preserve the same stable SDK error category across protocols, and any unexplained difference reports the compared case plus both observed outputs
- [x] #5 Validation treats A2A hyphenated skills and artifact framing versus MCP snake_case tools and application/json envelopes as intentional differences, while confirming neither protocol falls back to URI-only delivery
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add tests/image_parity.rs that runs one matrix of source listing, source-scoped listing, range-only, text-only, and combined search, specific and current retrieval, empty and final pages, half-open ranges, unknown ids, unavailable and missing providers, malformed requests, and payloads below, at, and above a configured maximum through A2aClient and McpLab on the same MemoryImages fixtures.
2. Require equal ordered pages and byte-identical images, including a generated 3840x2160 RGBA payload, and the same A2aLabError code on failures. A mismatch panics with the case name and both outputs.
3. Assert hyphenated A2A skills and artifact data parts versus snake_case MCP tools and JSON envelopes, and require inline bytes instead of URI delivery. Use McpLab::connect_with for configured limits.
4. Run fmt, clippy, and the image parity, A2A image, and MCP image tests.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added tests/image_parity.rs. One matrix drives A2aClient and McpLab against the same MemoryImages fixtures.
- Equivalent successes must match ordered source or descriptor pages and byte-identical images, including a generated 3840x2160 RGBA payload.
- Coverage includes empty and final pages, half-open ranges, unknown source and image ids, unavailable and unconfigured providers, malformed requests, and payloads of 7, 8, and 9 bytes against a configured maximum of 8.
- Equivalent failures must share the stable SDK error category. A mismatch panics with the case name and both rendered outputs.
- A2A hyphenated skills and artifact data parts versus MCP snake_case tools and JSON envelopes are intentional. Both return inline base64, and neither falls back to URI delivery.
- Every MCP client uses McpLab::connect_with. Transport behavior was not changed. Limits above 64 MiB were not sent.
- mise run fmt, mise run clippy, and nextest image_parity, a2a_images, and mcp_images passed. Full mise run quality is left for TASK-22.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
A2A and MCP image operations now share one parity matrix. Equivalent successes return the same ordered pages and byte-identical images, including a 3840x2160 RGBA payload, and equivalent failures keep the same SDK error category.

Key file: tests/image_parity.rs. Checks: mise run fmt, mise run clippy, and nextest for image_parity, a2a_images, and mcp_images.

Residual risk: category comparison ignores message text, malformed A2A calls are classified from the REST problem body, and limits above 64 MiB were not exercised. connect_with is used so a larger HTTP window can be supplied without a transport change. Full mise run quality remains for TASK-22.
<!-- SECTION:FINAL_SUMMARY:END -->
