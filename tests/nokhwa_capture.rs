//! Injected-camera negotiation and Nokhwa format mapping.

use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CaptureClock, CaptureFormat, CaptureFormatPreference,
    CaptureIdentity, CaptureMode, CaptureProfileRequest, CaptureRate, CaptureSession,
    CaptureTiming, CapturedColorFrame, ColorCamera, ColorMatrix, ColorProperties, ColorRange,
    RawCameraSample, SelectedCaptureProfile,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};

struct FakeCamera {
    formats: Vec<AdvertisedCaptureProfile>,
    negotiated: Option<SelectedCaptureProfile>,
    raw: RawCameraSample,
    opens: u32,
    stopped: bool,
    deny: bool,
}

impl ColorCamera for FakeCamera {
    fn advertised(
        &mut self,
    ) -> Result<Vec<AdvertisedCaptureProfile>, a2a_lab_dev_kit::A2aLabError> {
        Ok(self.formats.clone())
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, a2a_lab_dev_kit::A2aLabError> {
        if self.deny {
            return Err(a2a_lab_dev_kit::A2aLabError::invalid(
                "camera",
                "permission denied",
            ));
        }
        self.opens += 1;
        self.stopped = false;
        self.negotiated = Some(*profile);
        Ok(*profile)
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, a2a_lab_dev_kit::A2aLabError> {
        if self.stopped {
            return Err(a2a_lab_dev_kit::A2aLabError::invalid(
                "camera",
                "stream is stopped",
            ));
        }
        Ok(self.raw.clone())
    }

    fn stop(&mut self) -> Result<(), a2a_lab_dev_kit::A2aLabError> {
        self.stopped = true;
        self.negotiated = None;
        Ok(())
    }
}

fn profile(format: CaptureFormat, width: u32, height: u32, fps: u32) -> AdvertisedCaptureProfile {
    AdvertisedCaptureProfile::new(
        CaptureMode::new(format, width, height).expect("mode"),
        CaptureRate::fps(fps).expect("rate"),
    )
}

fn request() -> CaptureProfileRequest {
    CaptureProfileRequest::exact(
        CaptureFormatPreference::Yuy2,
        640,
        480,
        CaptureRate::fps(5).expect("output"),
        CaptureRate::fps(30).expect("capture"),
    )
    .expect("request")
}

fn timing() -> CaptureTiming {
    CaptureTiming::new(
        UtcTimestamp::parse("2020-01-01T00:00:00Z").expect("time"),
        CaptureClock::monotonic(10),
    )
}

fn identity() -> CaptureIdentity {
    CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), 3)
}

#[test]
fn negotiation_keeps_raw_padding_and_reconnects() {
    let mut packed = vec![0_u8; 640 * 2 * 480];
    packed[0] = 7;
    let camera = FakeCamera {
        formats: vec![
            profile(CaptureFormat::Mjpeg, 640, 480, 30),
            profile(CaptureFormat::Yuy2, 640, 480, 30),
            profile(CaptureFormat::Yuy2, 320, 240, 30),
        ],
        negotiated: None,
        raw: RawCameraSample::new(CaptureFormat::Yuy2, 640, 480, 640 * 2, packed.clone()),
        opens: 0,
        stopped: true,
        deny: false,
    };
    let mut session = CaptureSession::new(camera, request());
    let selected = session.open().expect("open");
    assert_eq!(selected.mode().format(), CaptureFormat::Yuy2);
    assert_eq!(selected.mode().width(), 640);
    assert_eq!(selected.capture_rate().numerator(), 30);
    assert_eq!(selected.output_rate().numerator(), 5);
    let frame = session
        .read(
            identity(),
            timing(),
            ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown),
        )
        .expect("frame");
    assert_eq!(frame.format(), CaptureFormat::Yuy2);
    assert_eq!(frame.bytes(), packed.as_slice());
    assert_eq!(frame.identity().sequence(), 3);
    session.reconnect().expect("reconnect");
    assert_eq!(session.selected().expect("open").mode().width(), 640);
}

#[test]
fn omitted_capture_rate_skips_a_rejected_advertised_rate() {
    struct RejectLow;
    impl ColorCamera for RejectLow {
        fn advertised(
            &mut self,
        ) -> Result<Vec<AdvertisedCaptureProfile>, a2a_lab_dev_kit::A2aLabError> {
            Ok(vec![
                profile(CaptureFormat::Yuy2, 640, 480, 15),
                profile(CaptureFormat::Yuy2, 640, 480, 30),
            ])
        }

        fn open(
            &mut self,
            profile: &SelectedCaptureProfile,
        ) -> Result<SelectedCaptureProfile, a2a_lab_dev_kit::A2aLabError> {
            if profile.capture_rate().numerator() == 15 {
                return Err(a2a_lab_dev_kit::A2aLabError::invalid(
                    "camera",
                    "15 FPS is advertised but rejected",
                ));
            }
            Ok(*profile)
        }

        fn raw_frame(&mut self) -> Result<RawCameraSample, a2a_lab_dev_kit::A2aLabError> {
            Ok(RawCameraSample::new(
                CaptureFormat::Yuy2,
                640,
                480,
                4,
                vec![0, 0, 0, 0],
            ))
        }

        fn stop(&mut self) -> Result<(), a2a_lab_dev_kit::A2aLabError> {
            Ok(())
        }
    }

    let request = CaptureProfileRequest::new(
        CaptureFormatPreference::Yuy2,
        Some(640),
        Some(480),
        CaptureRate::fps(5).expect("output"),
        None,
    )
    .expect("request");
    let mut session = CaptureSession::new(RejectLow, request);
    let selected = session.open().expect("open");
    assert_eq!(selected.capture_rate().numerator(), 30);
}

