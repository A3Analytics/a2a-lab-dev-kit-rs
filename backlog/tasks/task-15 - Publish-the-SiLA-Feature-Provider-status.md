---
id: TASK-15
title: Publish the SiLA Feature Provider status
status: Done
assignee:
  - '@me'
created_date: '2026-10-06 06:40'
updated_date: '2026-10-06 07:16'
labels:
  - sila
dependencies:
  - TASK-14
priority: high
ordinal: 15000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Readers need the public SiLA reference and repository badge to describe the provider role without claiming certification.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The public SiLA reference documents LabOperations, identity, discovery, cancellation, and the excluded optional features.
- [x] #2 The README badge points at the provider interop workflow and does not use the official SiLA logo or the word certified.
- [x] #3 mise run quality runs the provider interop after the existing checks.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Document the Feature Provider, its exclusions, and the interop pin in the public SiLA reference.
2. Point the README badge at the provider workflow without a certification claim.
3. Run the provider interop from mise run quality.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- doc-8 describes the provider, LabOperations, identity, discovery, cancellation, and the excluded optional features.
- The README badge is labeled SiLA 2 provider and links to the provider workflow.
- mise run quality calls sila2-interop after the A2A TCK.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The public SiLA reference describes the Feature Provider, LabOperations, identity, discovery, cancellation, and the optional features this profile leaves out. The README badge points at the provider interop workflow and does not use the official logo or claim certification.

mise run quality runs that interop after the existing checks and completed green.
<!-- SECTION:FINAL_SUMMARY:END -->
