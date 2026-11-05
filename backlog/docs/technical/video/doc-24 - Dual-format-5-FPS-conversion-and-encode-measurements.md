---
id: doc-24
title: Dual-format 5 FPS conversion and encode measurements
type: technical
audience: technical
created_date: '2026-11-05 19:38'
---

# Dual-format 5 FPS conversion and encode measurements

Measured with `mise run media-bench` on 2026-11-05. That task runs `cargo run --release --example media_bench --features yuv,openh264`. It is not part of `mise run quality` and it does not fail the process when a sample is over the 200 ms budget.

Host: aarch64-macos Apple M5 Pro. Warmup: 2. Samples: 12. The fixture is a flat gray JPEG or packed YUY2 frame, not a detailed camera scene. Linux and Windows were not run.

Times are milliseconds. Peak bytes are the largest intermediate buffer retained for that case. The 200 ms column is the p95 verdict.

| Resolution | Path | Stage | p50 ms | p95 ms | Peak bytes | 200 ms |
| --- | --- | --- | --- | --- | --- | --- |
| 640x480 | MJPEG to I420 | decode | 0.260 | 0.465 | 460800 | pass |
| 640x480 | MJPEG to I420 | encode | 0.831 | 0.935 | 460800 | pass |
| 640x480 | MJPEG to I420 | copy | 0.007 | 0.011 | 460800 | pass |
| 640x480 | MJPEG to I420 | total | 1.099 | 1.327 | 460800 | pass |
| 640x480 | MJPEG to YUY2 to I420 | decode | 0.318 | 0.558 | 1075200 | pass |
| 640x480 | MJPEG to YUY2 to I420 | convert | 0.028 | 0.059 | 1075200 | pass |
| 640x480 | MJPEG to YUY2 to I420 | encode | 0.943 | 1.070 | 1075200 | pass |
| 640x480 | MJPEG to YUY2 to I420 | total | 1.304 | 1.628 | 1075200 | pass |
| 640x480 | JPEG derivative | derivative | 1.367 | 1.571 | 9027 | pass |
| 1280x720 | MJPEG to I420 | decode | 0.758 | 0.876 | 1382400 | pass |
| 1280x720 | MJPEG to I420 | encode | 2.630 | 2.897 | 1382400 | pass |
| 1280x720 | MJPEG to I420 | copy | 0.019 | 0.020 | 1382400 | pass |
| 1280x720 | MJPEG to I420 | total | 3.417 | 3.620 | 1382400 | pass |
| 1280x720 | MJPEG to YUY2 to I420 | decode | 0.840 | 0.896 | 3225600 | pass |
| 1280x720 | MJPEG to YUY2 to I420 | convert | 0.069 | 0.081 | 3225600 | pass |
| 1280x720 | MJPEG to YUY2 to I420 | encode | 2.688 | 2.905 | 3225600 | pass |
| 1280x720 | MJPEG to YUY2 to I420 | total | 3.572 | 3.826 | 3225600 | pass |
| 1280x720 | JPEG derivative | derivative | 3.964 | 4.163 | 25827 | pass |
| 1280x800 | MJPEG to I420 | decode | 0.837 | 0.906 | 1536000 | pass |
| 1280x800 | MJPEG to I420 | encode | 2.874 | 3.229 | 1536000 | pass |
| 1280x800 | MJPEG to I420 | copy | 0.022 | 0.028 | 1536000 | pass |
| 1280x800 | MJPEG to I420 | total | 3.750 | 4.038 | 1536000 | pass |
| 1280x800 | MJPEG to YUY2 to I420 | decode | 0.904 | 1.067 | 3584000 | pass |
| 1280x800 | MJPEG to YUY2 to I420 | convert | 0.077 | 0.106 | 3584000 | pass |
| 1280x800 | MJPEG to YUY2 to I420 | encode | 2.878 | 3.239 | 3584000 | pass |
| 1280x800 | MJPEG to YUY2 to I420 | total | 3.872 | 4.221 | 3584000 | pass |
| 1280x800 | JPEG derivative | derivative | 4.390 | 4.539 | 28627 | pass |
| 1920x1080 | MJPEG to I420 | decode | 1.699 | 2.246 | 3110400 | pass |
| 1920x1080 | MJPEG to I420 | encode | 6.054 | 6.688 | 3110400 | pass |
| 1920x1080 | MJPEG to I420 | copy | 0.045 | 0.068 | 3110400 | pass |
| 1920x1080 | MJPEG to I420 | total | 7.748 | 8.933 | 3110400 | pass |
| 1920x1080 | MJPEG to YUY2 to I420 | decode | 1.843 | 2.132 | 7257600 | pass |
| 1920x1080 | MJPEG to YUY2 to I420 | convert | 0.144 | 0.172 | 7257600 | pass |
| 1920x1080 | MJPEG to YUY2 to I420 | encode | 5.951 | 6.542 | 7257600 | pass |
| 1920x1080 | MJPEG to YUY2 to I420 | total | 7.946 | 8.755 | 7257600 | pass |
| 1920x1080 | JPEG derivative | derivative | 8.730 | 8.905 | 57327 | pass |

On every H.264 path, encode is larger than JPEG decode, YUY2 conversion, and plane copy. Copy is the smallest stage. The separate JPEG derivative is slower than encode at the same resolution, and it is the slowest single stage in this run (8.730 ms p50 at 1920x1080). Nothing in this table is a reason to replace the `image` JPEG decoder or the `yuv` converter. No case was left unmeasured among the four resolutions, and every measured p95 is under 200 ms.

Related: task-124 and doc-23.
