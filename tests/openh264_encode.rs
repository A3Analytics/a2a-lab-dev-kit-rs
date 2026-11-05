//! Real `OpenH264` encode and decode of the first and last pictures.

#![cfg(feature = "openh264")]

use a2a_lab_dev_kit::VideoSourceId;
use a2a_lab_dev_kit::videos::{
    AllIdrEncoder, CaptureIdentity, ColorMatrix, ColorProperties, ColorRange, I420Picture,
    OPENH264_NOTICE, decode_luma,
};

fn gray(value: u8) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (vec![value; 16 * 16], vec![128; 8 * 8], vec![128; 8 * 8])
}

fn color() -> ColorProperties {
    ColorProperties::new(ColorMatrix::Bt601, ColorRange::Limited)
}

fn identity(sequence: u64) -> CaptureIdentity {
    CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence)
}

fn mean(samples: &[u8]) -> f32 {
    let sum: u32 = samples.iter().map(|sample| u32::from(*sample)).sum();
    #[allow(clippy::cast_precision_loss)]
    {
        sum as f32 / samples.len() as f32
    }
}

#[test]
fn all_idr_encoder_roundtrips_first_and_last_picture_at_30fps() {
    let mut encoder = AllIdrEncoder::new(16, 16, 30.0, 2_000_000, color()).expect("encoder");
    assert!((encoder.fps() - 30.0).abs() < f32::EPSILON);
    assert!(OPENH264_NOTICE.contains("0.9.9"));
    assert!(OPENH264_NOTICE.contains("BSD-2-Clause"));
    let (y0, u0, v0) = gray(40);
    let (y1, u1, v1) = gray(200);
    let first = encoder
        .encode(
            I420Picture::new(16, 16, &y0, &u0, &v0, color()),
            &identity(1),
            1_000,
        )
        .expect("first");
    let first_owned = first.bytes().to_vec();
    let last = encoder
        .encode(
            I420Picture::new(16, 16, &y1, &u1, &v1, color()),
            &identity(2),
            2_000,
        )
        .expect("last");
    assert_eq!(first.bytes(), first_owned.as_slice());
    assert_ne!(first.bytes(), last.bytes());
    assert!(first.keyframe() && last.keyframe());
    assert_eq!(first.identity().sequence(), 1);
    assert_eq!(last.timestamp_ns(), 2_000);
    assert!(!first.initialization().is_empty());
    let decoded_first = decode_luma(first.bytes()).expect("decode first");
    let decoded_last = decode_luma(last.bytes()).expect("decode last");
    assert!((mean(&decoded_first) - 40.0).abs() < 20.0);
    assert!((mean(&decoded_last) - 200.0).abs() < 20.0);
    assert!(mean(&decoded_last) > mean(&decoded_first) + 50.0);

    let (y_mismatch, u_mismatch, v_mismatch) = gray(10);
    assert!(
        encoder
            .encode(
                I420Picture::new(
                    8,
                    8,
                    &y_mismatch[..64],
                    &u_mismatch[..16],
                    &v_mismatch[..16],
                    color()
                ),
                &identity(3),
                3_000,
            )
            .is_err()
    );
    encoder.shutdown();
    assert!(
        encoder
            .encode(
                I420Picture::new(16, 16, &y0, &u0, &v0, color()),
                &identity(4),
                4_000,
            )
            .is_err()
    );
}

#[test]
fn encoder_rejects_unknown_color_and_odd_dimensions() {
    let unknown = ColorProperties::new(ColorMatrix::Unknown, ColorRange::Full);
    assert!(AllIdrEncoder::new(16, 16, 5.0, 1_000_000, unknown).is_err());
    assert!(AllIdrEncoder::new(15, 16, 5.0, 1_000_000, color()).is_err());
    assert!(AllIdrEncoder::new(16, 16, 0.0, 1_000_000, color()).is_err());
}
