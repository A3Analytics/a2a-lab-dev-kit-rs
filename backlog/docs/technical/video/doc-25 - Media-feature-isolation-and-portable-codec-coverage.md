---
id: doc-25
title: Media feature isolation and portable codec coverage
type: technical
audience: technical
created_date: '2026-11-05 20:06'
---

# Media feature isolation and portable codec coverage

`mise run media-features` ran on 2026-11-05. Host: Darwin arm64. Rust: 1.98.1 (48a229cea 2026-09-01). Related: task-123, task-115, task-122, and doc-23.

## Feature graphs

OpenH264 and Nokhwa were absent from the default graph and from `--no-default-features`. They were also absent for `wasm32-unknown-unknown`, which is installed on this machine. `yuv` was present with `--features yuv`. OpenH264 was present with `--features openh264`. Nokhwa was present with `--features nokhwa`. None of those three features is in the default feature set. Application code does not select them by operating system.

The ordinary `mise run quality` web-ui feature-off check still covers the browser package. This media task does not add those crates to that graph.

## What ran

The same command then ran `cargo test --test media_validate --features yuv,openh264`. Both fixture tests passed on this Apple aarch64 host. They decode generated MJPEG and YUY2 through the bundled OpenH264 decoder. No camera and no FFmpeg.

OpenH264 0.9.9 is pinned with the `source` feature, so the C++ codec is built with the crate. This host already had a C++ toolchain; the check did not download a runtime codec. Licenses for the pinned crates: OpenH264 BSD-2-Clause, `yuv` 0.8.20 BSD-3-Clause OR Apache-2.0, `image` 0.25.10 MIT OR Apache-2.0, Nokhwa 0.10.11 Apache-2.0. `image` is pulled only by the `yuv` feature, with default features off and `jpeg` on.

## Extra-target builds

Linux GNU/musl and Windows x86_64 builds are skipped for this task. They are not compile-only coverage and they are not a runtime pass. The `media-features` script names those targets as skipped extra-target builds. No Windows OT-2 release is claimed. The fixture has no operating-system gate; it ran on Darwin arm64.

Pinned toolchain: Rust 1.98.1. OpenH264 is built from bundled C++ source on this host, which already had a C++ toolchain. There is no downloaded runtime codec. Application code does not select these crates by operating system. If an extra-target toolchain is later required, that is a later check, not this task.
