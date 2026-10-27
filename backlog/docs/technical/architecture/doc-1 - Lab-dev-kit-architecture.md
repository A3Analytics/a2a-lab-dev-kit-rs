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

Field definitions, timeline intervals, the 60-second clip limit, frame-at-time extraction, live follow, and the four availability codes are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Wire details are in [A2A and MCP protocols](<../protocol/doc-2 - A2A-and-MCP-protocols.md>).

The provider owns recording, indexing, and retention. It coalesces continuous coverage across rotated files or partitions into one logical interval. The timeline API rejects reversed, overlapping, or abutting intervals, so a genuine temporal gap stays a separate interval. Empty retention is a successful timeline with no intervals and no newest timestamp. `no_recording` applies only when a `get_video_clip` range misses coverage. The provider expires clip handles and cancels a live subscription when the consumer drops the stream or the provider stops the source. It supplies only metadata it actually has: a newest timestamp only inside a retained interval, and live availability only when follow can be prepared.

`get_video_frame_at` remains the public name. It extracts a frame from retained video. `get_video_clip` remains the public name for a half-open range of at most 60 seconds. `follow_video` returns a metadata-only live handle: an opaque id, the source id, the MJPEG media type, and a relative path. Agent2Agent (A2A) and Model Context Protocol (MCP) download clips at `GET /media/video-clips/{id}` and follow live video at `GET /media/video-live/{id}`. Responses are `no-store`. A bearer token is sent only in the `Authorization` header. There is no public route, byte range, or shared cache. On A2A those routes use the protocol access gate. On MCP, `/media/*` has no bearer gate, and the `/mcp` host allowlist does not cover it. SiLA 2 `LabVideos` 1.1 returns a timeline and a finite clip. It does not follow live video.

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
