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
    CaptureProfileRequest::new(
        CaptureFormatPreference::Yuy2,
        640,
        480,
        CaptureRate::fps(5).expect("output"),
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
    assert_eq!(selected.sensor_rate().numerator(), 30);
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
    assert_eq!(mapped[1].sensor_rate().numerator(), 5);
}
