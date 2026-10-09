---
id: TASK-18
title: Integrate images into the lab service
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 20:56'
labels:
  - sdk
dependencies:
  - TASK-17
references:
  - src/service.rs
  - src/memory/mod.rs
  - tests/provider_tests.rs
priority: high
type: feature
ordinal: 18000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The shared lab service needs to route image operations through an opt-in provider while preserving existing callers that construct it with log, metric, and task providers. A deterministic in-memory image provider is required for examples, protocol tests, and provider conformance.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Existing source that calls A2aLabService::new(logs, metrics, tasks) still compiles and runs with image operations unavailable through an empty default provider, while with_images(provider) enables image operations without changing the three-provider constructor
- [x] #2 The shared command and result envelopes execute all five image operations, keep list and search results metadata-only, enforce the configured decoded-byte maximum before returning image payloads, and retain snapshots through the existing task API
- [x] #3 MemoryImages lists sources and source-scoped descriptors in stable paged order, applies half-open UTC and deterministic case-insensitive caption filtering, returns specific and current images, and enforces source/image relationships
- [x] #4 Unknown image or source IDs return not-found errors, and simulated provider unavailability is observable for each image operation
- [x] #5 Provider and service tests cover paging, combined and individual search criteria, boundary timestamps, current-image selection, payload-limit rejection, missing resources, and unavailable providers
- [x] #6 A2aLabService exposes a builder-style ImageTransportConfig setting so deployments can change the decoded-image maximum without replacing the preserved three-provider constructor
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Keep A2aLabService::new(logs, metrics, tasks) and add a default empty image provider plus with_images and with_image_transport.
2. Add the five image command and result variants, route them through the existing snapshot task store, and reject payloads above the configured decoded maximum before storing them.
3. Add MemoryImages with stable paging, half-open UTC filters, case-insensitive caption search, source/image checks, current-by-source retrieval, not-found errors, and simulated unavailability.
4. Cover provider and service behavior in provider tests, including the default-constructor compatibility path and configured payload rejection of images accepted under the 64 MiB decode default.
5. Add only the MCP client match arm required for the new command variants to compile; leave A2A, MCP exposure, docs, and SiLA behavior to later tasks.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Preserved A2aLabService::new(logs, metrics, tasks) with an empty image provider. with_images enables operations and with_image_transport sets the decoded maximum.
- Routed all five image operations through the shared command and result envelopes. List and search stay metadata-only. The configured maximum is checked before a payload is stored on the task.
- MemoryImages pages sources by id and descriptors by captured time then id, filters with a half-open UTC range and case-insensitive caption substring, selects the current frame by source, and returns not-found or unavailable errors.
- MCP client has a compile-only fallback that reports image operations unavailable. It does not add MCP tools.
- Verified with mise run fmt, mise run clippy, provider nextest (7), foundation nextest (17), kiss, and duplication. Did not run mise run quality.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Image operations now run through A2aLabService while A2aLabService::new(logs, metrics, tasks) stays source-compatible. An empty provider makes image calls unavailable until with_images, and with_image_transport sets the decoded maximum that is enforced before payloads are returned or stored.

Key files: src/service.rs, src/memory/images.rs, src/lib.rs, tests/provider_tests.rs. Checks: fmt, clippy with warnings denied, provider nextest, foundation nextest, kiss, and duplication. Full mise run quality was not run.

MCP rejects the new commands until image tools exist. Image JSON decode still uses the 64 MiB default, so a raised service limit does not raise decoding. Caption search is a case-insensitive substring.
<!-- SECTION:FINAL_SUMMARY:END -->
