---
id: TASK-22
title: Document and demonstrate image support
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 19:53'
updated_date: '2026-10-08 22:15'
labels:
  - docs
dependencies:
  - TASK-21
references:
  - examples/a2a_and_mcp.rs
  - backlog/docs/reference/primitives/doc-17 - A2A-LAB-primitives.md
  - backlog/docs/technical/protocol/doc-2 - A2A-and-MCP-protocols.md
  - backlog/docs/technical/architecture/doc-1 - Lab-dev-kit-architecture.md
  - backlog/docs/guide/custom-provider/doc-21 - Implement-a-custom-provider.md
priority: medium
type: docs
ordinal: 22000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implementers need an executable example and authoritative Backlog-managed guidance for the new provider-neutral image contract, its inline transport costs, and its configuration boundaries. Completion must also prove that the devkit remains source-compatible for the downstream OT-2 agent without adding OT-2 or SiLA image behavior.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The combined A2A and MCP example registers MemoryImages and successfully demonstrates image-source listing plus current and specific inline image retrieval
- [x] #2 The existing primitives, protocol, architecture, and custom-provider docs describe source and descriptor models, all five operations, provider-defined current-frame semantics, metadata-only list/search responses, inline base64 overhead, the 64 MiB default, raised-limit configuration, and deferred source/capture helpers
- [x] #3 The documented scope states that HTTP, USB, IP-camera, and custom are metadata kinds only and that MCP resources, URI-only delivery, built-in source adapters, and SiLA image commands are not provided
- [x] #4 mise run quality passes in a2a-lab-dev-kit-rs with the image example and Backlog-managed documentation included
- [x] #5 mise run check passes in the sibling a2a-lab-ot2 repository using its existing three-provider A2aLabService constructor, with no OT-2 source remapping or camera-specific changes
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Register MemoryImages on the combined A2A and MCP example and call list_image_sources, get_current_image, and get_image on both protocols.
2. Update the primitives, protocol, architecture, and custom-provider docs with the source and descriptor models, five operations, current-frame semantics, metadata-only list and search, inline base64 overhead, the 64 MiB default, raised-limit configuration, and deferred helpers. State that source kinds are metadata and that MCP resources, URI-only delivery, built-in adapters, and SiLA image commands are absent. Align the A2A, MCP, and serve-example pages that quote the old skill list or example output.
3. Run the example, mise run quality here, and mise run check in a2a-lab-ot2 without changing the three-provider constructor.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Registered MemoryImages in examples/a2a_and_mcp.rs. The example lists image sources and reads the current frame and image earlier on A2A and MCP. cargo run --example a2a_and_mcp printed bench, current 4, and earlier 3 on both protocols.
- Updated doc-17, doc-2, doc-1, and doc-21 with the source and descriptor models, five operations, provider-defined current frames, metadata-only list and search, base64 overhead, the 64 MiB default, raised limits, and deferred helpers. Kinds stay metadata. MCP resources, URI-only delivery, built-in adapters, and SiLA image commands are documented as absent.
- Aligned doc-4, doc-5, and doc-13 with the twelve lab operations and the example output.

- mise run quality passed, including fmt, the SiLA matrix, wiki-check, kiss, duplication, check, clippy, nextest, the A2A TCK, and the SiLA interop checks.
- mise run check in a2a-lab-ot2 failed first: A2aServer::new expects &Arc<dyn A2aLabApi> and the binary passed &McpLab. Wrapped that existing client in Arc. A2aLabService::new(lab.clone(), lab.clone(), lab) is unchanged, and no OT-2 camera or image source was added. The check then passed.
- ImageSourceKind remains.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The combined example now registers MemoryImages and reads image sources, the current frame, and one specific inline image over A2A and MCP. The primitives, protocol, architecture, and custom-provider docs describe the image contract, the 64 MiB default, raised limits, and the deferred capture scope.

Key files: examples/a2a_and_mcp.rs, doc-17, doc-2, doc-1, doc-21, plus doc-4, doc-5, and doc-13. Checks: cargo run --example a2a_and_mcp, mise run quality, and mise run check in a2a-lab-ot2. OT-2 needed an Arc wrap so A2aServer::new accepts the existing McpLab. ImageSourceKind stays. Public overview, implementations, and custom-role pages still omit images.
<!-- SECTION:FINAL_SUMMARY:END -->
