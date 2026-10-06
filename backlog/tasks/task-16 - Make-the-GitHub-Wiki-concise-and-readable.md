---
id: TASK-16
title: Make the GitHub Wiki concise and readable
status: Done
assignee:
  - '@me'
created_date: '2026-10-06 20:11'
updated_date: '2026-10-06 20:57'
labels:
  - docs
dependencies: []
priority: high
ordinal: 16000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Readers need a short README and a GitHub Wiki they can scan without duplicated contracts or diagram errors. Public docs should separate getting started, task guides, and reference, and GitHub should render the diagrams itself.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 README.md is a short landing page with one quickstart, one native Mermaid diagram, and links that match .mise/wiki-map.toml.
- [x] #2 The Wiki map publishes Home, four example guides, and the six standards pages, and the staged sidebar groups those pages.
- [x] #3 Staged Wiki pages keep native Mermaid fences, accessibility titles and descriptions, and prose that restates each diagram.
- [x] #4 mise run wiki-check exits 0, and a pull request runs that check without a Wiki token.
- [x] #5 After the Wiki workflow publishes main, mise run wiki-drift matches the remote and every published page and diagram renders.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Stub four public guide docs for the runnable examples.
2. Rewrite README, Home, guides, and reference pages so each page answers one reader question.
3. Publish native Mermaid fences with accessibility metadata and grouped sidebar links.
4. Run wiki-check and quality, publish from main, then confirm drift and rendered pages.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Rewrote the README, overview, four guides, and six reference pages.
- Wiki pages keep native Mermaid fences with accTitle and accDescr.
- Pull requests run wiki-check without a token. mise run quality passed.

- mise run quality passed.
- The signed commit is blocked in this session because the 1Password SSH agent cannot complete the signature.

- Wiki workflow 37530032030 published 11 pages and wiki-drift matched the remote.
- Local mise run wiki-drift also matched. A2A routes keep literal {id}. All seven Mermaid diagrams render.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The README is a short landing page, and the Wiki now has an overview, four example guides, and six reference pages. GitHub renders the Mermaid diagrams from the published Markdown.

The Wiki workflow published the pages and both the workflow and a local drift check matched the remote. Quality had already passed before the commit.
<!-- SECTION:FINAL_SUMMARY:END -->
