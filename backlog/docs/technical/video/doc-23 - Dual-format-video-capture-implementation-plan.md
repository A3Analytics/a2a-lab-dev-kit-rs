---
id: doc-23
title: Dual-format video capture implementation plan
type: technical
audience: technical
created_date: '2026-11-04 21:27'
---

# Dual-format video capture implementation plan

Deliver YUY2/YUYV and MJPEG capture with reusable Rust media code in the devkit and application/vendor wiring in OT-2. Orbbec color must explicitly normalize to YUY2. Preserve current JPEG consumers and 5 FPS H.264 delivery while retiring the FFmpeg subprocess after compatibility checks.

Status: planned, with 30 small To Do tasks across the two repositories. This document records desk research and a delivery backlog; it does not report implementation or benchmark results.

Companion: [the OT-2 integration plan](../../../../../a2a-lab-ot2/backlog/docs/technical/video/doc-5%20-%20Dual-format-video-capture-implementation-plan.md). Task IDs in prose are always qualified by the repository/column containing them.

## Inspected baseline

Devkit checkout: commit `1ed5b93` on 2026-11-04. Its current video module provides storage contracts, packaging, and HTTP/protocol seams, and explicitly does not capture cameras. The optional capture/conversion/encoder boundary is new. Existing `DvrMediaTrack` binds one initialization blob to a track; `DvrMediaPackager` selects one track per session, and CMAF flags currently assume independent samples.

Read existing `src/videos/{mod,codec,dvr_store,packager,cmaf}.rs`, `src/memory/videos.rs`, and the focused video/backend seam tests as the relevant task needs them. Keep SQLite schemas and vendor SDKs downstream. Existing completed video tasks remain the behavioral baseline, not work to recreate.

## Library decision

The selected stack is the best fit from desk research for these requirements: retain existing capture support, avoid FFmpeg at runtime, keep conversion in Rust, use an established H.264 codec, and limit native build dependencies. This is an implementation choice, not a measured performance claim. Recheck APIs, release notes, license notices, and dependency advisories when each dependency-bearing task starts; pin the reviewed versions and lockfile then.

| Work | Selected library | Why this choice | Alternative and trigger |
| --- | --- | --- | --- |
| Webcam capture | `nokhwa` 0.10.11, optional `input-native` adapter | Already used by OT-2; exposes advertised modes and raw frame buffers through native capture backends. Preserve it rather than build per-OS capture. | Direct V4L2/Media Foundation/AVFoundation code adds ownership and portability work without a demonstrated need. |
| YUY2/I420/RGB/BGR conversion | `yuv` from awxkee/yuvutils-rs, candidate 0.8.20 | Pure Rust with stride-aware packed/planar APIs and SIMD implementations; avoids another C/C++ library. | `libyuv` through Rust FFI is the mature native alternative if fixture correctness or measured performance fails. No second converter by default. |
| JPEG decode and still encode | `image` 0.25.10 with `default-features = false`, `features = ["jpeg"]` | Established Rust image API; JPEG decoding uses zune-jpeg, and JPEG encoding and decode limits are available in one dependency. | Direct `zune-jpeg` APIs only if profiling establishes a material benefit or necessary camera-JPEG behavior unavailable through image. Avoid adding a second JPEG stack preemptively. |
| Software H.264 | `openh264` 0.9.9, bundled `source` feature | Rust API to Cisco OpenH264; persistent encoder and owned picture handling can replace the FFmpeg subprocess. | Retain existing x264 recordings for compatibility. FFmpeg/libx264 requires external/native packaging; experimental pure-Rust H.264 encoders are not the default for this application. |

OpenH264 is a C++ codec behind a Rust wrapper. It needs a C++ build toolchain and verified runtime/static linking; it is FFmpeg-free, not entirely Rust. Its suitability for current GNU/musl/Apple artifacts is a release gate, not an assumption. Hardware encoding is outside this first implementation.

### Verified API direction

