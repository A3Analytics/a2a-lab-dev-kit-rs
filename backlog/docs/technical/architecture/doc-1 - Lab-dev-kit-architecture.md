---
id: doc-1
title: A2A-LAB devkit architecture
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A-LAB devkit architecture

## Purpose

A2A-LAB devkit gives an automated lab agent one Rust API for logs, metrics, tasks, images, and optional videos. Protocol adapters call that API. They do not own the data.

## Providers

Implement these traits:

- `LogProvider` lists sources and queries structured records.
- `MetricProvider` lists descriptors and queries finite samples.
- `TaskProvider` lists definitions, starts a run, and returns its status.
- `ImageProvider` lists image sources, lists and searches metadata, and returns inline frames.
- `VideoProvider` lists video sources, returns a retained timeline, extracts a frame at time with `get_video_frame_at`, prepares bounded clip handles, and prepares live-follow handles. It cancels a live subscription when the consumer drops the stream or the provider stops the source.

`A2aLabService::new` still takes the log, metric, and task providers. Image operations stay unavailable until `with_images`. Video operations stay unavailable until `with_videos`. That keeps the three-provider constructor source-compatible.

`A2aLabService` validates page limits and UTC ranges.

It calls the matching provider.

It stores an Agent2Agent (A2A) task snapshot.

`MemoryLogs`, `MemoryMetrics`, `MemoryTasks`, `MemoryImages`, and `MemoryVideos` are in-memory implementations for examples and tests. `MemoryVideos` is not durable storage.

## Images

Field definitions, the five image operations, and transport limits are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Agent2Agent (A2A) and Model Context Protocol (MCP) wire details are in [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>).

This crate does not open an HTTP endpoint, a USB device, or an IP camera. It does not include a source helper, a capture helper, or a built-in source adapter. MCP exposes tools only: no image resources and no URI-only delivery. SiLA 2 `LabImages` lists sources and returns one image through binary download. `LabOperations` stays the seven log, metric, and task commands. Saved still images remain exclusively in the Images primitive. Videos neither browses nor owns them.

## Videos

Field definitions, timeline intervals, playback sessions, frame-at-time extraction, live follow, and the four availability codes are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Wire details are in [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>).

Video work is split so a custom backend does not implement HLS or HTTP:

- `VideoCatalog` (every `VideoProvider`) lists sources, returns retained timelines, extracts a frame at time, and prepares opaque finite and live playback sessions. Catalog JSON never includes media bytes, playlists, or credentials.
- `DvrMediaStore` reads retained media windows. It does not speak HTTP.
- `DvrMediaPackager` turns a store window into bounded HLS/CMAF (or leaves MJPEG as multipart). Packaging is shared. Catalogs and stores do not implement it.
- `MediaAssetProvider` is the gateway: relative `/media/video-clips/{id}/` and `/media/video-live/{id}/` assets. `LocalMediaAssetProvider` is the in-process seam. A remote store can plug in without a live external deployment in tests.
- The browser player fetches those gateway paths with the bearer only in `Authorization`, then plays HLS through Media Source Extensions or paints MJPEG on a canvas into `#video-player`.

The catalog owns recording, indexing, and retention. It coalesces continuous coverage across rotated files or partitions into one logical interval. The timeline API rejects reversed, overlapping, or abutting intervals, so a genuine temporal gap stays a separate interval. Empty retention is a successful timeline with no intervals and no newest timestamp. `no_recording` applies only when a `get_video_clip` range misses coverage. The catalog expires session handles and cancels a live subscription when the consumer drops the stream or the provider stops the source. It supplies only metadata it actually has: a newest timestamp only inside a retained interval, and live availability only when follow can be prepared.

`get_video_frame_at` remains the public name. It extracts a frame from retained video. `get_video_clip` remains the public name for one half-open retained range. There is no 60-second catalog limit; the player requests a bounded HLS window (default 120 seconds) inside a longer selection. `follow_video` returns a metadata-only live session: an opaque id, the source id, media type, optional 8x trick-play metadata, and a relative gateway root. Agent2Agent (A2A) and Model Context Protocol (MCP) serve session assets under `GET /media/video-clips/{id}/` and `GET /media/video-live/{id}/`. Every gateway response uses `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. A bearer token is sent only in the `Authorization` header. There is no public route, byte range, shared cache, or credential in a query. When security requirements are configured, those `/media/video-*` routes use the same access gate as protocol routes on both the A2A and MCP listeners. SiLA 2 `LabVideos` 1.1 returns a timeline and a finite BinaryDownload clip. It does not follow live video.

This crate supplies contracts and transport. It does not capture cameras and does not persist recordings.

## Shared values

Identifiers are non-empty ASCII tokens. Timestamps are UTC. `TimeRange` is half-open: the start is included and the end is excluded. Pages use an opaque numeric cursor and a limit from 1 to 1000. Task input is a JSON object. `A2aLabError` uses the stable codes `invalid`, `not_found`, `unavailable`, `transport`, and `protocol`. Video availability also uses `no_recording`, `recording_gap`, `clip_expired`, and `live_unavailable`.

## Adapters

`A2aServer` speaks HTTP+JSON, JSON-RPC, and gRPC.

`A2aClient` speaks HTTP+JSON.

`McpServer` registers the same operations as Model Context Protocol (MCP) tools.

It serves standard input and output, or Streamable HTTP.

The default A2A agent uses `McpLab` to call that MCP server.

`A2aServer::new` accepts any `A2aLabApi`, including `A2aLabService`.

`SilaServer` is the inbound [SiLA 2](<../../reference/sila-2/doc-8 - SiLA-2.md>) Feature Provider.

`Ros2Tasks` implements `TaskProvider` for [ROS 2](<../../reference/ros-2/doc-9 - ROS-2.md>) actions.

See [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>) for the wire contracts.

See [Industrial equipment connectors](<../industrial/doc-3 - Industrial-equipment-connectors.md>) for the AAS, OPC UA, and SiLA 2 connectors.

## Optional web interface

The `web-ui` Cargo feature packages a browser shell. `A2aServer::with_web_ui` serves that shell at `/ui/` on the HTTP listener that already speaks A2A. The default build has no `/ui/` route and no `with_web_ui` method. Calling the method is a second opt-in after the feature is enabled.

The browser is a same-origin A2A HTTP+JSON client. It is not a new provider or a new protocol. It browses the primitives on `A2aLabService`: logs, metrics, tasks, images, and optional videos. Saved still images stay on the Images page. Videos does not browse them. It sends the seventeen `A2aLabCommand` variants:

- `ListLogSources`
- `QueryLogs`
- `ListMetrics`
- `QueryMetric`
- `ListTasks`
- `StartTask`
- `GetTaskStatus`
- `ListImageSources`
- `ListImages`
- `SearchImages`
- `GetImage`
- `GetCurrentImage`
- `ListVideoSources`
- `GetVideoFrameAt`
- `GetVideoClip`
- `GetVideoTimeline`
- `FollowVideo`

Protocol tasks stay in the server's in-memory task store. Lab command snapshots stay in `A2aLabService` memory. Both disappear when the process stops. The browser keeps the current chat `contextId` in page memory only. [Open the optional web interface](<../../guide/web-interface/doc-22 - Open-the-optional-web-interface.md>) covers the example and that page-local conversation.
