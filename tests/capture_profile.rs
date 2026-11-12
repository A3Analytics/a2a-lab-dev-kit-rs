//! Hardware-free capture profile selection.

use a2a_lab_dev_kit::A2aLabError;
use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureFormatPreference, CaptureMode,
    CaptureProfileError, CaptureProfileRequest, CaptureRate, confirm_selected_profile,
    rank_capture_profiles, select_capture_profile,
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
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Auto,
        1920,
        1080,
        output(),
        CaptureRate::fps(30).unwrap(),
    )
    .unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().format(), CaptureFormat::Mjpeg);
    assert_eq!(selected.mode().width(), 1920);
    assert_eq!(selected.mode().height(), 1080);
    assert_eq!(selected.capture_rate(), CaptureRate::fps(30).unwrap());
    assert_eq!(selected.output_rate(), output());
}

#[test]
fn explicit_yuy2_reports_available_profiles() {
    let advertised = vec![
        profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        profile(CaptureFormat::Yuy2, 640, 480, 30),
    ];
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Yuy2,
        1920,
        1080,
        output(),
        CaptureRate::fps(30).unwrap(),
    )
    .unwrap();
    match select_capture_profile(request, &advertised).unwrap_err() {
        CaptureProfileError::Unsupported { available } => assert_eq!(available, advertised),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

#[test]
fn exact_capture_rate_is_selected_not_a_nearby_rate() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 1280, 720, 30),
        profile(CaptureFormat::Mjpeg, 1280, 720, 5),
        profile(CaptureFormat::Mjpeg, 1280, 720, 15),
    ];
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Auto,
        1280,
        720,
        output(),
        CaptureRate::fps(15).unwrap(),
    )
    .unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.capture_rate(), CaptureRate::fps(15).unwrap());
    assert_eq!(selected.output_rate().numerator(), 5);
}

#[test]
fn omitted_capture_rate_selects_lowest_rate_at_the_pinned_size() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 1280, 720, 30),
        profile(CaptureFormat::Mjpeg, 1280, 720, 15),
        profile(CaptureFormat::Mjpeg, 640, 480, 5),
    ];
    let request = CaptureProfileRequest::new(
        CaptureFormatPreference::Auto,
        Some(1280),
        Some(720),
        output(),
        None,
    )
    .unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().width(), 1280);
    assert_eq!(selected.capture_rate(), CaptureRate::fps(15).unwrap());
}

#[test]
fn omitted_size_selects_largest_resolution_then_lowest_rate() {
    let advertised = [
        profile(CaptureFormat::Yuy2, 640, 480, 5),
        profile(CaptureFormat::Yuy2, 1280, 720, 30),
        profile(CaptureFormat::Yuy2, 1280, 720, 15),
    ];
    let request = CaptureProfileRequest::unconstrained(CaptureFormatPreference::Yuy2, output());
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().width(), 1280);
    assert_eq!(selected.mode().height(), 720);
    assert_eq!(selected.capture_rate(), CaptureRate::fps(15).unwrap());
    let ranked = rank_capture_profiles(request, &advertised).unwrap();
    assert_eq!(ranked[1].capture_rate(), CaptureRate::fps(30).unwrap());
}

#[test]
fn omitted_size_caps_default_resolution_at_1080p() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 3840, 2160, 15),
        profile(CaptureFormat::Mjpeg, 2560, 1440, 30),
        profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        profile(CaptureFormat::Mjpeg, 1920, 1080, 15),
        profile(CaptureFormat::Mjpeg, 1280, 720, 5),
    ];
    let request = CaptureProfileRequest::unconstrained(CaptureFormatPreference::Auto, output());
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().width(), 1920);
    assert_eq!(selected.mode().height(), 1080);
    assert_eq!(selected.capture_rate(), CaptureRate::fps(15).unwrap());
}

#[test]
fn explicit_size_may_exceed_the_1080p_default_cap() {
    let advertised = [
        profile(CaptureFormat::Mjpeg, 3840, 2160, 30),
        profile(CaptureFormat::Mjpeg, 1920, 1080, 15),
    ];
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Auto,
        3840,
        2160,
        output(),
        CaptureRate::fps(30).unwrap(),
    )
    .unwrap();
    let selected = select_capture_profile(request, &advertised).unwrap();
    assert_eq!(selected.mode().width(), 3840);
    assert_eq!(selected.mode().height(), 2160);
}

#[test]
fn mismatched_capture_rate_is_unsupported() {
    let advertised = [
        profile(CaptureFormat::Yuy2, 640, 480, 60),
        profile(CaptureFormat::Yuy2, 640, 480, 30),
    ];
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Yuy2,
        640,
        480,
        output(),
        CaptureRate::fps(15).unwrap(),
    )
    .unwrap();
    match select_capture_profile(request, &advertised).unwrap_err() {
        CaptureProfileError::Unsupported { available } => assert_eq!(available, advertised),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

#[test]
fn missing_and_too_slow_profiles_are_unsuitable() {
    let request = CaptureProfileRequest::auto(640, 480).unwrap();
    assert_eq!(
        select_capture_profile(request, &[]).unwrap_err(),
        CaptureProfileError::NoSuitableProfile
    );
    let too_slow = [profile(CaptureFormat::Mjpeg, 640, 480, 1)];
    match select_capture_profile(request, &too_slow).unwrap_err() {
        CaptureProfileError::Unsupported { available } => assert_eq!(available, too_slow),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

#[test]
fn device_rejection_does_not_select_another_profile() {
    let advertised = [profile(CaptureFormat::Mjpeg, 640, 480, 30)];
    let request = CaptureProfileRequest::exact(
        CaptureFormatPreference::Auto,
        640,
        480,
        output(),
        CaptureRate::fps(30).unwrap(),
    )
    .unwrap();
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
    let odd = CaptureProfileRequest::exact(CaptureFormatPreference::Yuy2, 3, 2, output(), output())
        .unwrap_err();
    assert!(matches!(odd, A2aLabError::Invalid { field: "width", .. }));
}
