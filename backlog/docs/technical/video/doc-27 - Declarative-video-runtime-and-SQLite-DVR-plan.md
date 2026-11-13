---
id: doc-27
title: Declarative video runtime and SQLite DVR plan
type: specification
audience: technical
created_date: "2026-11-12 22:07"
---

# Declarative video runtime and SQLite DVR plan

## Target architecture

`DeclarativeVideoRuntime` owns camera discovery, Nokhwa capture, bounded
draining, color conversion, OpenH264 live encoding, the sliding
`H264LiveStore`, and historical playback. Callers provide `VideoConfig` and
receive a `VideoProvider`, warm still cache, settings handle, and DVR handle.

## Declarative configuration

Each video source accepts `camera_id` as a non-negative integer index or a
non-empty text selector. Text is matched case-insensitively against
`CameraInfo.human_name`, `description`, and `misc`. An omitted selector chooses
the first unclaimed camera advertising `MJPEG` or `YUYV`; zero-profile
metadata nodes are ignored. The legacy `device_index` alias remains accepted
when `camera_id` is absent. Omitted `format` means `"auto"`, which probes
supported `MJPEG` and `YUYV` profiles without operating-system conditionals.

## SQLite DVR boundary

`SqliteChunkDvrStore` is a non-optional devkit dependency backed by `sqlx`
SQLite. It persists recoverable CRC32 minute chunks, immutable H.264 epochs,
startup tail recovery, and retention cleanup in `index.sqlite`. Media callers
use `media_pool()`. Semantic manifests, jobs, embeddings, and decisions live
in a second file opened by `SemanticSqliteStore`. `DvrPersistHook` runs after
the media transaction commits and must not share that connection.

## Module and quality constraints

The implementation keeps new traits in dedicated files and splits modules to
respect `.kissconfig`: at most one trait per file, fewer than 441 statements,
1,567 lines, and 40 methods per struct. No feature or behavior is gated by
`target_os`.

## Wave order

DK-1 through DK-5 establish configuration, camera behavior, codec performance,
DVR persistence, and browser regression coverage. DK-6 through DK-8 add
discovery, capture draining, live encoding, and settings watches. DK-9 adds
live/historical provider routing; DK-10 composes the runtime and is the
cross-repository gate. OT-2 then performs storage/config mapping, runtime
cutover, and module deletion in dependency order.

## Backlog tasks

- DK-1: TASK-138
- DK-2: TASK-139
- DK-3: TASK-140
- DK-4: TASK-141
- DK-5: TASK-142
- DK-6: TASK-143
- DK-7: TASK-144
- DK-8: TASK-145
- DK-9: TASK-146
- DK-10: TASK-147
