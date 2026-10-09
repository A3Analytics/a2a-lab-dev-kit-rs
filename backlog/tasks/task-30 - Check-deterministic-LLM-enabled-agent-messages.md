---
id: TASK-30
title: Check deterministic LLM-enabled agent messages
status: Done
assignee:
  - '@me'
created_date: '2026-10-09 15:58'
updated_date: '2026-10-09 16:21'
labels:
  - compliance
  - backlog-implementer
dependencies:
  - TASK-29
references:
  - task-29
  - src/a2a/message.rs
  - src/a2a/client.rs
priority: high
type: feature
ordinal: 30000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add the full-suite check for the devkit's actual LLM-enabled surface: the optional A2A agent-message skill whose handler may use MCP tools. The check must be deterministic and credential-free when an implementation supplies a fake model, verify a useful A2A-to-MCP handoff, and remain explicitly disableable for consumers that choose not to include model-backed behavior.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The local command exposes an LLM-check control that defaults to enabled and has an explicit opt-out; basic ignores the LLM scenario, while full records enabled, passed, failed, or opted-out state in the report
- [x] #2 When enabled, the runner requires the A2A Agent Card to advertise agent-message, obtains a task identifier through MCP, sends a deterministic plain-text A2A request grounded in that identifier, and requires the reply to preserve the expected identifier linkage
- [x] #3 The LLM scenario uses the existing A2A agent-message endpoint and MCP task tool surface and does not require or claim a plain-text MCP operation
- [x] #4 Missing agent-message advertisement, timeout, model or tool failure, ungrounded reply, or broken context data produces an actionable LLM scenario failure without preventing report creation
- [x] #5 Opting out does not contact a model endpoint, is visible in machine-readable evidence, and can still yield a compliant full report when every enabled required case passes
- [x] #6 Devkit tests use a deterministic in-process handler or fake model behavior and require no Bedrock, OpenAI, Anthropic, AWS, or other external model credentials
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a default-enabled full-suite LLM check control to the runner and CLI, with explicit opt-out evidence and no effect on basic.
2. Extend the compliance endpoint boundary to inspect the A2A Agent Card and send the existing plain-text agent-message request after discovering an exact task identifier through MCP.
3. Record isolated, actionable failures for advertisement, transport/timeout, model/tool behavior, reply grounding, and context integrity.
4. Add credential-free deterministic in-process tests for enabled, opted-out, and failure paths; run focused checks and mise run quality.
5. Verify every acceptance criterion, record notes and final summary, then mark TASK-30 Done.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added `ComplianceRunnerConfig::agent_message_check`; full executes it when enabled, basic ignores it, and the local command defaults it on with explicit `--no-llm-check`. Reports use the profile's existing evidence model: `enabled_checks` records enablement/opt-out and `agent-message.a2a` records pass/fail.
- The enabled scenario verifies Agent Card advertisement, discovers an exact task identifier through MCP `list_tasks`, sends a deterministic text/plain request through the existing A2A `agent_message` client path, and requires nonblank context plus the exact identifier in the reply. No plain-text MCP operation was added or claimed.
- Advertisement, MCP task-tool, model/handler, timeout, ungrounded reply, and broken-context failures are isolated as actionable scenario diagnostics and still produce a valid report.
- Credential-free fake-model endpoint tests cover grounded success, opt-out with zero model calls, basic ignoring the check, every required failure family, report validation, and CLI default/opt-out evidence.
- Verification passed: `mise run test` and the full `mise run quality` gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented the optional LLM-enabled `agent-message.a2a` full-suite scenario with default-on local CLI behavior and explicit `--no-llm-check` opt-out. The runner now checks A2A advertisement, carries an MCP-discovered task identifier into a deterministic plain-text A2A request, validates bounded reply/context linkage, and records isolated pass/fail or opt-out evidence without external model credentials.

Changed `src/compliance_runner.rs`, `src/bin/a2a-lab-compliance.rs`, and `tests/compliance_runner.rs`. `mise run test` and `mise run quality` pass; no TASK-31/32 action or documentation scope was implemented.
<!-- SECTION:FINAL_SUMMARY:END -->
