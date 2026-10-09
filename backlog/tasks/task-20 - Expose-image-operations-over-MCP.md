---
id: TASK-20
title: Expose image operations over MCP
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 21:46'
labels:
  - mcp
dependencies:
  - TASK-18
references:
  - src/mcp/server.rs
  - src/mcp/client.rs
  - tests/mcp_tests.rs
priority: high
type: feature
ordinal: 20000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
MCP clients need the same provider-neutral image operations and bounded inline payloads as A2A, routed through the shared lab service. The MCP surface must remain tools-only with the existing application/json result envelope and no camera-specific behavior.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 MCP lists list_image_sources, list_images, search_images, get_image, and get_current_image in addition to the existing tools, and each tool's generated input and output schemas match the shared request and result contract
- [x] #2 McpLab invokes all five image operations over the supported MCP transports and exposes a configurable decoded-image maximum with the same 64 MiB default as the service
- [x] #3 List and search tools return metadata-only paged results, while get_image and get_current_image return byte-preserving base64 inline data through the existing application/json result envelope
- [x] #4 MCP coverage includes source-scoped listing, range and text search, specific and current retrieval, paging, malformed input, missing resources, provider unavailability, and client/server payload-limit failures
- [x] #5 A generated 3840x2160 RGBA-sized payload completes an MCP round trip under the default limit, fails under a lower configured limit, and succeeds when matching client and server limits are raised
- [x] #6 The MCP server exposes no image resources or URI-only image delivery and adds no protocol-specific capture commands
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add list_image_sources, list_images, search_images, get_image, and get_current_image on McpServer, routed through A2aLabApi, with generated schemas from the shared request and result types. Keep the surface tools-only.
2. Route those five commands through McpLab on stdio and Streamable HTTP, and apply ImageTransportConfig when decoding image JSON so a raised limit is not stuck at the 64 MiB default.
3. Cover catalog schemas, both transports, source-scoped listing, range and text search, specific and current retrieval, paging, malformed input, missing resources, unavailability, and the 3840x2160 RGBA client/server limit cases.
4. Run fmt, clippy, and the MCP, A2A image, and provider tests.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added list_image_sources, list_images, search_images, get_image, and get_current_image on McpServer. Generated schemas come from the shared request and result types. The server stays tools-only: no resources, no image content blocks, and no capture tools.
- McpLab routes all five commands on stdio and Streamable HTTP. Image JSON is decoded with the configured ImageTransportConfig, defaulting to 64 MiB, and the HTTP event budget follows that limit.
- Tests cover source-scoped listing, range and text search, specific and current retrieval, paging, malformed input, missing resources, provider unavailability, and client/server payload limits, including the 3840x2160 RGBA round trip.
- fmt and clippy passed. nextest passed mcp_tests, mcp_images, a2a_images, and provider_tests. Full mise run quality is left for TASK-22.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
MCP now exposes the five provider-neutral image tools through McpServer and McpLab. List and search return metadata pages. get_image and get_current_image return byte-preserving base64 in the existing JSON tool result. The client applies the configured decoded-image maximum, including a raised limit, instead of the fixed 64 MiB image deserializer.

Key files: src/mcp/server.rs, src/mcp/client.rs, tests/mcp_tests.rs, tests/mcp_images.rs.

Checks: mise run fmt, mise run clippy, and nextest for mcp_tests, mcp_images, a2a_images, and provider_tests.

Residual risk: with_image_transport after connect changes decode only. A limit above 64 MiB must be passed to connect_with so the HTTP event window grows with it. MCP docs still describe the previous tool list until TASK-22. Full mise run quality was not run.
<!-- SECTION:FINAL_SUMMARY:END -->
