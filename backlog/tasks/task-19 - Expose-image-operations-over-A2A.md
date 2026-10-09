---
id: TASK-19
title: Expose image operations over A2A
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 21:21'
labels:
  - a2a
dependencies:
  - TASK-18
references:
  - src/a2a/card.rs
  - src/a2a/client.rs
  - src/a2a/wire.rs
  - tests/a2a_tests.rs
priority: high
type: feature
ordinal: 19000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A2A agents need to discover and invoke the provider-neutral image contract with metadata paging and bounded inline bytes, using the same service behavior as existing lab operations. This exposure must remain JSON based and must not introduce protocol-specific camera behavior or URI-only delivery.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The agent card advertises list-image-sources, list-images, search-images, get-image, and get-current-image in addition to the existing skills, with exact catalog coverage in tests
- [x] #2 The typed A2aClient invokes all five image operations and exposes a configurable decoded-image maximum with the same 64 MiB default as the service
- [x] #3 Paged source and descriptor responses reconstruct in stable order across A2A artifact chunks, while get-image and get-current-image return byte-preserving base64 inline data in a single JSON data part
- [x] #4 A2A loopback coverage includes source-scoped listing, range and text search, specific and current retrieval, paging, malformed input, missing resources, provider unavailability, and client/server payload-limit failures
- [x] #5 A generated 3840x2160 RGBA-sized payload completes an A2A round trip under the default limit, fails under a lower configured limit, and succeeds when matching client and server limits are raised
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Advertise the five image skills on the agent card and update the exact skill catalog assertions.
2. Add A2aClient methods for the five operations, with ImageTransportConfig defaulting to 64 MiB and applied when image JSON is decoded.
3. Chunk and merge paged image source and descriptor results in stable order, leaving get-image and get-current-image as one JSON data part.
4. Cover listing, search, retrieval, paging, malformed input, missing resources, unavailability, and client/server payload limits, including the 3840x2160 RGBA round trip.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Advertised list-image-sources, list-images, search-images, get-image, and get-current-image after the existing lab skills and updated the exact catalog assertions.
- Added typed A2aClient methods and with_image_transport. Image results decode with that limit instead of Image Deserialize, which stays at 64 MiB.
- Chunked and merged paged source and descriptor results in order. get-image and get-current-image stay one JSON data part with standard base64.
- Loopback tests cover source-scoped listing, range and text search, specific and current retrieval, paging, malformed input, missing resources, provider unavailability, and client/server payload limits, including the 3840x2160 RGBA payload.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
A2A now advertises and invokes the five image operations with the same 64 MiB decoded-image default as the service. Paged source and descriptor results merge in order, and image bytes stay in one base64 JSON data part. The client applies its configured limit while decoding, so a raised maximum is not stuck at the 64 MiB Image Deserialize default.

Key files: src/a2a/card.rs, src/a2a/client.rs, src/a2a/wire.rs, tests/a2a_images.rs, tests/a2a_tests.rs. Checks: mise run fmt, mise run clippy, and nextest for a2a_tests, a2a_images, a2a_compliance, foundation_tests, and provider_tests. Full mise run quality was not run.

MCP image calls still return unavailable until TASK-20. A payload larger than 64 MiB was not sent over HTTP. The 3840x2160 RGBA payload is under that default, and the decode-limit behavior was also checked with a smaller configured maximum.
<!-- SECTION:FINAL_SUMMARY:END -->
