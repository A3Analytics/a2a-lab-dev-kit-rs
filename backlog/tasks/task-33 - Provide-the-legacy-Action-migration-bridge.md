---
id: TASK-33
title: Expose a typed TCK malformed-image request seam
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 20:00'
updated_date: '2026-10-09 20:19'
labels:
  - compliance
  - tck-api
  - backlog-implementer
dependencies: []
references:
  - src/images/model.rs
  - src/compliance_runner.rs
  - src/a2a/client.rs
  - src/mcp/client.rs
  - tests/a2a_images.rs
  - tests/mcp_images.rs
priority: high
type: enhancement
ordinal: 33000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Allow the standalone A2A-LAB TCK to exercise the three profile 1.1.0 malformed-image cases through the public typed devkit clients. The public seam must be intentionally limited to these conformance requests so ordinary callers cannot construct arbitrary unchecked image requests and normal validated constructors remain unchanged.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A public typed TCK-focused API constructs exactly the profile 1.1.0 malformed requests for list_image_sources with an invalid zero page limit, list_images with a caller-supplied source and invalid zero page limit, and search_images with no range or text criterion
- [x] #2 The TCK-focused API returns request or command values accepted by the existing public A2aClient and McpLab execution paths, and serialized A2A and MCP payloads preserve the intended malformed fields until the server rejects them
- [x] #3 Public ListImageSourcesRequest::new, ListImagesRequest::new, SearchImagesRequest::new, and deserialization continue rejecting malformed input; no general-purpose public unchecked constructor, arbitrary raw JSON client call, or unrelated validation bypass is exposed
- [x] #4 An external consumer-style test uses only public devkit exports to execute all three malformed-image requests over deterministic A2A and MCP endpoints and observes the shared invalid error code on both interfaces
- [x] #5 Existing image request validation tests, A2A/MCP behavior tests, official upstream A2A TCK checks, and mise run quality pass
- [x] #6 Cargo metadata has no dependency on a2a-lab-tck, and this task does not move the TCK contract or runner back into the devkit
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a public TCK-focused image-request factory whose three methods can only construct the profile 1.1.0 invalid list-source limit, list-images limit, and criterion-free search requests.
2. Reuse that seam in the embedded runner without changing validated constructors, deserialization, or client execution APIs.
3. Add an external consumer-style integration test that sends all three requests through public A2A and MCP clients and verifies the shared invalid code and serialized malformed shape.
4. Run focused tests and cargo metadata inspection, then mise run quality; record verified acceptance criteria, notes, and final summary.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added public `TckMalformedImageRequests` with only three constructors: zero-limit list_image_sources, caller-source zero-limit list_images, and criterion-free search_images.
- Removed the broader internal image request unchecked constructors and updated the embedded runner to consume the narrow seam; validated constructors and Deserialize paths are unchanged.
- Added `tests/tck_malformed_images.rs`, an external consumer-style test using public exports. It verifies exact serialized malformed fields, executes all three cases through A2aClient and McpLab against local endpoints, and observes `invalid` on both interfaces.
- Focused verification passed: foundation_tests, a2a_images, mcp_images, compliance_runner, and tck_malformed_images (38 tests total). Cargo metadata confirms no a2a-lab-tck dependency.
- Full `mise run quality` passed, including formatting, compile/clippy/tests, official upstream A2A TCK, Wiki checks, and SiLA provider checks. No commit, push, tag, release, migration bridge, or cleanup was performed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Exposed a narrowly typed public profile 1.1.0 TCK seam for exactly the three malformed image requests and used it in the existing compliance runner. Added public-consumer A2A/MCP coverage proving the malformed payload shapes survive serialization and both paths return the shared invalid code while normal validation remains intact. Focused checks, cargo metadata inspection, and `mise run quality` pass; no publication operation was performed.
<!-- SECTION:FINAL_SUMMARY:END -->
