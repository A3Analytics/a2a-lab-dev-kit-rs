---
id: doc-12
title: Run the memory lab
type: guide
audience: public
created_date: "2026-10-06 20:11"
---

# Run the memory lab

Run the [logs, metrics, and tasks example](../../../../examples/logs_metrics_tasks.rs) to print logs, metrics, and tasks.

The example uses `MemoryLogs`, `MemoryMetrics`, and `MemoryTasks`.

## Run the example

Run these commands from the repository root.

1. Install the tools:

   ```bash
   mise install
   ```

2. Run the logs, metrics, and tasks example:

   ```bash
   mise exec -- cargo run --example logs_metrics_tasks
   ```

## Read the output

The example prints this text:

```text
log sources page 1: alpha next=Some("1")
log sources page 2: beta next=None
half-open [2024-01-01T00:00:00Z, 2024-01-01T01:00:00Z) kept: start, middle
metric latency sample 5
task build run run-1 Completed
```

Page 1 prints `alpha` and `next=Some("1")`.

Page 2 prints `beta` and `next=None`.

Sources print in id order.

Each page lists one source.

The printed range is a half-open Coordinated Universal Time (UTC) interval, `[start, end)`.

That range keeps the messages `start` and `middle`.

The message `end` is timestamped at the exclusive end.

The query omits that message.

Metric `latency` has one sample with value `5`.

Task `build` reports run `run-1` in state `Completed`.

## Related pages

Read the A2A-LAB devkit overview and the repository README:

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [README](../../../../README.md)
