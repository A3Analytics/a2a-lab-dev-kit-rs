---
id: doc-22
title: Adopt A2A-LAB compliance
type: guide
audience: public
created_date: '2026-10-09 04:10'
---

# Adopt A2A-LAB compliance

Run A2A-LAB compliance profile `1.1.0` against deterministic Agent2Agent (A2A) and Model Context Protocol (MCP) endpoints that expose the same implementation. The runner writes a JSON report and returns a status suitable for local development or a consumer-owned GitHub Actions workflow.

A2A-LAB compliance checks the scope described in the [A2A-LAB compliance profile](<../../reference/compliance/doc-23 - A2A-LAB-compliance-profile.md>). The default `full` suite includes the `basic` operation cases, 16 linked capability scenarios, and a default-on deterministic agent-message check. It is separate from the official A2A protocol Technology Compatibility Kit (TCK). A2A-LAB results do not establish A2A TCK status.

## Provide deterministic fixtures

Create `.a2a-lab/compliance-fixtures.json` in the implementation repository:

```json
{
  "suite_version": "1.1.0",
  "range": {
    "start": "2026-01-01T00:00:00Z",
    "end": "2026-01-02T00:00:00Z"
  },
  "logs": {
    "availability": "required",
    "fixtures": {
      "source_id": "logs.primary",
      "alternate_source_id": "logs.secondary",
      "missing_source_id": "logs.missing"
    }
  },
  "metrics": {
    "availability": "required",
    "fixtures": {
      "metric_id": "metrics.primary",
      "alternate_metric_id": "metrics.secondary",
      "missing_metric_id": "metrics.missing"
    }
  },
  "tasks": {
    "availability": "required",
    "fixtures": {
      "task_id": "tasks.primary",
      "alternate_task_id": "tasks.secondary",
      "run_id": "runs.primary",
      "missing_task_id": "tasks.missing",
      "missing_run_id": "runs.missing",
      "input": {
        "duration": 1
      }
    }
  },
  "images": {
    "availability": "required",
    "fixtures": {
      "source_id": "images.sources.primary",
      "alternate_source_id": "images.sources.secondary",
      "image_id": "images.primary",
      "alternate_image_id": "images.secondary",
      "missing_source_id": "images.sources.missing",
      "missing_image_id": "images.missing"
    }
  }
}
```

Configure both endpoints to return stable values for these identifiers and for the half-open fixture range. Provide at least two listable values where the profile tests pagination. Make the declared missing identifiers consistently return `not_found`. Make the task input produce a repeatable result, and reset mutable fixture state before each run.

Use in-memory providers, recorded responses, or another controlled software substitute for hardware-backed behavior. Physical robots, physical cameras, Docker simulators, live model providers, OpenID Connect (OIDC), and Standardization in Lab Automation (SiLA) are outside the badge profile. Tests of those systems can run in the same repository, but their results do not change what the A2A-LAB badge represents.

If an implementation does not offer one primitive, declare it unavailable:

```json
{
  "availability": "unavailable"
}
```

The endpoint must then return `unavailable` consistently for that primitive. The [compliance reference](<../../reference/compliance/doc-23 - A2A-LAB-compliance-profile.md>) defines which cases become required or skipped.

## Provide a deterministic agent-message fixture

The default full suite includes the LLM check. Configure the fixture's existing A2A agent-message handler with a deterministic fake completion model:

1. Expose `agent-message` in the A2A Agent Card.
2. Make the fake model choose the fixture's `list_tasks` MCP tool when the agent asks for task data.
3. Return a fixed response containing the exact task identifier supplied by the compliance prompt.
4. Preserve a nonblank A2A context identifier.
5. Keep the model, MCP endpoints, and fixture state in process or otherwise under test control.

This setup requires no Bedrock, OpenAI, Anthropic, AWS, or other external provider credentials. It exercises the implementation's agent-message integration with deterministic model behavior. It does not validate a live provider, compare provider quality, or prove from the compliance report that the model internally invoked a specific tool. The report proves only the externally observable assertions defined in the compliance reference.

If the implementation does not include model-backed behavior in its compliance claim, use the explicit LLM opt-out. Do not supply placeholder credentials or route compliance traffic to a live provider to make the deterministic fixture pass.

## Run the default profile locally

Start the deterministic implementation so its A2A endpoint is available at `http://127.0.0.1:3000` and its MCP Streamable HTTP endpoint is available at `http://127.0.0.1:3001/mcp`. From an A2A-LAB devkit checkout, run:

```bash
mise exec -- cargo run --locked --bin a2a-lab-compliance -- \
  --a2a-url http://127.0.0.1:3000 \
  --mcp-url http://127.0.0.1:3001/mcp \
  --fixtures .a2a-lab/compliance-fixtures.json \
  --profile 1.1.0 \
  --report target/a2a-lab-compliance.json \
  --implementation-name example-lab-agent \
  --implementation-version "$(git rev-parse HEAD)" \
  --timeout-milliseconds 30000
```

Omitting `--suite` selects `full`. Omitting `--no-llm-check` enables the LLM check. The report records:

- `"selected_suite": "full"`;
- `basic_operations`, `linked_scenarios`, and `a2a_agent_message` in `enabled_checks`; and
- 16 linked capability results plus `agent-message.a2a` in `scenarios`.

To run full without contacting the model endpoint, add the explicit opt-out:

