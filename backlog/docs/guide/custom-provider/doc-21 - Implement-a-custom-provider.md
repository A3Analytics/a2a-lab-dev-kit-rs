---
id: doc-21
title: Implement a custom provider
type: guide
audience: public
created_date: "2026-10-06 23:17"
---

# Implement a custom provider

Implement `LogProvider`, `MetricProvider`, `TaskProvider`, `ImageProvider`, `VideoProvider`, or a combination of those traits. Pass the log, metric, and task implementations to `A2aLabService::new`. Pass an image implementation to `with_images`. Pass a video implementation to `with_videos`. [Custom](<../../reference/custom/doc-20 - Custom.md>) defines that provider role.

The following diagram shows where the implementation sits:

```mermaid
flowchart LR
  accTitle: Wire a custom provider
  accDescr: A custom type implements LogProvider, MetricProvider, TaskProvider, ImageProvider, or VideoProvider. A2aLabService::new accepts the first three. with_images accepts ImageProvider. with_videos accepts VideoProvider.
  custom["Custom type"] --> logs["LogProvider"]
  custom --> metrics["MetricProvider"]
  custom --> tasks["TaskProvider"]
  custom --> images["ImageProvider"]
  custom --> videos["VideoProvider"]
  logs --> service["A2aLabService::new"]
  metrics --> service
  tasks --> service
  images --> attached["A2aLabService::with_images"]
  videos --> videoAttached["A2aLabService::with_videos"]
```

In the preceding diagram, a custom type implements `LogProvider`, `MetricProvider`, `TaskProvider`, `ImageProvider`, or `VideoProvider`. `A2aLabService::new` accepts the log, metric, and task implementations. `A2aLabService::with_images` accepts `ImageProvider`. `A2aLabService::with_videos` accepts `VideoProvider`.

## Implement the traits

The provider type is `Send` and `Sync`. Each method returns `Result<_, A2aLabError>`.

`LogProvider` has two methods:

- `list_sources` returns a page of `LogSource`.
- `query` returns a page of `LogRecord` for one source.

`MetricProvider` has two methods:

- `list_metrics` returns a page of `MetricDescriptor`.
- `query` returns a page of `MetricPoint` for one metric.

`TaskProvider` has four methods:

- `list_tasks` returns a page of `TaskDefinition`.
- `start` accepts a run and returns a `TaskRun`. The run id is the task id.
- `status` returns that `TaskRun`.
- `cancel` stops a run. The default returns `A2aLabError::unavailable`.

`ImageProvider` has five methods:

- `list_image_sources` returns a page of `ImageSource`.
- `list_images` returns a page of `ImageDescriptor` for one source. The page is metadata only.
- `search_images` returns a page of `ImageDescriptor`. The search needs a half-open UTC range, nonblank text, or both. A source id can narrow it. A source id alone is not a search. The page is metadata only.
- `get_image` returns one `Image`, including its inline bytes.
- `get_current_image` returns the current `Image` for one source, including its inline bytes. The request names that source. It does not omit an image id. The provider defines the current frame. It can be newly captured, taken from video, or the latest stored image.

Source fields, descriptor fields, and inline bytes are defined in the [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). The provider supplies the frames. Saved still images remain exclusively in the Images primitive. Videos neither browses nor owns them.

`VideoProvider` has these methods:

- `list_video_sources` returns a page of `VideoSource`.
- `get_video_timeline` returns ordered retained coverage for one source. Coalesce continuous coverage across rotated files or partitions into one logical interval before returning it. The timeline API rejects reversed, overlapping, or abutting intervals. A genuine temporal gap remains a separate interval. Empty retention is a successful timeline with no intervals and no newest timestamp. Report live availability only when follow can be prepared. The default returns `unavailable` until the provider implements this method.
- `get_video_frame_at` remains the public name. It extracts a frame at time: the nearest retained JPEG at a UTC instant from retained video. Ties choose the earlier frame. Empty coverage, a request outside retained coverage, or a coverage gap is `not_found`. The extracted frame is not a saved image.
- `get_video_clip` remains the public name. It prepares a finite playback session for one half-open UTC range. There is no 60-second catalog limit. A range that misses retained coverage is `no_recording`. That code applies only to a missed clip range. A range that crosses a gap is `recording_gap` when the catalog requires contiguous coverage. Invalid or reversed ranges are `invalid`. The handle is metadata only.
- `open_video_clip` opens retained media for a previously prepared session. An expired handle is `clip_expired`. An unknown handle is `not_found`. The catalog owns that expiry.
- `follow_video` prepares a live-follow session. A source that cannot be followed is `live_unavailable`. The handle contains no frame bytes. The default returns `unavailable` until the provider implements this method.
- `open_video_follow` opens retained live media for a previously prepared session. Unknown handles are `not_found`. Cancel the subscription when the consumer drops the stream or when the provider stops the source. The default returns `unavailable` until the provider implements this method.

