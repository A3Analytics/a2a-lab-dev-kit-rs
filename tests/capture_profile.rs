//! Hardware-free capture profile selection.

use a2a_lab_dev_kit::A2aLabError;
use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureFormatPreference, CaptureMode,
    CaptureProfileError, CaptureProfileRequest, CaptureRate, confirm_selected_profile,
    select_capture_profile,
};

fn mode(format: CaptureFormat, width: u32, height: u32) -> CaptureMode {
    CaptureMode::new(format, width, height).expect("mode")
}

fn profile(format: CaptureFormat, width: u32, height: u32, fps: u32) -> AdvertisedCaptureProfile {
    AdvertisedCaptureProfile::new(mode(format, width, height), CaptureRate::fps(fps).unwrap())
}

fn output() -> CaptureRate {
    CaptureRate::output_default()
}

#[test]
fn auto_keeps_same_resolution_mjpeg() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        profile(CaptureFormat::Yuy2, 640, 480, 30),
    ];
    let request = CaptureProfileRequest::auto(1920, 1080).unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().format(), CaptureFormat::Mjpeg);
    assert_eq!(selected.mode().width(), 1920);
    assert_eq!(selected.mode().height(), 1080);
    assert_eq!(selected.sensor_rate(), CaptureRate::fps(30).unwrap());
    assert_eq!(selected.output_rate(), output());
}

#[test]
fn explicit_yuy2_reports_available_profiles() {
    let advertised = vec![
        profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        profile(CaptureFormat::Yuy2, 640, 480, 30),
    ];
    let request =
        CaptureProfileRequest::new(CaptureFormatPreference::Yuy2, 1920, 1080, output()).unwrap();
    match select_capture_profile(request, &advertised).unwrap_err() {
        CaptureProfileError::Unsupported { available } => assert_eq!(available, advertised),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

#[test]
fn native_five_fps_wins_over_a_higher_rate() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 1280, 720, 30),
        profile(CaptureFormat::Mjpeg, 1280, 720, 5),
        profile(CaptureFormat::Mjpeg, 1280, 720, 15),
    ];
    let request = CaptureProfileRequest::auto(1280, 720).unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.sensor_rate(), CaptureRate::fps(5).unwrap());
    assert_eq!(selected.output_rate().numerator(), 5);
}

#[test]
fn higher_rates_pick_the_lowest_adequate_rate_stably() {
    let slow = AdvertisedCaptureProfile::new(
        mode(CaptureFormat::Yuy2, 640, 480),
        CaptureRate::new(30000, 1001).unwrap(),
    );
    let faster = profile(CaptureFormat::Yuy2, 640, 480, 60);
    let earlier_duplicate = profile(CaptureFormat::Yuy2, 640, 480, 30);
    let later_duplicate = profile(CaptureFormat::Yuy2, 640, 480, 30);
    let advertised = [faster, earlier_duplicate, slow, later_duplicate];
    let request =
        CaptureProfileRequest::new(CaptureFormatPreference::Yuy2, 640, 480, output()).unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(
        selected.sensor_rate(),
        CaptureRate::new(30000, 1001).unwrap()
    );
    assert_eq!(selected.output_rate(), output());

    let without_fraction = [faster, earlier_duplicate, later_duplicate];
    let selected = select_capture_profile(request, &without_fraction).unwrap();
    assert_eq!(selected.sensor_rate(), CaptureRate::fps(30).unwrap());
}

#[test]
fn missing_and_too_slow_profiles_are_unsuitable() {
    let request = CaptureProfileRequest::auto(640, 480).unwrap();
    assert_eq!(
        select_capture_profile(request, &[]).unwrap_err(),
        CaptureProfileError::NoSuitableProfile
    );
    let too_slow = [profile(CaptureFormat::Mjpeg, 640, 480, 1)];
    assert_eq!(
        select_capture_profile(request, &too_slow).unwrap_err(),
        CaptureProfileError::NoSuitableProfile
    );
}

#[test]
fn device_rejection_does_not_select_another_profile() {
    let advertised = [profile(CaptureFormat::Mjpeg, 640, 480, 30)];
    let request = CaptureProfileRequest::auto(640, 480).unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    match confirm_selected_profile(selected, false).unwrap_err() {
        CaptureProfileError::DeviceRejected { profile } => assert_eq!(profile, selected),
        other => panic!("expected device rejection, got {other:?}"),
    }
    assert_eq!(confirm_selected_profile(selected, true).unwrap(), selected);
}

#[test]
fn rates_and_odd_raw_widths_are_rejected() {
    let zero = CaptureRate::new(0, 1).unwrap_err();
    assert!(matches!(zero, A2aLabError::Invalid { field: "rate", .. }));
    let denominator = CaptureRate::new(30, 0).unwrap_err();
    assert!(matches!(
        denominator,
        A2aLabError::Invalid { field: "rate", .. }
    ));
    let odd =
        CaptureProfileRequest::new(CaptureFormatPreference::Yuy2, 3, 2, output()).unwrap_err();
    assert!(matches!(odd, A2aLabError::Invalid { field: "width", .. }));
}
