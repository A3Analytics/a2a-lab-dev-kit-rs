//! Flexible capture profile selection without exact resolution matching.

use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureFormatPreference, CaptureMode,
    CaptureProfileError, CaptureRate, select_capture_profile_flexible,
};

fn mode(format: CaptureFormat, width: u32, height: u32) -> CaptureMode {
    CaptureMode::new(format, width, height).expect("mode")
}

fn profile(format: CaptureFormat, width: u32, height: u32, fps: u32) -> AdvertisedCaptureProfile {
    AdvertisedCaptureProfile::new(mode(format, width, height), CaptureRate::fps(fps).unwrap())
}

#[test]
fn flexible_auto_prefers_mjpeg_and_highest_matching_resolution() {
    let advertised = [
        profile(CaptureFormat::Yuy2, 640, 480, 30),
        profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        profile(CaptureFormat::Mjpeg, 1280, 720, 30),
    ];
    let selected = select_capture_profile_flexible(
        CaptureFormatPreference::Auto,
        CaptureRate::output_default(),
        &advertised,
    )
    .unwrap();
    assert_eq!(selected.mode().format(), CaptureFormat::Mjpeg);
    assert_eq!(selected.mode().width(), 1920);
}

#[test]
fn flexible_reports_unsuitable_when_no_profile_meets_output_rate() {
    let advertised = [profile(CaptureFormat::Mjpeg, 640, 480, 1)];
    assert_eq!(
        select_capture_profile_flexible(
            CaptureFormatPreference::Auto,
            CaptureRate::output_default(),
            &advertised,
        )
        .unwrap_err(),
        CaptureProfileError::NoSuitableProfile
    );
}
