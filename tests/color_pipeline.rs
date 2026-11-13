//! Injected MJPEG and YUY2 frames share one H.264 path. No camera.

#![cfg(all(feature = "yuv", feature = "openh264"))]

use std::io::Write;
use std::sync::{Arc, Mutex};

use a2a_lab_dev_kit::videos::{
    CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorEncodePipeline,
    ColorMatrix, ColorProperties, ColorRange, OUTPUT_5FPS_PERIOD_NS, decode_luma,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgb};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};

fn color(matrix: ColorMatrix, range: ColorRange) -> ColorProperties {
    ColorProperties::new(matrix, range)
}

fn yuy2_frame(
    sequence: u64,
    time_ns: u64,
    y: u8,
    properties: ColorProperties,
) -> CapturedColorFrame {
    let mut packed = Vec::with_capacity(16 * 2 * 16);
    for _ in 0..(16 * 16 / 2) {
        packed.extend_from_slice(&[y, 128, y, 128]);
    }
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").expect("time"),
            CaptureClock::monotonic(time_ns),
        ),
        16,
        16,
        32,
        &packed,
        properties,
        None,
    )
    .expect("yuy2")
}

fn jpeg_frame(sequence: u64, time_ns: u64, y: u8) -> CapturedColorFrame {
    let image: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(16, 16, vec![y; 16 * 16 * 3]).expect("image");
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 80)
        .write_image(image.as_raw(), 16, 16, ExtendedColorType::Rgb8)
        .expect("jpeg");
    CapturedColorFrame::mjpeg(
        CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").expect("time"),
            CaptureClock::monotonic(time_ns),
        ),
        16,
        16,
        &jpeg,
        color(ColorMatrix::Bt601, ColorRange::Full),
    )
    .expect("mjpeg")
}

fn bad_jpeg(sequence: u64, time_ns: u64) -> CapturedColorFrame {
    CapturedColorFrame::mjpeg(
        CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").expect("time"),
            CaptureClock::monotonic(time_ns),
        ),
        16,
        16,
        &[0x01, 0x02, 0x03],
        color(ColorMatrix::Bt601, ColorRange::Limited),
    )
    .expect("bad jpeg")
}

fn annex_b(initialization: &[u8], sample: &[u8]) -> Vec<u8> {
    let mut annex_b = initialization.to_vec();
    let mut index = 0_usize;
    while index + 4 <= sample.len() {
        let length = usize::try_from(u32::from_be_bytes([
            sample[index],
            sample[index + 1],
            sample[index + 2],
            sample[index + 3],
        ]))
        .expect("length");
        index += 4;
        annex_b.extend_from_slice(&[0, 0, 0, 1]);
        annex_b.extend_from_slice(&sample[index..index + length]);
        index += length;
    }
    annex_b
}

#[derive(Clone, Default)]
struct WarnCapture(Arc<Mutex<Vec<u8>>>);

impl Write for WarnCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn install_warn_capture() -> (WarnCapture, tracing::Dispatch) {
    let capture = WarnCapture::default();
    let writer = capture.clone();
    let filter = EnvFilter::try_new("a2a_lab_dev_kit::videos::color_encode=warn").expect("filter");
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_filter(filter)
        .boxed();
    let subscriber = tracing_subscriber::registry().with(layer);
    (capture, tracing::Dispatch::new(subscriber))
}

fn warn_text(capture: &WarnCapture) -> String {
    String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf8")
}

fn mean(samples: &[u8]) -> f32 {
    let sum: u32 = samples.iter().map(|sample| u32::from(*sample)).sum();
    #[allow(clippy::cast_precision_loss)]
    {
        sum as f32 / samples.len() as f32
    }
}

#[test]
fn mjpeg_and_normalized_yuy2_encode_on_the_same_path() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    for frame in [yuy2_frame(1, 0, 40, limited), jpeg_frame(1, 0, 40)] {
        let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, Some(80)).expect("pipeline");
        pipeline.push(frame).expect("push");
        let encoded = pipeline.pull().expect("sample");
        assert!(encoded.keyframe());
        assert_eq!(encoded.original().identity().sequence(), 1);
        assert_eq!(encoded.original().timing().clock().monotonic_ns(), Some(0));
        assert!(encoded.jpeg().is_some());
        let luma =
            decode_luma(&annex_b(encoded.initialization(), encoded.sample())).expect("decode");
        assert!((mean(&luma) - 40.0).abs() < 25.0);
    }
}

#[test]
fn thirty_fps_input_converts_only_selected_frames() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 8, None).expect("pipeline");
    let step = 1_000_000_000 / 30;
    for index in 0..31 {
        pipeline
            .push(yuy2_frame(index, index * step, 40, limited))
            .expect("push");
    }
    assert_eq!(pipeline.counters().converted(), 5);
    assert_eq!(pipeline.counters().encoded(), 5);
    assert!(pipeline.counters().dropped() > 0);
    assert!(pipeline.queued() <= 8);
    let mut count = 0_u32;
    while pipeline.pull().is_some() {
        count += 1;
    }
    assert_eq!(count, 5);
}

#[test]
fn a_full_queue_stays_bounded_for_a_slow_consumer() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 1, None).expect("pipeline");
    for index in 0..3 {
        pipeline
            .push(yuy2_frame(
                index,
                index * OUTPUT_5FPS_PERIOD_NS,
                40,
                limited,
            ))
            .expect("push");
    }
    assert_eq!(pipeline.queued(), 1);
    assert!(pipeline.counters().dropped() > 0);
    assert_eq!(pipeline.counters().converted(), 3);
}

