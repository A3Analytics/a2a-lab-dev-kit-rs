---
id: TASK-17
title: Add image models and provider contract
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 20:40'
labels:
  - sdk
dependencies:
  - TASK-1
references:
  - /Users/dylangustaveson/.cursor/plans/add_image_primitives_9755bc4b.plan.md
  - src/id.rs
  - src/page.rs
priority: high
type: feature
ordinal: 17000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Lab integrations need a provider-neutral image contract that carries source metadata, searchable descriptors, and bounded inline image bytes without prescribing how HTTP, USB, IP-camera, or custom sources capture frames. The public SDK must define the five image operations and safe transport limits while leaving source helpers and built-in adapters out of scope.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The public API exposes validated ImageId and ImageSourceId values, ImageSource metadata with http, usb, ip_camera, and custom kinds, metadata-only ImageDescriptor values, and Image values whose bytes round-trip through the shared JSON profile as base64
- [x] #2 Requests cover listing image sources, listing images by source, searching by optional source with a half-open UTC range and/or nonblank text, retrieving an image by ID, and retrieving the current image by source; invalid IDs, pages, ranges, and searches with neither criterion are rejected
- [x] #3 Images reject empty or non-image media types, zero dimensions, malformed base64, and descriptor/payload inconsistencies at the public model boundary
- [x] #4 ImageTransportConfig defaults to a decoded 64 MiB maximum, rejects zero and overflow-prone limits, accepts a generated 3840x2160 RGBA-sized payload at the default, rejects it below its decoded size, and accepts it when the limit is raised
- [x] #5 ImageProvider exposes exactly the five provider-neutral operations, documents provider-defined current-frame semantics, and the crate exposes no source/capture helper or built-in HTTP, USB, or IP-camera adapter
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add validated ImageId and ImageSourceId using the existing identifier rules.
2. Add ImageSource, metadata-only ImageDescriptor, base64 Image, ImageTransportConfig, and the five request types, rejecting invalid IDs, pages, ranges, media types, dimensions, base64, and empty searches at the model boundary.
3. Add ImageProvider with exactly those five operations and provider-defined current-frame semantics, and export the contract from the crate root.
4. Promote base64 to a required dependency and verify each acceptance criterion with foundation tests.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added validated ImageId and ImageSourceId, source kinds, metadata-only descriptors, base64 images, ImageTransportConfig, five requests, and ImageProvider.
- Empty payloads are rejected as descriptor/payload inconsistencies. Image headers are not parsed.
- Verified with mise run fmt, cargo clippy --all-targets --all-features -D warnings, cargo nextest --test foundation_tests (17 passed), and mise run duplication. Did not run mise run quality.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The public SDK now defines a provider-neutral image contract: validated IDs, source metadata, metadata-only descriptors, inline base64 images, a 64 MiB decoded transport limit, and five ImageProvider operations. Current-frame retrieval names a source.

Key files: src/images/, src/id.rs, src/lib.rs, Cargo.toml, tests/foundation_tests.rs. Checks: fmt, clippy with warnings denied, foundation nextest, and duplication. Full mise run quality was not run. JSON image decode uses the default limit; a raised limit applies through ImageTransportConfig and Image::with_transport.
<!-- SECTION:FINAL_SUMMARY:END -->
