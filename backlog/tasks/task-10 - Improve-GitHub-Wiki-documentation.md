---
id: TASK-10
title: Improve GitHub Wiki documentation
status: Done
assignee:
  - '@me'
created_date: '2026-09-30 18:10'
updated_date: '2026-09-30 18:15'
labels:
  - docs
dependencies: []
priority: high
ordinal: 10000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create source-backed documentation for every standard used by a2a-lab, publishable to the GitHub Wiki, with runnable examples and an accurate README landing page.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Six standards each have a public canonical Backlog documentation page and the overview links to them.
- [x] #2 Runnable examples demonstrate the in-memory, A2A plus MCP, ROS 2, and scripted industrial paths.
- [x] #3 README.md is a GitHub-renderable landing page with operations, quickstart, Wiki links, feature notes, and local API documentation instructions.
- [x] #4 Offline Wiki generation validates public docs, strips frontmatter, rewrites links, and produces Home.md plus _Sidebar.md.
- [x] #5 The repository quality gate passes with Wiki validation included.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Author code-backed public pages under backlog/docs for the overview and six supported standards.
2. Add runnable examples extracted from the existing tests.
3. Add offline Wiki generation, validation, publication, and drift commands.
4. Update README.md and repository metadata for GitHub readers.
5. Run examples, Wiki validation, and the full quality gate; record the authenticated Wiki publication limitation.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added seven public canonical docs (overview plus A2A, MCP, AAS, OPC UA, SiLA 2, and ROS 2) and corrected technical claims.
- Added four runnable examples and verified each with --no-default-features.
- Added offline Wiki staging/link validation plus publish and drift commands; Wiki publication remains blocked because the private Wiki Git remote reports repository not found even though the Wiki is enabled.
- Ran mise run quality: formatting, Wiki validation, kiss, duplication, cargo check, Clippy, and 27 nextest tests all passed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented the GitHub-facing documentation system for a2a-lab-sdk.

Key changes:
- Added source-backed public Backlog docs for the overview and six standards.
- Corrected technical A2A/MCP and industrial connector documentation.
- Added memory, A2A plus MCP, ROS 2, and scripted industrial examples.
- Reworked README.md as a GitHub landing page with quickstarts and Wiki links.
- Added offline Wiki generation, link checks, publication, and drift tasks; quality includes wiki-check.
- Renamed the Backlog project from New repo to a2a-lab-sdk.

Verification: mise run quality passed; all four examples compiled and ran. The Wiki is enabled, but publication was not pushed because the private Wiki Git remote was inaccessible.
<!-- SECTION:FINAL_SUMMARY:END -->