#[test]
fn permission_failure_and_short_stride_are_explicit() {
    let camera = FakeCamera {
        formats: vec![profile(CaptureFormat::Yuy2, 640, 480, 30)],
        negotiated: None,
        raw: RawCameraSample::new(CaptureFormat::Yuy2, 640, 480, 2, vec![0, 0]),
        opens: 0,
        stopped: false,
        deny: true,
    };
    let mut session = CaptureSession::new(camera, request());
    assert!(session.open().is_err());
    let error = CapturedColorFrame::yuy2(
        identity(),
        timing(),
        4,
        1,
        2,
        &[0, 0],
        ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown),
        None,
    );
    assert!(error.is_err());
}

#[cfg(feature = "nokhwa")]
#[test]
fn nokhwa_maps_mjpeg_and_yuyv_and_rejects_other_layouts() {
    use a2a_lab_dev_kit::videos::profiles_from_nokhwa;
    use nokhwa::utils::{CameraFormat, FrameFormat};

    let formats = [
        CameraFormat::new_from(640, 480, FrameFormat::MJPEG, 30),
        CameraFormat::new_from(640, 480, FrameFormat::YUYV, 5),
        CameraFormat::new_from(640, 480, FrameFormat::GRAY, 30),
    ];
    let mapped = profiles_from_nokhwa(&formats);
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].mode().format(), CaptureFormat::Mjpeg);
    assert_eq!(mapped[1].mode().format(), CaptureFormat::Yuy2);
    assert_eq!(mapped[1].capture_rate().numerator(), 5);
}

#[cfg(feature = "nokhwa")]
mod nokhwa_hardening {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use a2a_lab_dev_kit::videos::{
        AUTO_PROBE_FOURCCS, AUTO_PROBE_LADDER, CaptureFormat, normalize_raw_camera_sample,
        with_nokhwa_open_lock,
    };
    use nokhwa::utils::FrameFormat;

    #[test]
    fn auto_probe_ladder_is_mjpeg_then_yuyv_and_descending_resolutions() {
        assert_eq!(AUTO_PROBE_FOURCCS, [FrameFormat::MJPEG, FrameFormat::YUYV]);
        assert_eq!(AUTO_PROBE_LADDER.len(), 3);
        assert!(AUTO_PROBE_LADDER[0].0 >= AUTO_PROBE_LADDER[1].0);
    }

    #[test]
    fn open_lock_allows_one_holder_at_a_time() {
        static PEAK: AtomicUsize = AtomicUsize::new(0);
        static ACTIVE: AtomicUsize = AtomicUsize::new(0);
        let handles: Vec<_> = (0..6)
            .map(|_| {
                thread::spawn(|| {
                    with_nokhwa_open_lock(|| {
                        let current = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
                        PEAK.fetch_max(current, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(8));
                        ACTIVE.fetch_sub(1, Ordering::SeqCst);
                    });
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("join");
        }
        assert_eq!(PEAK.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn normalize_raw_sample_trims_mjpeg_and_packs_yuy2_stride() {
        let jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11,
            0x00, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFF, 0xD9, 0xAA,
        ];
        let exact_len = jpeg.len() - 1;
        let mjpeg = normalize_raw_camera_sample(CaptureFormat::Mjpeg, 1, 1, jpeg).expect("mjpeg");
        assert_eq!(mjpeg.bytes().len(), exact_len);

        let row = 8usize;
        let height = 2u32;
        let mut yuy2 = Vec::new();
        for marker in [11_u8, 22_u8] {
            yuy2.extend(std::iter::repeat_n(marker, row));
            yuy2.extend([0, 0]);
        }
        let yuy2 = normalize_raw_camera_sample(CaptureFormat::Yuy2, 4, height, yuy2).expect("yuy2");
        assert_eq!(yuy2.stride(), u32::try_from(row).unwrap());
        let expected: Vec<u8> = std::iter::repeat_n(11_u8, row)
            .chain(std::iter::repeat_n(22_u8, row))
            .collect();
        assert_eq!(yuy2.bytes(), expected.as_slice());
    }
}
