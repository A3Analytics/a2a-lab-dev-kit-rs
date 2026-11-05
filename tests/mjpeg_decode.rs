//! Real JPEG decode of selected MJPEG frames.

#![cfg(feature = "yuv")]

use std::io::Cursor;

use a2a_lab_dev_kit::videos::{
    CaptureClock, CaptureFormat, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, DecodedPicture, MjpegDecodeLimits, MjpegDecodeTarget,
    decode_mjpeg,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;

fn identity() -> CaptureIdentity {
    CaptureIdentity::new(VideoSourceId::new("cam-1").expect("source"), 4)
}

fn timing() -> CaptureTiming {
    CaptureTiming::new(
        UtcTimestamp::parse("2020-01-01T00:00:01Z").expect("timestamp"),
        CaptureClock::identified("device", Some(99)).expect("clock"),
    )
}

fn encode(pixels: &[u8], width: u32, height: u32, color: ExtendedColorType) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut cursor, 90)
        .encode(pixels, width, height, color)
        .expect("encode jpeg");
    cursor.into_inner()
}

fn frame(jpeg: &[u8], width: u32, height: u32, color: ColorProperties) -> CapturedColorFrame {
    CapturedColorFrame::mjpeg(identity(), timing(), width, height, jpeg, color).expect("frame")
}

#[test]
fn color_jpeg_keeps_identity_and_decodes_at_original_size() {
    let jpeg = encode(
        &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        2,
        2,
        ExtendedColorType::Rgb8,
    );
    let captured = frame(
        &jpeg,
        2,
        2,
        ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown),
    );
    let decoded = decode_mjpeg(
        &captured,
        MjpegDecodeTarget::I420,
        MjpegDecodeLimits::standard(),
    )
    .expect("decode");
    assert_eq!(decoded.identity().sequence(), 4);
    assert_eq!(decoded.timing().captured_at(), timing().captured_at());
    assert_eq!(decoded.jpeg(), jpeg.as_slice());
    assert_eq!(decoded.width(), 2);
    assert_eq!(decoded.height(), 2);
    assert_eq!(decoded.properties().matrix(), ColorMatrix::Bt601);
    assert_eq!(decoded.properties().range(), ColorRange::Full);
    let DecodedPicture::I420(picture) = decoded.picture() else {
        panic!("expected I420");
    };
    assert_eq!(picture.width(), 2);
    assert_eq!(picture.y().len(), 4);
    assert!(picture.y().iter().any(|sample| *sample > 0));
}

#[test]
fn grayscale_jpeg_expands_and_packs_yuy2() {
    let jpeg = encode(&[10, 20, 30, 40], 2, 2, ExtendedColorType::L8);
    let captured = frame(
        &jpeg,
        2,
        2,
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
    );
    let decoded = decode_mjpeg(
        &captured,
        MjpegDecodeTarget::Yuy2,
        MjpegDecodeLimits::standard(),
    )
    .expect("decode");
    assert_eq!(decoded.properties().matrix(), ColorMatrix::Bt709);
    let DecodedPicture::Yuy2(packed) = decoded.picture() else {
        panic!("expected YUY2");
    };
    assert_eq!(packed.len(), 8);
    assert_eq!(captured.format(), CaptureFormat::Mjpeg);
}

#[test]
fn truncated_and_oversized_jpegs_are_rejected() {
    let jpeg = encode(
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        2,
        2,
        ExtendedColorType::Rgb8,
    );
    let truncated = frame(
        &jpeg[..jpeg.len() / 2],
        2,
        2,
        ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full),
    );
    assert!(
        decode_mjpeg(
            &truncated,
            MjpegDecodeTarget::I420,
            MjpegDecodeLimits::standard()
        )
        .is_err()
    );

    let abbreviated = frame(
        &[0xFF, 0xD8, 0xFF, 0xD9],
        2,
        2,
        ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full),
    );
    assert!(
        decode_mjpeg(
            &abbreviated,
            MjpegDecodeTarget::I420,
            MjpegDecodeLimits::standard()
        )
        .is_err()
    );

    let tight = MjpegDecodeLimits::new(8, 2).expect("limits");
    let captured = frame(
        &jpeg,
        2,
        2,
        ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full),
    );
    assert!(decode_mjpeg(&captured, MjpegDecodeTarget::I420, tight).is_err());

    let edge = MjpegDecodeLimits::new(MAX, 1).expect("edge");
    assert!(decode_mjpeg(&captured, MjpegDecodeTarget::Yuy2, edge).is_err());
}

const MAX: usize = 8 * 1024 * 1024;

#[test]
fn declared_dimensions_must_match_the_jpeg() {
    let jpeg = encode(
        &[9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9],
        2,
        2,
        ExtendedColorType::Rgb8,
    );
    let mismatch = frame(
        &jpeg,
        4,
        2,
        ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full),
    );
    assert!(
        decode_mjpeg(
            &mismatch,
            MjpegDecodeTarget::I420,
            MjpegDecodeLimits::standard()
        )
        .is_err()
    );
}
