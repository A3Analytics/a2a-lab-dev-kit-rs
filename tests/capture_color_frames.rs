//! Contract for owned MJPEG and packed YUY2 capture frames.

use a2a_lab_dev_kit::videos::{
    CaptureCapabilities, CaptureClock, CaptureFormat, CaptureIdentity, CaptureMode, CaptureTiming,
    CapturedColorFrame, ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange,
    MemoryColorCapture,
};
use a2a_lab_dev_kit::{A2aLabError, UtcTimestamp, VideoCodec, VideoSource, VideoSourceId};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];

fn timestamp() -> UtcTimestamp {
    UtcTimestamp::parse("2020-01-01T00:00:00Z").expect("timestamp")
}

fn source() -> VideoSourceId {
    VideoSourceId::new("cam-1").expect("source")
}

fn identity(sequence: u64) -> CaptureIdentity {
    CaptureIdentity::new(source(), sequence)
}

fn timing() -> CaptureTiming {
    CaptureTiming::new(
        timestamp(),
        CaptureClock::identified("device-clock", Some(12_345)).expect("clock"),
    )
}

fn unknown_color() -> ColorProperties {
    ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown)
}

fn yuy2_frame(
    packed: &[u8],
    stride: usize,
    jpeg: Option<&[u8]>,
) -> Result<CapturedColorFrame, A2aLabError> {
    CapturedColorFrame::yuy2(
        identity(7),
        timing(),
        4,
        2,
        stride,
        packed,
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Full),
        jpeg,
    )
}

fn invalid_field(error: A2aLabError) -> &'static str {
    match error {
        A2aLabError::Invalid { field, .. } => field,
        other => panic!("expected invalid input, got {other}"),
    }
}

#[test]
fn yuyv_is_the_packed_yuy2_layout() {
    assert_eq!(CaptureFormat::YUYV, CaptureFormat::Yuy2);
    let packed = CaptureMode::new(CaptureFormat::Yuy2, 640, 480).expect("yuy2");
    let alias = CaptureMode::new(CaptureFormat::YUYV, 640, 480).expect("yuyv");
    let error = CaptureCapabilities::new(vec![packed, alias]).expect_err("alias is one mode");
    assert_eq!(invalid_field(error), "capabilities");
}

#[test]
fn frame_exposes_validated_metadata_and_original_jpeg() {
    let mut packed = padded_yuy2();
    let mut jpeg = JPEG.to_vec();
    let frame = yuy2_frame(&packed, 10, Some(&jpeg)).expect("frame");
    assert_yuy2_metadata(&frame);

    let snapshot = frame.clone();
    packed.fill(0);
    jpeg.fill(0);
    assert_eq!(snapshot.bytes(), &padded_yuy2());
    assert_eq!(snapshot.original_jpeg(), Some(JPEG));
    assert_eq!(snapshot.timing().captured_at(), timestamp());
    assert_eq!(snapshot.timing().clock().monotonic_ns(), Some(12_345));
}

#[test]
fn mjpeg_frame_keeps_source_time_and_encoded_jpeg() {
    let mut jpeg = JPEG.to_vec();
    let frame = CapturedColorFrame::mjpeg(
        identity(3),
        CaptureTiming::new(timestamp(), CaptureClock::monotonic(99)),
        640,
        480,
        &jpeg,
        unknown_color(),
    )
    .expect("mjpeg");

    assert_eq!(frame.format(), CaptureFormat::Mjpeg);
    assert_eq!(frame.stride(), None);
    assert_eq!(frame.bytes(), JPEG);
    assert_eq!(frame.original_jpeg(), Some(JPEG));
    assert_eq!(frame.color().matrix(), ColorMatrix::Unknown);
    assert_eq!(frame.color().range(), ColorRange::Unknown);
    assert_eq!(frame.timing().clock().identity(), None);
    assert_eq!(frame.timing().clock().monotonic_ns(), Some(99));

    jpeg.fill(0);
    assert_eq!(frame.bytes(), JPEG);
    assert_eq!(frame.timing().captured_at(), timestamp());
}

#[test]
fn rejected_geometry_is_a_typed_error() {
    let packed = padded_yuy2();
    let cases = [
        (0, 2, 4, &packed[..], "width"),
        (4, 0, 10, &packed[..], "height"),
        (3, 2, 6, &packed[..], "width"),
        (4, 2, 7, &packed[..], "stride"),
        (4, 2, 10, &packed[..8], "bytes"),
        (4, 2, usize::MAX, &[][..], "stride"),
    ];
    for (width, height, stride, bytes, field) in cases {
        let error = CapturedColorFrame::yuy2(
            identity(1),
            timing(),
            width,
            height,
            stride,
            bytes,
            unknown_color(),
            None,
        )
        .expect_err("invalid geometry");
        assert_eq!(invalid_field(error), field);
    }
}

#[test]
fn overflow_stride_does_not_panic_or_allocate_the_product() {
    let error = CapturedColorFrame::yuy2(
        identity(1),
        timing(),
        2,
        2,
        usize::MAX,
        &[0, 1, 2, 3],
        unknown_color(),
        None,
    )
    .expect_err("overflow");
    match error {
        A2aLabError::Invalid { field, message } => {
            assert_eq!(field, "stride");
            assert!(message.contains("overflows"), "{message}");
        }
        other => panic!("expected invalid input, got {other}"),
    }
}