#[test]
fn loss_fault_reconnect_and_cancel_are_counted() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let full = color(ColorMatrix::Bt709, ColorRange::Full);
    let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, None).expect("pipeline");
    pipeline.push(yuy2_frame(1, 0, 40, limited)).expect("first");
    pipeline.source_lost();
    pipeline
        .push(yuy2_frame(2, OUTPUT_5FPS_PERIOD_NS, 40, limited))
        .expect("lost");
    assert_eq!(pipeline.counters().source_lost(), 1);
    assert_eq!(pipeline.counters().converted(), 1);
    pipeline.reconnect();
    pipeline.fault_encoder();
    pipeline
        .push(yuy2_frame(3, OUTPUT_5FPS_PERIOD_NS, 80, limited))
        .expect("fault");
    assert_eq!(pipeline.counters().encoder_failures(), 1);
    pipeline.reconnect();
    pipeline
        .push(yuy2_frame(4, OUTPUT_5FPS_PERIOD_NS * 2, 200, full))
        .expect("recovered");
    assert!(pipeline.counters().reconnects() >= 2);
    assert_eq!(pipeline.counters().encoded(), 2);
    pipeline
        .push(bad_jpeg(9, OUTPUT_5FPS_PERIOD_NS * 3))
        .expect("malformed");
    assert_eq!(pipeline.counters().malformed(), 1);
    let encoded_before_cancel = pipeline.counters().encoded();
    pipeline.cancel();
    pipeline
        .push(yuy2_frame(6, OUTPUT_5FPS_PERIOD_NS * 4, 40, limited))
        .expect("cancelled");
    assert!(pipeline.counters().cancelled() >= 1);
    assert_eq!(pipeline.counters().encoded(), encoded_before_cancel);
}

#[test]
fn injected_slow_encode_emits_structured_warning() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let (capture, dispatch) = install_warn_capture();
    tracing::dispatcher::with_default(&dispatch, || {
        let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, None).expect("pipeline");
        pipeline.inject_encode_elapsed_ns(OUTPUT_5FPS_PERIOD_NS);
        pipeline
            .encode_frame(yuy2_frame(7, 42, 40, limited))
            .expect("encode");
    });
    let text = warn_text(&capture);
    assert!(text.contains("slow color encode may miss live output cadence"));
    assert!(text.contains(&format!("\"elapsed_ns\":{OUTPUT_5FPS_PERIOD_NS}")));
    assert!(text.contains(&format!("\"threshold_ns\":{OUTPUT_5FPS_PERIOD_NS}")));
    assert!(text.contains("\"sequence\":7"));
    assert!(text.contains("\"monotonic_ns\":42"));
}

#[test]
fn fast_injected_encode_does_not_emit_slow_warning() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let (capture, dispatch) = install_warn_capture();
    tracing::dispatcher::with_default(&dispatch, || {
        let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, None).expect("pipeline");
        pipeline.inject_encode_elapsed_ns(OUTPUT_5FPS_PERIOD_NS - 1);
        pipeline
            .encode_frame(yuy2_frame(8, 84, 40, limited))
            .expect("encode");
    });
    assert!(warn_text(&capture).is_empty());
}

#[test]
fn slow_encode_threshold_boundary_is_deterministic() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let (capture, dispatch) = install_warn_capture();
    let threshold = 50_000_000_u64;
    tracing::dispatcher::with_default(&dispatch, || {
        let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, None).expect("pipeline");
        pipeline.set_slow_encode_threshold_ns(threshold);
        pipeline.inject_encode_elapsed_ns(threshold.saturating_sub(1));
        pipeline
            .encode_frame(yuy2_frame(9, 126, 40, limited))
            .expect("under threshold");
        assert!(warn_text(&capture).is_empty());
        pipeline.inject_encode_elapsed_ns(threshold);
        pipeline
            .encode_frame(yuy2_frame(10, 168, 40, limited))
            .expect("at threshold");
    });
    let text = warn_text(&capture);
    assert!(text.contains(&format!("\"elapsed_ns\":{threshold}")));
    assert!(!text.contains("\"sequence\":9"));
    assert!(text.contains("\"sequence\":10"));
}

#[test]
fn a_color_change_opens_a_new_encoder_and_keeps_both_samples() {
    let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
    let full = color(ColorMatrix::Bt709, ColorRange::Full);
    let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 4, None).expect("pipeline");
    pipeline.push(yuy2_frame(1, 0, 40, limited)).expect("first");
    pipeline
        .push(yuy2_frame(2, OUTPUT_5FPS_PERIOD_NS, 200, full))
        .expect("second");
    assert_eq!(pipeline.counters().reconnects(), 1);
    let first = pipeline.pull().expect("first sample");
    let second = pipeline.pull().expect("second sample");
    assert_ne!(first.initialization(), second.initialization());
    assert_eq!(first.original().identity().sequence(), 1);
    assert_eq!(
        second.original().timing().clock().monotonic_ns(),
        Some(OUTPUT_5FPS_PERIOD_NS)
    );
    let dark = decode_luma(&annex_b(first.initialization(), first.sample())).expect("dark");
    let bright = decode_luma(&annex_b(second.initialization(), second.sample())).expect("bright");
    assert!(mean(&bright) > mean(&dark) + 50.0);
}
