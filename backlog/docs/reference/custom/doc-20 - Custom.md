---
id: doc-20
title: Custom
type: reference
audience: public
created_date: '2026-10-06 23:14'
---

# Custom

A custom provider supplies lab logs, metrics, or tasks for equipment that uses its own interface.

## Role in A2A-LAB devkit

Custom is a provider. Implement `LogProvider`, `MetricProvider`, `TaskProvider`, or a combination of those traits. Pass the implementations to `A2aLabService`. A service can mix built-in providers and a custom provider.

The following diagram shows that contract:

```mermaid
flowchart LR
  accTitle: A custom provider fulfills primitives
  accDescr: A2aLabService uses a custom provider. The custom provider implements LogProvider, MetricProvider, or TaskProvider.
  lab["A2aLabService"] --> custom["Custom provider"]
  custom --> logs["LogProvider"]
  custom --> metrics["MetricProvider"]
  custom --> tasks["TaskProvider"]
```

In the preceding diagram, `A2aLabService` uses a custom provider. The custom provider implements `LogProvider`, `MetricProvider`, or `TaskProvider`.

The [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>) define that contract. [Implement a custom provider](<../../guide/custom-provider/doc-21 - Implement-a-custom-provider.md>) shows how to write one. [Implementations](<../implementations/doc-18 - Implementations.md>) lists the built-in providers.

## Example

`A2aLabService` uses `OvenLogs` for logs, `MemoryMetrics` for metrics, and `MemoryTasks` for tasks.

```mermaid
flowchart LR
  accTitle: A2aLabService uses OvenLogs and memory providers
  accDescr: A2aLabService uses OvenLogs for logs, MemoryMetrics for metrics, and MemoryTasks for tasks.
  service["A2aLabService"] -->|logs| oven["OvenLogs"]
  service -->|metrics| metrics["MemoryMetrics"]
  service -->|tasks| taskProvider["MemoryTasks"]
```

In the preceding diagram, `A2aLabService` uses `OvenLogs` for logs, `MemoryMetrics` for metrics, and `MemoryTasks` for tasks.

```rust
let service = A2aLabService::new(OvenLogs::new()?, MemoryMetrics::new(), MemoryTasks::new());
let outcome = service
    .execute(A2aLabCommand::ListLogSources(ListLogSourcesRequest {
        page: PageRequest::new(None, 100)?,
    }))
    .await?;
```

Run this command from the repository root:

```bash
mise exec -- cargo run --example custom_provider
```

The example prints:

```text
log source: oven
```

The full source is [custom_provider.rs](../../../../examples/custom_provider.rs). [Implement a custom provider](<../../guide/custom-provider/doc-21 - Implement-a-custom-provider.md>) walks through the same program.

## Related

- [Implement a custom provider](<../../guide/custom-provider/doc-21 - Implement-a-custom-provider.md>)
- [Interfaces and providers](<../interfaces-and-providers/doc-19 - Interfaces-and-providers.md>)
- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [A2A-LAB primitives](<../primitives/doc-17 - A2A-LAB-primitives.md>)
- [Implementations](<../implementations/doc-18 - Implementations.md>)
