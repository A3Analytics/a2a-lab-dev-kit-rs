//! JPEG derivatives from raw YUY2 and original camera JPEGs.

#![cfg(feature = "yuv")]

use a2a_lab_dev_kit::videos::{
    CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix, ColorProperties,
    ColorRange, JpegDerivativeCache, JpegDerivativeLimits, jpeg_derivative,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};

fn identity(sequence: u64) -> CaptureIdentity {
    CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence)
}

fn timing() -> CaptureTiming {
    CaptureTiming::new(
        UtcTimestamp::parse("2020-01-01T00:00:00Z").expect("time"),
        CaptureClock::monotonic(4),
    )
}

fn color() -> ColorProperties {
    ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full)
}

fn yuy2(sequence: u64, packed: &[u8], jpeg: Option<&[u8]>) -> CapturedColorFrame {
    CapturedColorFrame::yuy2(identity(sequence), timing(), 2, 2, 4, packed, color(), jpeg)
        .expect("frame")
}

fn neutral() -> Vec<u8> {
    vec![128, 128, 128, 128, 128, 128, 128, 128]
}

#[test]
fn raw_yuy2_encodes_a_same_size_jpeg_and_keeps_an_original() {
    let limits = JpegDerivativeLimits::new(64 * 1024).expect("limits");
    let encoded = jpeg_derivative(&yuy2(1, &neutral(), None), 80, limits).expect("encode");
    assert!(!encoded.original());
    assert_eq!(encoded.width(), 2);
    assert_eq!(encoded.height(), 2);
    assert_eq!(encoded.identity().sequence(), 1);
    assert!(encoded.jpeg().starts_with(&[0xFF, 0xD8]));
    let decoded = image::ImageReader::new(std::io::Cursor::new(encoded.jpeg()))
        .with_guessed_format()
        .expect("guess")
        .decode()
        .expect("decode");
    assert_eq!(decoded.width(), 2);
    assert_eq!(decoded.height(), 2);

    let original = [0xFF, 0xD8, 0xFF, 0xD9];
    let kept =
        jpeg_derivative(&yuy2(2, &neutral(), Some(&original)), 20, limits).expect("original");
    assert!(kept.original());
    assert_eq!(kept.jpeg(), &original);
}

#[test]
fn cache_encodes_only_requested_identities() {
    let limits = JpegDerivativeLimits::new(64 * 1024).expect("limits");
    let first = yuy2(1, &neutral(), None);
    let second = yuy2(2, &neutral(), None);
    let mut cache = JpegDerivativeCache::new();
    assert!(cache.is_empty());
    let once = cache
        .get_or_create(&first, 70, limits)
        .expect("first")
        .jpeg()
        .to_vec();
    let again = cache
        .get_or_create(&first, 70, limits)
        .expect("repeat")
        .jpeg()
        .to_vec();
    assert_eq!(once, again);
    assert_eq!(cache.len(), 1);
    cache.get_or_create(&second, 70, limits).expect("second");
    assert_eq!(cache.len(), 2);
    let _unused = yuy2(3, &neutral(), None);
    assert_eq!(cache.len(), 2);
}

#[test]
fn invalid_quality_and_output_limit_fail() {
    let frame = yuy2(1, &neutral(), None);
    let limits = JpegDerivativeLimits::new(64 * 1024).expect("limits");
    assert!(jpeg_derivative(&frame, 0, limits).is_err());
    let unknown = CapturedColorFrame::yuy2(
        identity(9),
        timing(),
        2,
        2,
        4,
        &neutral(),
        ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown),
        None,
    )
    .expect("frame");
    assert!(jpeg_derivative(&unknown, 80, limits).is_err());
    let tight = JpegDerivativeLimits::new(8).expect("tight");
    assert!(jpeg_derivative(&frame, 90, tight).is_err());
}
