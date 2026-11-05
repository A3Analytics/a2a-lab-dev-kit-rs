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

## What did not run

These targets were not executed. This document does not claim they build or decode:

- x86_64-unknown-linux-gnu
- aarch64-unknown-linux-gnu
- x86_64-unknown-linux-musl
- aarch64-unknown-linux-musl
- x86_64-pc-windows-msvc

No Windows release of the OT-2 application is claimed here. The fixture itself has no operating-system gate; it was only executed on Darwin arm64.
