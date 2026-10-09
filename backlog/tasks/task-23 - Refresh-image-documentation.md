---
id: TASK-23
title: Refresh image documentation
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 22:31'
updated_date: '2026-10-08 22:40'
labels:
  - docs
dependencies: []
priority: medium
ordinal: 23000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Public docs still describe parts of the crate as logs, metrics, and tasks only, and the image pages repeat transport details without explaining when a 4K frame fits and when a larger image needs a higher limit.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 README, overview, implementations, custom, industrial, and SiLA docs name the current image scope without claiming SiLA or industrial connectors expose image operations
- [x] #2 Primitives are the canonical image model and explain decoded-byte limits, 4K fit, larger payloads, Image::with_transport, and matching service and client configuration
- [x] #3 Architecture and protocol docs keep only their ownership and link to the primitives instead of repeating the full image model
- [x] #4 A2A, MCP, the combined example guide, the custom-provider guide, and the memory-lab guide match the current examples and the MCP connect_with limit rule
- [x] #5 No new document is created, ImageSourceKind is not documented, and mise run wiki-check and mise run quality pass
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Update README, overview, implementations, custom, industrial, and SiLA pages so images are in scope and SiLA plus industrial connectors stay on logs, metrics, and tasks. Disambiguate SiLA interop image ids as container-image identifiers.
2. Make primitives the canonical image model: decoded-byte limits, matching service and client configuration, 4K fit, larger payloads, and Image::with_transport.
3. Trim architecture and protocol pages to ownership and wire behavior, with links to the primitives.
4. Sync the A2A example narrative and output with a2a_and_mcp.rs, and add MCP connect_with, custom-provider, serve, and memory-lab guidance.
5. Run the combined example, wiki-check, and quality, then check each acceptance criterion.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Scoped README, overview, implementations, custom, industrial, and SiLA docs to the twelve A2A and MCP operations, with SiLA and industrial connectors on logs, metrics, and tasks. SiLA interop image ids are container-image identifiers.
- Primitives now state decoded-byte limits, the 31.6 MiB 4K fit, larger PNG payloads, Image::with_transport, and matching service and client configuration.
- Architecture and protocol pages link to the primitives and keep ownership or wire behavior.
- A2A example output matches cargo run --example a2a_and_mcp. MCP, the serve guide, the custom-provider guide, and the memory-lab guide cover connect_with and MemoryImages.
- wiki-check passed. ImageSourceKind and the seven-operation contract wording are absent from the docs.

- mise run quality passed, including wiki-check, tests, the A2A TCK, and the SiLA checks. No new document was created.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Public docs now treat images as part of the twelve A2A and MCP operations. SiLA and the industrial connectors stay on logs, metrics, and tasks. Primitives own the image model, including decoded-byte limits, the 4K fit, larger payloads, Image::with_transport, and matching service and client limits. Architecture and protocol pages keep ownership and wire behavior and link back to the primitives.

Key files: README.md, doc-10, doc-18, doc-20, doc-3, doc-8, doc-17, doc-2, doc-1, doc-4, doc-5, doc-13, doc-21, and doc-12. Checks: cargo run --example a2a_and_mcp, mise run wiki-check, and mise run quality. Residual risk: doc-19 still describes interfaces as exposing the primitives without restating the SiLA subset, and the 24 megapixel note is sizing guidance rather than a new fixture.
<!-- SECTION:FINAL_SUMMARY:END -->