#[test]
fn row_padding_is_preserved_and_bytes_past_the_frame_are_not() {
    let mut packed = padded_yuy2();
    packed.push(0xAB);
    let frame = yuy2_frame(&packed, 10, None).expect("padded");
    assert_eq!(frame.bytes().len(), 20);
    assert_eq!(&frame.bytes()[8..10], &[0xA0, 0xA1]);
    assert_eq!(&frame.bytes()[18..20], &[0xB0, 0xB1]);
    assert!(!frame.bytes().contains(&0xAB));
}

#[test]
fn empty_jpeg_and_empty_clock_identity_are_rejected() {
    let jpeg = CapturedColorFrame::mjpeg(identity(1), timing(), 2, 2, &[], unknown_color())
        .expect_err("empty jpeg");
    assert_eq!(invalid_field(jpeg), "jpeg");

    let clock = CaptureClock::identified("", Some(1)).expect_err("empty clock");
    assert_eq!(invalid_field(clock), "clock");

    let alongside = yuy2_frame(&padded_yuy2(), 10, Some(&[])).expect_err("empty original");
    assert_eq!(invalid_field(alongside), "jpeg");
}

#[test]
fn capabilities_and_memory_source_represent_mjpeg_and_yuy2() {
    let modes = vec![
        CaptureMode::new(CaptureFormat::Mjpeg, 640, 480).expect("mjpeg mode"),
        CaptureMode::new(CaptureFormat::YUYV, 640, 480).expect("yuy2 mode"),
    ];
    let capabilities = CaptureCapabilities::new(modes).expect("capabilities");
    let yuy2 = CapturedColorFrame::yuy2(
        identity(1),
        timing(),
        640,
        480,
        1280,
        &vec![1; 1280 * 480],
        unknown_color(),
        Some(JPEG),
    )
    .expect("yuy2");
    let mjpeg = CapturedColorFrame::mjpeg(identity(2), timing(), 640, 480, JPEG, unknown_color())
        .expect("mjpeg");
    let mut source = MemoryColorCapture::new(capabilities, vec![yuy2, mjpeg]).expect("source");

    assert_eq!(
        ColorCaptureSource::capabilities(&source)
            .modes()
            .iter()
            .map(|mode| mode.format())
            .collect::<Vec<_>>(),
        vec![CaptureFormat::Mjpeg, CaptureFormat::Yuy2]
    );
    assert_eq!(
        source.pull_frame().expect("yuy2").format(),
        CaptureFormat::Yuy2
    );
    let pulled = source.pull_frame().expect("mjpeg");
    assert_eq!(pulled.format(), CaptureFormat::Mjpeg);
    assert_eq!(pulled.timing().captured_at(), timestamp());
    assert!(source.pull_frame().is_err());
}

#[test]
fn capture_modes_reject_empty_duplicate_and_odd_yuy2() {
    let error = CaptureCapabilities::new(vec![]).expect_err("empty");
    assert_eq!(invalid_field(error), "capabilities");

    let odd = CaptureMode::new(CaptureFormat::Yuy2, 3, 2).expect_err("odd");
    assert_eq!(invalid_field(odd), "width");

    let mjpeg = CaptureMode::new(CaptureFormat::Mjpeg, 3, 2).expect("odd mjpeg width");
    let error = CaptureCapabilities::new(vec![mjpeg, mjpeg]).expect_err("duplicate");
    assert_eq!(invalid_field(error), "capabilities");
}

#[test]
fn unadvertised_frame_cannot_enter_the_memory_source() {
    let capabilities = CaptureCapabilities::new(vec![
        CaptureMode::new(CaptureFormat::Mjpeg, 640, 480).expect("mode"),
    ])
    .expect("capabilities");
    let frame = yuy2_frame(&padded_yuy2(), 10, None).expect("frame");
    let error = MemoryColorCapture::new(capabilities, vec![frame]).expect_err("not advertised");
    assert_eq!(invalid_field(error), "frame");
}

#[test]
fn delivery_video_codec_serialization_is_unchanged() {
    assert_eq!(
        serde_json::to_string(&VideoCodec::Mjpeg).expect("json"),
        "\"mjpeg\""
    );
    assert_eq!(
        serde_json::to_string(&VideoCodec::H264).expect("json"),
        "\"h264\""
    );
    assert_eq!(
        serde_json::to_string(&VideoCodec::H265).expect("json"),
        "\"h265\""
    );
    let source = VideoSource::new_mjpeg(source(), "cam", "bench", None, None, 640, 480, 5)
        .expect("catalog source");
    assert_eq!(source.capabilities(), &[VideoCodec::Mjpeg]);
}

fn assert_yuy2_metadata(frame: &CapturedColorFrame) {
    assert_eq!(frame.format(), CaptureFormat::Yuy2);
    assert_eq!(frame.width(), 4);
    assert_eq!(frame.height(), 2);
    assert_eq!(frame.stride(), Some(10));
    assert_eq!(frame.identity().source_id(), &source());
    assert_eq!(frame.identity().sequence(), 7);
    assert_eq!(frame.timing().captured_at(), timestamp());
    assert_eq!(frame.timing().clock().identity(), Some("device-clock"));
    assert_eq!(frame.timing().clock().monotonic_ns(), Some(12_345));
    assert_eq!(frame.color().matrix(), ColorMatrix::Bt709);
    assert_eq!(frame.color().range(), ColorRange::Full);
    assert_eq!(frame.original_jpeg(), Some(JPEG));
}

fn padded_yuy2() -> Vec<u8> {
    let mut bytes = vec![0; 20];
    bytes[..8].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    bytes[8] = 0xA0;
    bytes[9] = 0xA1;
    bytes[10..18].copy_from_slice(&[8, 9, 10, 11, 12, 13, 14, 15]);
    bytes[18] = 0xB0;
    bytes[19] = 0xB1;
    bytes
}