Do not implement HLS playlists, CMAF fragments, or HTTP inside a custom catalog or store. Implement `VideoCatalog` / `VideoProvider` and, when you own bytes, `DvrMediaStore`. Reuse `DvrMediaPackager` for bounded HLS/CMAF. Serve assets through `MediaAssetProvider`: `LocalMediaAssetProvider` for in-process bytes, or a remote provider in tests without a live external deployment.

The kit supplies contracts and transport. It does not capture cameras and does not persist recordings. The catalog owns recording, indexing, retention, session expiry, and live-subscription cancellation. Supply only metadata the provider actually has: omit a newest timestamp when nothing is retained, and do not invent resolution, ranges, timestamps, or live availability. Agent2Agent (A2A) and Model Context Protocol (MCP) return a relative `/media/video-clips/{id}/` gateway root for a finite session and a relative `/media/video-live/{id}/` root for live follow. Those responses carry no video bytes. A bearer token is sent only in the `Authorization` header. When MCP security requirements are configured, `/media/video-*` uses the access gate. The `/mcp` host allowlist does not cover those routes.

`Image::new` uses the 64 MiB default. A provider that creates a larger payload uses `Image::with_transport`. Set the same decoded-byte maximum on the service with `A2aLabService::with_image_transport` and on each client with `A2aClient::with_image_transport` or `McpLab::connect_with`. Pass a raised Model Context Protocol (MCP) limit to `connect_with` before the connection opens.

Return pages with `Page::new`. Leave the next cursor absent on the last page. Read the requested page from `request.page.cursor()` and `request.page.limit()`.

A query keeps records or samples inside `request.range`. The range is a half-open Coordinated Universal Time (UTC) interval, `[start, end)`. An unknown id returns `A2aLabError::not_found`.

When `start` is called with `wait: true`, `A2aLabService` polls `status` until the state is terminal. The default wait is 60 seconds. `completed`, `failed`, and `canceled` are terminal. Return `Completed` from `start` when the work is already finished.

The [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>) define the identifiers, timestamps, pages, and error codes.

## Pass the providers to the service

`A2aLabService::new` takes one log provider, one metric provider, and one task provider. Image operations stay unavailable until `with_images`. Video operations stay unavailable until `with_videos`. Use `MemoryLogs`, `MemoryMetrics`, `MemoryTasks`, `MemoryImages`, or `MemoryVideos` for a primitive the custom type leaves to this crate. Attach image and video providers without replacing that constructor:

```rust
let service = A2aLabService::new(OvenLogs::new()?, MemoryMetrics::new(), MemoryTasks::new())
    .with_images(MemoryImages::new())
    .with_videos(MemoryVideos::new());
```

The [custom provider example](../../../../examples/custom_provider.rs) implements `LogProvider` for one oven log and uses the in-memory metric and task providers:

```rust
let service = A2aLabService::new(OvenLogs::new()?, MemoryMetrics::new(), MemoryTasks::new());
```

`OvenLogs::list_sources` returns the source `oven`. `OvenLogs::query` returns the record `ready` when that record's timestamp is inside the requested range.

## Run the example

Run this command from the repository root:

```bash
mise exec -- cargo run --example custom_provider
```

The example prints this text:

```text
log source: oven
```

## Use the SiLA provider

A remote SiLA 2 server does not need a new trait implementation. `SilaProvider::connect` reads `SilaProviderConfig` and returns one value that implements `LogProvider`, `MetricProvider`, and `TaskProvider`. It does not implement `ImageProvider` or `VideoProvider`. The inbound `SilaServer` serves images only when the lab service has an `ImageProvider`, and videos only when it has a `VideoProvider`. The three-provider constructor below leaves image and video operations unavailable.

```rust
let provider = SilaProvider::connect(config).await?;
let service = A2aLabService::new(provider.clone(), provider.clone(), provider);
```

[SiLA 2](<../../reference/sila-2/doc-8 - SiLA-2.md>) defines the endpoint, certificate, and binding configuration. The [SiLA provider example](../../../../examples/sila_provider.rs) (`examples/sila_provider.rs`) prints `log record: ready`.

Run this command from the repository root:

```bash
mise exec -- cargo run --example sila_provider
```

## Related

- [Custom](<../../reference/custom/doc-20 - Custom.md>)
- [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>)
- [Implementations](<../../reference/implementations/doc-18 - Implementations.md>)
- [Run the memory lab](<../memory-lab/doc-12 - Run-the-memory-lab.md>)
