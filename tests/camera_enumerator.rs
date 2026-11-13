//! Capability-filtered camera discovery and selector resolution.

use std::collections::HashSet;

use a2a_lab_dev_kit::config::CameraSelector;
use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CameraDiscoverySnapshot, CameraEnumerator, CaptureFormat,
    CaptureMode, CaptureRate, DiscoveredCamera, UnsupportedCameraNode, resolve_camera_selector,
};

#[derive(Clone)]
struct StubEnumerator {
    snapshot: CameraDiscoverySnapshot,
}

impl CameraEnumerator for StubEnumerator {
    fn discover(&self) -> Result<CameraDiscoverySnapshot, a2a_lab_dev_kit::A2aLabError> {
        Ok(self.snapshot.clone())
    }
}

fn profile() -> AdvertisedCaptureProfile {
    AdvertisedCaptureProfile::new(
        CaptureMode::new(CaptureFormat::Yuy2, 640, 480).expect("mode"),
        CaptureRate::fps(30).expect("rate"),
    )
}

fn capable(index: u32, name: &str) -> DiscoveredCamera {
    DiscoveredCamera::new(index, name, "", "", vec![profile()])
}

#[test]
fn enumerator_seam_filters_zero_profile_nodes_from_capable_list() {
    let stub = StubEnumerator {
        snapshot: CameraDiscoverySnapshot::from_parts(
            vec![capable(0, "mjpeg")],
            vec![UnsupportedCameraNode::new(1, "depth", "", "")],
        ),
    };
    let discovered = stub.discover().expect("discover");
    assert_eq!(discovered.capable().len(), 1);
    assert_eq!(discovered.unsupported().len(), 1);
    assert!(discovered.capable()[0].profiles().iter().all(|item| {
        matches!(
            item.mode().format(),
            CaptureFormat::Mjpeg | CaptureFormat::Yuy2
        )
    }));
}

#[test]
fn resolve_covers_index_text_default_claimed_and_failures() {
    let snap = CameraDiscoverySnapshot::from_parts(
        vec![capable(0, "alpha"), capable(2, "beta")],
        vec![UnsupportedCameraNode::new(1, "raw-only", "", "")],
    );
    let mut claimed = HashSet::new();
    let default_first = resolve_camera_selector(None, &snap, &claimed).expect("default");
    assert_eq!(default_first.index(), 0);
    claimed.insert(0);
    let default_second = resolve_camera_selector(None, &snap, &claimed).expect("second default");
    assert_eq!(default_second.index(), 2);

    let by_index = resolve_camera_selector(Some(&CameraSelector::Index(2)), &snap, &HashSet::new())
        .expect("index");
    assert_eq!(by_index.human_name(), "beta");

    let by_text = resolve_camera_selector(
        Some(&CameraSelector::Text("ALPHA".into())),
        &snap,
        &HashSet::new(),
    )
    .expect("text");
    assert_eq!(by_text.index(), 0);

    let unsupported = resolve_camera_selector(
        Some(&CameraSelector::Text("raw-only".into())),
        &snap,
        &HashSet::new(),
    )
    .expect_err("unsupported text");
    assert!(unsupported.to_string().contains("unsupported-format"));
    assert!(unsupported.to_string().contains("raw-only"));

    let unavailable =
        resolve_camera_selector(Some(&CameraSelector::Index(9)), &snap, &HashSet::new())
            .expect_err("unavailable");
    assert!(unavailable.to_string().contains("unavailable-index"));
    assert!(unavailable.to_string().contains("`9`"));
    assert!(unavailable.to_string().contains("alpha[0]"));
}

#[cfg(feature = "nokhwa")]
#[test]
fn nokhwa_maps_only_mjpeg_and_yuyv_profiles() {
    use a2a_lab_dev_kit::videos::profiles_from_nokhwa;
    use nokhwa::utils::{CameraFormat, FrameFormat};

    let formats = [
        CameraFormat::new_from(640, 480, FrameFormat::MJPEG, 30),
        CameraFormat::new_from(640, 480, FrameFormat::YUYV, 30),
        CameraFormat::new_from(640, 480, FrameFormat::NV12, 30),
    ];
    let profiles = profiles_from_nokhwa(&formats);
    assert_eq!(profiles.len(), 2);
}