```bash
mise exec -- cargo run --locked --bin a2a-lab-compliance -- \
  --a2a-url http://127.0.0.1:3000 \
  --mcp-url http://127.0.0.1:3001/mcp \
  --fixtures .a2a-lab/compliance-fixtures.json \
  --profile 1.1.0 \
  --suite full \
  --no-llm-check \
  --report target/a2a-lab-compliance.json \
  --implementation-name example-lab-agent \
  --implementation-version "$(git rev-parse HEAD)" \
  --timeout-milliseconds 30000
```

The opted-out report still records `"selected_suite": "full"`, but `enabled_checks` contains only `basic_operations` and `linked_scenarios`. Its `scenarios` contains the 16 linked capability results and omits `agent-message.a2a`.

To run only operation-level checks, replace `--suite full --no-llm-check` with `--suite basic`. A basic report contains only `basic_operations` in `enabled_checks` and has an empty `scenarios` array.

The command writes `target/a2a-lab-compliance.json` after every completed run. It exits with status `0` when every required enabled case and scenario passes and status `1` when the report is noncompliant. Invalid arguments, invalid fixtures, an unsupported profile, or a report-writing failure exit with status `2`; these setup failures might not produce a report.

## Run the profile in GitHub Actions

The action is consumed from the implementation repository, so that repository owns the workflow, retained report, and badge status.

The action is not published at a remotely fetchable revision yet. After a commit containing `action.yml` is published, replace the illustrative 40-character value in the following workflow with that published commit's full SHA. Do not use a branch, tag, abbreviated SHA, or the illustrative value.

Create `.github/workflows/a2a-lab-compliance.yml` in the implementation repository:

```yaml
name: A2A-LAB Compliance

on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:

permissions:
  contents: read

jobs:
  compliance:
    runs-on: ubuntu-latest
    steps:
      - name: Check out implementation
        uses: actions/checkout@08c6903cd8c0fde910a37f88322edcfb5dd907a8

      - name: Start deterministic compliance fixture
        run: ./scripts/start-compliance-fixture.sh

      # Replace this illustrative value with a published full devkit commit SHA.
      - name: Run A2A-LAB compliance
        id: compliance
        uses: A3Analytics/a2a-lab-dev-kit-rs@0123456789abcdef0123456789abcdef01234567
        with:
          a2a-url: http://127.0.0.1:3000
          mcp-url: http://127.0.0.1:3001/mcp
          fixtures: .a2a-lab/compliance-fixtures.json
          profile: "1.1.0"
          suite: full
          llm-check: "true"
          report-path: target/a2a-lab-compliance.json
          timeout-milliseconds: "30000"

      - name: Upload compliance report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: a2a-lab-compliance-report
          path: target/a2a-lab-compliance.json
          if-no-files-found: warn
```

The action defaults are `profile: "1.1.0"`, `suite: full`, and `llm-check: "true"`. They are shown explicitly so reviewers can see the intended scope in the workflow.

To opt out of the LLM check, use the same pinned action with `llm-check: "false"`:

```yaml
      - name: Run A2A-LAB compliance without the LLM check
        id: compliance
        uses: A3Analytics/a2a-lab-dev-kit-rs@0123456789abcdef0123456789abcdef01234567
        with:
          a2a-url: http://127.0.0.1:3000
          mcp-url: http://127.0.0.1:3001/mcp
          fixtures: .a2a-lab/compliance-fixtures.json
          profile: "1.1.0"
          suite: full
          llm-check: "false"
          report-path: target/a2a-lab-compliance.json
          timeout-milliseconds: "30000"
```

The action maps `"false"` to the local command's `--no-llm-check`. Its `profile`, `suite`, `llm-check`, `compliant`, and `report-path` outputs expose the selected controls and result. The retained JSON report records the effective suite and enabled checks as described for local runs.

The compliance step fails when a completed report is noncompliant. `if: always()` lets the upload step retain that report after either success or failure. A setup failure can occur before report creation, so `if-no-files-found: warn` keeps the missing evidence visible without hiding the original error.

Choose an artifact retention period that matches the implementation repository's evidence policy. The JSON report records profile `1.1.0`, selected suite, enabled checks including LLM-check state, implementation name and revision, interfaces, case and scenario outcomes, handoff paths, and diagnostics. The action's full immutable commit reference records the suite implementation revision; the JSON report does not duplicate that action commit SHA. Retain the workflow file or run metadata with the report so both revisions remain identifiable.

## Display the workflow badge

After the workflow exists on the default branch, replace `OWNER`, `REPOSITORY`, and `main` as needed, then add this standard GitHub Actions badge to the implementation repository:

```markdown
[![A2A-LAB Compliance](https://github.com/OWNER/REPOSITORY/actions/workflows/a2a-lab-compliance.yml/badge.svg?branch=main)](https://github.com/OWNER/REPOSITORY/actions/workflows/a2a-lab-compliance.yml?query=branch%3Amain)
```

The badge displays the native GitHub Actions status for the named workflow and branch. A passing status means the latest matching workflow run succeeded for the tested implementation revision, pinned devkit suite revision, profile `1.1.0`, selected suite, and LLM-check state configured by that run. Follow the badge link, inspect the retained report, and inspect the workflow's immutable action reference to identify that evidence.

The badge does not establish certification, exhaustive correctness, official A2A TCK status, behavior of an untested revision, live model quality, internal model tool invocation, or correctness of physical hardware, cameras, simulators, model providers, OIDC, or SiLA integrations. Do not copy the badge from another repository or replace it with a separately hosted static image.

## Related

- [A2A-LAB compliance profile](<../../reference/compliance/doc-23 - A2A-LAB-compliance-profile.md>)
- [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>)
- [Serve A2A and MCP](<../a2a-and-mcp/doc-13 - Serve-A2A-and-MCP.md>)

