# AGENTS.md

Operating guide for AI agents working in this repository.

## What this repo is

A2A-LAB devkit is a Rust library managed with **mise** (tooling + tasks), **Backlog.md** (task + doc management), and **Cursor** (agent skills). The library crate is `a2a-lab-dev-kit` and lives at the repository root. Human setup and the local quality commands live in `README.md`.

## Tooling

Everything runs through `mise`. The `backlog` binary is provided by mise and is **not on PATH** — always invoke it as `mise exec -- backlog ...`.

- Install tools: `mise install`
- List tasks: `mise tasks`
- Run a task: `mise run <task>`
- Configured tools: Rust 1.98.1, cargo-nextest, kiss-ai, watchexec, and Backlog.md (`mise.toml`)
- Configured tasks: `build`, `fmt`, `fmt-check`, `check`, `clippy`, `test`, `kiss`, `duplication`, `duplication-test-code`, `quality`, `web-ui-watch`, and `backlog-browser` (`.mise/run.toml`)

Add tools under `[tools]` in `mise.toml`; add tasks in `.mise/run.toml`.

### Rust build targets

- Reuse existing Cargo targets. Do not add a new build or test target unless the user explicitly requests one.
- Add integration tests as modules in an existing suite listed in `Cargo.toml`. Keep `autotests = false` so each test file does not become a separate executable.
- Use the existing `mise` tasks for routine Rust builds and tests. They use `--all-features` and share the default `target` directory. Keep feature-compatibility checks in their dedicated CI job.

### Rust build cache

- Keep routine builds in the shared `target` directory. Avoid per-task target directories and ad hoc changes to features, profiles, toolchains, or `RUSTFLAGS`. These can leave additional copies of compiled artifacts.
- For focused verification, select the existing integration suite with `mise run test -- --test <suite> -E '<filter>'` so Cargo links only that integration target. Run the full quality gate for final verification.
- Before a cold full build or after unexpectedly large growth, check `du -sh target` and `df -h .`. If `target` exceeds 50 GB or the disk has less than 50 GB free, diagnose the growth before starting another full build.
- When cleaning, check that builds and tests are idle. Preview host debug cleanup with `mise exec -- cargo clean --dry-run --profile dev --target-dir target`. Keep cleanup scoped to rebuildable output so reports, fixtures, and tool checkouts under `target` are preserved.

## Cursor skills

Repo-specific agent skills live in `.cursor/skills/`:

- `backlog` — author atomic, testable backlog tasks
- `doc` — author backlog docs
- `work` — implement a task to done

The skills include examples referencing a downstream app (`apps/web`, TanStack Start, shadcn). Ignore those paths. This repository is A2A-LAB devkit, and its quality gate is `mise run quality`.

## Documentation policy

No loose markdown files anywhere in the repo. All docs go through Backlog.md under `backlog/docs/`.

- `mise exec -- backlog doc list`
- `mise exec -- backlog doc view <id>`
- `mise exec -- backlog doc create "Title" -p <path> -t <type>`

Rules:

- Edit the generated file in `backlog/docs/`.
- Frontmatter must set `audience: technical` (internal) or `audience: public` (web).
- Routing: technical → `-t technical -p technical/<topic>`; other → `-t guide|reference|overview -p <path>`.
- Path is defined by directory structure, not frontmatter.
- No HTML comments in docs.

## Task policy

All task operations go through the Backlog.md CLI — **never hand-edit files** in `backlog/tasks/`. Direct edits break metadata and relationships. The entire `backlog/` directory is gitignored. Use `--plain` when reading for clean output.

Statuses (only these): `To Do`, `In Progress`, `Done`.

### Common commands

- Create: `mise exec -- backlog task create "Title" -d "Why" --ac "Testable outcome"`
- List / view: `mise exec -- backlog task list --plain` · `mise exec -- backlog task <id> --plain`
- Search: `mise exec -- backlog search "topic" --plain`
- Start: `mise exec -- backlog task edit <id> -s "In Progress" -a @me`
- Plan / notes / summary: `--plan "..."` · `--append-notes "..."` · `--final-summary "..."`
- Check criteria: `--check-ac 1 --check-ac 2` (one flag per index; no ranges or commas)
- Finish: `mise exec -- backlog task edit <id> -s Done`

For multi-line values use ANSI-C quoting: `--notes $'Line 1\nLine 2'`.

### Task quality

- Atomic (one PR), independent, and testable — acceptance criteria are observable outcomes, not implementation steps.
- Never reference a higher-numbered task.
- Description = the _why/what_; Implementation Plan (added only after starting) = the _how_.

### Definition of Done

All acceptance criteria checked · Implementation Notes written · Final Summary added · `mise run quality` passes · status set to `Done` via CLI.

Full CLI reference: `mise exec -- backlog --help`.

## Core rules

- Prefer editing existing files over creating new ones.
- Use `mise` tasks before raw binaries; never guess commands or paths.
- Keep code, comments, and responses terse.
- Don't install tools or dependencies without approval.