- Use Nokhwa `Camera::compatible_camera_formats`, `frame`/`frame_raw`, and returned format information; avoid unconditional `decode_image`.
- Use `yuv::yuyv422_to_yuv420` for raw input to I420. Use `rgb_to_yuv422` / `bgr_to_yuv422` plus `yuv422_to_yuyv422` for packed Orbbec output. Use the library's YUYV-to-RGB conversion for still JPEG derivatives. The published package index listed yuv 0.8.20 during this review while the API page showed 0.8.19; confirm the chosen release signatures before pinning.
- Use `image::ImageReader` with JPEG format and strict limits for decoding, and `image::codecs::jpeg::JpegEncoder` for raw-source JPEG derivatives. No implicit decoder allocation without input/dimension limits.
- Use `openh264::encoder::Encoder` / `encode_at`, explicit `EncoderConfig`, and a validated I420 `YUVSource`. Copy borrowed output before the next encode. Output FPS, bitrate, VUI, frame-skip policy, and intra policy are explicit.

Sources checked 2026-11-04: [Nokhwa Camera](https://docs.rs/nokhwa/0.10.11/nokhwa/struct.Camera.html), [yuv package](https://docs.rs/crate/yuv/latest), [yuv APIs](https://docs.rs/yuv/latest/yuv/), [image JPEG features](https://docs.rs/crate/image/latest/features), [ImageReader](https://docs.rs/image/latest/image/struct.ImageReader.html), [JPEG encoder](https://docs.rs/image/latest/image/codecs/jpeg/struct.JpegEncoder.html), [OpenH264 wrapper](https://github.com/ralfbiedert/openh264-rs), [EncoderConfig](https://docs.rs/openh264/latest/openh264/encoder/struct.EncoderConfig.html), [bundled source feature](https://docs.rs/crate/openh264/latest/features).

## Ownership and data flow

| Devkit owns reusable behavior | OT-2 owns application behavior |
| --- | --- |
| Color frame/capability contracts, stride and allocation checks, profile/rate selection, optional Nokhwa capture adapter | Source identifiers, camera selection/configuration and diagnostics |
| `image` JPEG handling, `yuv` conversion, lazy JPEG derivative API, bounded fresh-frame selection | Orbbec SDK access, serial/profile enum mapping, shared session and synchronized channel association |
| Persistent OpenH264 adapter, complete-picture framing, codec-configuration identity, epoch-aware packaging contracts and fixtures | SQLite/chunk persistence/migration, retention policy, source fanout, endpoint registration and lifecycle |
| Optional Cargo features, reusable HLS/CMAF/MP4 packaging, portable fixture tests | Deployment artifacts, operator settings, hardware verification |

Raw `CaptureFormat::Yuy2` is distinct from delivery/storage `VideoCodec::H264` or `Mjpeg`. Proposed names describe responsibilities; final API names follow existing crate conventions. SDK types and OT-2 IDs must not leak into public generic contracts.

```text
Webcam/external MJPEG -> select fresh output frame -> image JPEG decode -> yuv I420 -> OpenH264
Webcam/external YUY2  -> select fresh output frame -> yuv I420 --------------------> OpenH264

Orbbec shared frame set:
  native YUYV -> pass through as YUY2
  MJPEG      -> image decode -> yuv -> YUY2
  RGB/BGR    -> yuv planar422 -> packed YUY2
  normalized color YUY2 -> yuv I420 -> OpenH264
  depth / left-right IR / confidence remain their numeric channel formats

Original JPEG, when present -> existing JPEG consumers unchanged
Raw color without JPEG     -> devkit lazy JPEG derivative -> existing JPEG consumers
Complete H.264 picture     -> app immutable DVR epoch -> shared HLS/CMAF/finite MP4
```

Orbbec color explicitly becomes YUY2 before entering the generic video path. Retain original camera JPEG alongside the normalized frame where available; do not encode it again for JPEG consumers. This conversion does not imply that the physical camera can send HD YUYV over USB2. Native YUYV is preferred only when it satisfies the requested resolution/rate; otherwise retain HD MJPEG capture and normalize in memory. Perform expensive work only for selected output frames or actual synchronized consumers, without rebuilding the shared session or converting every unused 30/60 FPS frame.

## Behavioral invariants

- Default output is 5 FPS; sensor FPS is negotiated independently. Prefer an advertised native 5 FPS mode, otherwise sample a suitable advertised higher rate by monotonic time. No sleeping consumer is allowed to accumulate stale frames.
- Preserve requested resolution. Explicit unsupported raw mode returns actual available modes; auto retains same-resolution MJPEG rather than silently choosing VGA YUY2.
- YUY2/YUYV packed order is `Y0 U0 Y1 V0`. Validate even width, supported encoder height/alignment, stride, bounds, and overflow. Preserve matrix/range metadata; use an explicit documented fallback with provenance if a backend lacks it. Match I420 range/matrix to encoder VUI.
- All representations retain source/frame identity, original UTC capture time, and device timestamps where available. Never replace capture time with decode or encode time. Preserve Orbbec calibration, device frame numbers, and synchronized association.
- Reuse one capture session per device and one existing shared Orbbec pipeline. Depth/IR/confidence retain their bit depth, units, bytes, and timing.
- Retain all-IDR/no-B-frame behavior for the first migration. Verify actual IDR NALs and independent decode; current CMAF sample flags and 8x playback rely on it. Longer GOPs are a separate optimization.
- SPS/PPS and declared frame properties are immutable per DVR epoch. Old x264 history must not be rebound to new OpenH264 initialization. No destructive DVR migration or invented recovery for already-lost initialization.
- Keep existing source IDs, JPEG consumers, metadata-only protocol responses, media URL security, retention windows, bounded queues, and honest gap reporting. Raw YUY2 capture is not a request to archive uncompressed video.

## Delivery gates and cross-repository order

Local CLI dependencies point only to lower-numbered tasks in their own repository. Cross-repository dependencies are explicit gates because Backlog IDs are repository-scoped; a bare task number from another repo is never a CLI dependency. A gate is satisfied by reviewed delivered code and passing checks, not by a document or task status alone. Before starting a downstream task, verify its required APIs in the pinned devkit revision.

| Gate | Devkit tasks that must be delivered | OT-2 consumers |
| --- | --- | --- |
| Capture gate | TASK-109, TASK-110, TASK-115 | TASK-54, TASK-55, TASK-56 |
| Pixel gate | TASK-111, TASK-112, TASK-113 | TASK-57, TASK-58 |
| Epoch contract gate | TASK-119 | TASK-59 |
| Packaging gate | TASK-120, TASK-121 | TASK-60 |
| Encoding gate | TASK-114, TASK-116, TASK-117, TASK-118 | TASK-61 |
| Media fixture gate | TASK-122 | TASK-63 |
| Portable library gate | TASK-123 | TASK-64 |
| Performance harness gate | TASK-124 | TASK-65 |

Start devkit contracts/pixel work and OT-2 baseline reconciliation independently. Each downstream app task may advance the devkit revision and lockfile to a reviewed commit containing its newly required helpers; never use a floating branch or assume an earlier capture-only pin contains the encoder. Complete the app epoch writer/provider before enabling the replacement encoder. Remove FFmpeg after the shared path works. Release-artifact and opt-in camera checks follow fixture parity. Operators decide separately when to deploy; this planning work does not change a running host.

## Task size and implementation handoff

Each row is one focused PR, with its own scope, observable acceptance criteria, and lower-numbered local prerequisites. Start with the task, this plan, the named source files, and focused tests; read additional files only for the boundary being changed. No task requires the entire prior research conversation or all transport implementations in context. Keep a typical change to one module/seam plus its tests; if a task reveals unrelated migrations or missing upstream work, record and split that work rather than absorbing it silently.

Do not populate implementation-plan fields before starting. The tables below are the delivery backlog; detailed implementation plans are added to individual tasks after moving them to In Progress. A dependency introduced by later findings may require splitting/reordering tasks so earlier tasks never reference higher-numbered tasks.

## Devkit reusable-media backlog

| Task | Focus | Local prerequisites |
| --- | --- | --- |
| [TASK-109](../../../tasks/task-109%20-%20Define-backend-neutral-captured-color-frames.md) | Define backend-neutral captured color frames | None |
| [TASK-110](../../../tasks/task-110%20-%20Select-capture-format-and-sensor-rate-from-advertised-profiles.md) | Select capture format and sensor rate from advertised profiles | TASK-109 |
| [TASK-111](../../../tasks/task-111%20-%20Convert-packed-YUY2-and-RGB-color-with-the-yuv-crate.md) | Convert packed YUY2 and RGB color with the yuv crate | TASK-109 |
| [TASK-112](../../../tasks/task-112%20-%20Decode-bounded-MJPEG-frames-into-reusable-color-buffers.md) | Decode bounded MJPEG frames into reusable color buffers | TASK-111 |
| [TASK-113](../../../tasks/task-113%20-%20Produce-JPEG-derivatives-from-raw-color-frames.md) | Produce JPEG derivatives from raw color frames | TASK-112 |
| [TASK-114](../../../tasks/task-114%20-%20Select-fresh-output-frames-at-a-bounded-target-rate.md) | Select fresh output frames at a bounded target rate | TASK-109 |
| [TASK-115](../../../tasks/task-115%20-%20Add-an-optional-Nokhwa-adapter-for-undecoded-webcam-frames.md) | Add an optional Nokhwa adapter for undecoded webcam frames | TASK-110 |
| [TASK-116](../../../tasks/task-116%20-%20Encode-I420-pictures-through-bundled-OpenH264.md) | Encode I420 pictures through bundled OpenH264 | TASK-111 |
| [TASK-117](../../../tasks/task-117%20-%20Adapt-OpenH264-pictures-to-existing-DVR-media-samples.md) | Adapt OpenH264 pictures to existing DVR media samples | TASK-116 |
| [TASK-118](../../../tasks/task-118%20-%20Compose-a-reusable-dual-format-color-encoding-pipeline.md) | Compose a reusable dual-format color encoding pipeline | TASK-113, TASK-114, TASK-117 |
| [TASK-119](../../../tasks/task-119%20-%20Define-immutable-H.264-configuration-epochs-for-DVR-stores.md) | Define immutable H.264 configuration epochs for DVR stores | TASK-109 |
| [TASK-120](../../../tasks/task-120%20-%20Package-HLS-across-sequential-H.264-configuration-epochs.md) | Package HLS across sequential H.264 configuration epochs | TASK-119 |
| [TASK-121](../../../tasks/task-121%20-%20Preserve-decoder-configuration-in-finite-H.264-exports.md) | Preserve decoder configuration in finite H.264 exports | TASK-120 |
| [TASK-122](../../../tasks/task-122%20-%20Validate-real-dual-format-media-through-DVR-packaging.md) | Validate real dual-format media through DVR packaging | TASK-118, TASK-121 |
| [TASK-123](../../../tasks/task-123%20-%20Verify-media-feature-isolation-and-portable-codec-builds.md) | Verify media feature isolation and portable codec builds | TASK-122, TASK-115 |
| [TASK-124](../../../tasks/task-124%20-%20Measure-dual-format-conversion-and-encoding-at-5-FPS.md) | Measure dual-format conversion and encoding at 5 FPS | TASK-118 |

## OT-2 integration task map

These IDs belong to the companion repository and are delivery references, not local CLI dependencies. See the companion plan for individual task links and local dependencies.

| Task | Focus |
| --- | --- |
| TASK-53 | Reconcile the recorded shared Orbbec source baseline |
| TASK-54 | Expose dual-format capture preferences through devkit configuration |
| TASK-55 | Route webcam and external capture through shared raw-frame adapters |
| TASK-56 | Accept advertised raw and MJPEG color profiles in the Orbbec session |
| TASK-57 | Normalize every supported Orbbec color frame to YUY2 |
| TASK-58 | Keep JPEG image consumers working with raw capture |
| TASK-59 | Store H.264 initialization as immutable DVR epochs |
| TASK-60 | Select retained H.264 epochs by requested time range |
| TASK-61 | Use the shared H.264 pipeline for every local color source |
| TASK-62 | Remove the superseded FFmpeg encoder subprocess path |
| TASK-63 | Verify dual-format media parity through current OT-2 contracts |
| TASK-64 | Verify FFmpeg-free encoding in OT-2 release artifacts |
| TASK-65 | Verify YUY2 and MJPEG modes on chupacabra at 5 FPS |
| TASK-66 | Document dual-format capture and FFmpeg-free operation |

## Verification and rollout criteria

Use deterministic hardware-free tests for most tasks. Conversion fixtures check channel order, known color values/tolerances, padded rows, malformed/overflowing buffers, dimensions, and bounded JPEG decoding. Encoder fixtures must actually decode media, including first/final pictures and all-IDR trick-play subsets. A metadata-only mock is not enough to validate a new codec.

The fixture path covers JPEG stills/preview, HLS live/finite playback, timeline and seek, 0.5x through 8x playback, bounded MP4, reconnect, old/new DVR epochs, and metadata-only A2A/MCP plus enabled SiLA exports. Reuse existing security and transport tests. Historical arbitrary H.264-to-JPEG extraction is currently unsupported and is not silently claimed fixed by capture work.

Finite exports spanning same-dimension configuration changes must use the correct decoder configuration for every sample. Incompatible transitions, including unsupported resolution changes, must return a structured error before publication; do not publish corrupt MP4, silently truncate, or transcode history. If complete cross-resolution clip support is later required, scope it separately.

Target checks cover the current OT-2 release matrix: x86_64/aarch64 Linux GNU and musl, and aarch64 Apple Darwin. Both musl outputs must remain fully static. Verify runtime dependencies and decode a generated sample in the minimal container. Add separate library fixture coverage for Windows x86_64; this does not assert a Windows application release exists. Feature-disabled and browser WASM graphs must omit OpenH264 and native camera libraries.

Performance verification is future work, with an opt-in fixture runner and camera measurements. A 5 FPS target supplies 200 ms per output frame. Record p50/p95 processing latency, actual output/drop rate, warmup, source profile, resolution, memory, target, and hardware. Include normal MJPEG-to-I420 and the required Orbbec MJPEG-to-YUY2-to-I420 path; JPEG derivative cost is separate. An unmeasured platform is not a performance pass.

The recorded chupacabra inventory showed USB2 links and HD MJPEG profiles while raw YUYV was restricted to smaller modes on some cameras. Treat those observations as fixtures and repeat capability discovery during the opt-in hardware task; do not hard-code node numbers or assume the cabling/firmware has stayed unchanged. Test EMEET/HP HD MJPEG and available raw modes, plus Orbbec native raw and HD MJPEG normalization, concurrent consumers, single pipeline, synchronized channels, reconnect, recovery, and a bounded dual-source soak.

Every implementation task remains To Do until started. Its acceptance criteria, focused checks, implementation notes, final summary, and owning repository `mise run quality` are required before Done. Register new focused commands in `.mise/run.toml`; do not guess unsupported raw commands. Dependency/tool installation still follows repository approval rules. No dependencies, prototypes, host configuration, or deployments were performed while creating this plan.

## Boundaries

This delivery does not add RealSense/ESP32 drivers, replace the browser player, add AV1/H.265/hardware encoding, adopt longer GOPs, or rewrite existing synchronized frame-set storage. Their future adapters can reuse the color boundary. General remuxer replacement and arbitrary historical H.264-to-JPEG extraction need separate requirements.
