//! Declarative camera selector resolution against discovered capabilities.

use std::collections::HashSet;
use std::fmt::Write;
use std::hash::BuildHasher;

use crate::config::CameraSelector;
use crate::error::A2aLabError;
use crate::videos::{CameraDiscoverySnapshot, DiscoveredCamera, UnsupportedCameraNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorKind<'a> {
    Omitted,
    Index(u32),
    Text(&'a str),
}

/// Resolves `selector` against `snapshot`, skipping `claimed` indices.
pub fn resolve_camera_selector<S: BuildHasher>(
    selector: Option<&CameraSelector>,
    snapshot: &CameraDiscoverySnapshot,
    claimed: &HashSet<u32, S>,
) -> Result<DiscoveredCamera, A2aLabError> {
    let kind = selector_kind(selector);
    match kind {
        SelectorKind::Omitted => first_unclaimed(snapshot.capable(), claimed, "default camera"),
        SelectorKind::Index(index) => resolve_index(index, snapshot),
        SelectorKind::Text(text) => resolve_text(text, snapshot),
    }
}

fn selector_kind(selector: Option<&CameraSelector>) -> SelectorKind<'_> {
    match selector {
        None => SelectorKind::Omitted,
        Some(CameraSelector::Index(index)) => SelectorKind::Index(*index),
        Some(CameraSelector::Text(text)) => SelectorKind::Text(text),
    }
}

fn first_unclaimed<S: BuildHasher>(
    capable: &[DiscoveredCamera],
    claimed: &HashSet<u32, S>,
    selector_label: &str,
) -> Result<DiscoveredCamera, A2aLabError> {
    for camera in capable {
        if !claimed.contains(&camera.index()) {
            return Ok(camera.clone());
        }
    }
    Err(resolve_error(
        "no-match",
        selector_label,
        capable,
        "every capable camera is already claimed",
    ))
}

fn resolve_index(
    index: u32,
    snapshot: &CameraDiscoverySnapshot,
) -> Result<DiscoveredCamera, A2aLabError> {
    let label = index.to_string();
    if let Some(camera) = snapshot
        .capable()
        .iter()
        .find(|camera| camera.index() == index)
    {
        return Ok(camera.clone());
    }
    if snapshot
        .unsupported()
        .iter()
        .any(|node| node.index() == index)
    {
        return Err(resolve_error(
            "unsupported-format",
            &label,
            snapshot.capable(),
            "camera does not advertise MJPEG or YUYV",
        ));
    }
    Err(resolve_error(
        "unavailable-index",
        &label,
        snapshot.capable(),
        "no camera advertises that index",
    ))
}

fn resolve_text(
    text: &str,
    snapshot: &CameraDiscoverySnapshot,
) -> Result<DiscoveredCamera, A2aLabError> {
    let mut capable_matches: Vec<&DiscoveredCamera> = snapshot
        .capable()
        .iter()
        .filter(|camera| metadata_matches_text(camera, text))
        .collect();
    if capable_matches.len() > 1 {
        return Err(resolve_error(
            "ambiguous",
            text,
            snapshot.capable(),
            "multiple capable cameras match that text",
        ));
    }
    if let Some(camera) = capable_matches.pop() {
        return Ok(camera.clone());
    }
    let unsupported_matches: Vec<&UnsupportedCameraNode> = snapshot
        .unsupported()
        .iter()
        .filter(|node| metadata_matches_node(node, text))
        .collect();
    if !unsupported_matches.is_empty() {
        return Err(resolve_error(
            "unsupported-format",
            text,
            snapshot.capable(),
            "matched camera does not advertise MJPEG or YUYV",
        ));
    }
    Err(resolve_error(
        "no-match",
        text,
        snapshot.capable(),
        "no camera name, description, or misc field matches",
    ))
}

fn metadata_matches_text(camera: &DiscoveredCamera, text: &str) -> bool {
    field_matches(text, camera.human_name())
        || field_matches(text, camera.description())
        || field_matches(text, camera.misc())
}

fn metadata_matches_node(node: &UnsupportedCameraNode, text: &str) -> bool {
    field_matches(text, node.human_name())
        || field_matches(text, node.description())
        || field_matches(text, node.misc())
}

fn field_matches(selector: &str, field: &str) -> bool {
    field.eq_ignore_ascii_case(selector)
        || field
            .to_ascii_lowercase()
            .contains(&selector.to_ascii_lowercase())
}

fn resolve_error(
    kind: &str,
    selector: &str,
    capable: &[DiscoveredCamera],
    detail: &str,
) -> A2aLabError {
    A2aLabError::invalid(
        "camera_id",
        format_resolve_message(kind, selector, capable, detail),
    )
}

fn format_resolve_message(
    kind: &str,
    selector: &str,
    capable: &[DiscoveredCamera],
    detail: &str,
) -> String {
    let mut message = format!("{kind} camera selector `{selector}`: {detail}");
    if capable.is_empty() {
        message.push_str("; no capable cameras are available");
        return message;
    }
    message.push_str("; capable cameras: ");
    for (offset, camera) in capable.iter().enumerate() {
        if offset > 0 {
            message.push_str(", ");
        }
        let _ = write!(message, "{}[{}]", camera.human_name(), camera.index());
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::videos::{AdvertisedCaptureProfile, CaptureFormat, CaptureMode, CaptureRate};

    fn capable(index: u32, name: &str, description: &str, misc: &str) -> DiscoveredCamera {
        let profile = AdvertisedCaptureProfile::new(
            CaptureMode::new(CaptureFormat::Mjpeg, 640, 480).expect("mode"),
            CaptureRate::fps(30).expect("rate"),
        );
        DiscoveredCamera::new(index, name, description, misc, vec![profile])
    }

    #[test]
    fn default_picks_first_unclaimed_capable_camera() {
        let snap = CameraDiscoverySnapshot::from_parts(
            vec![capable(0, "built-in", "", ""), capable(1, "usb", "", "")],
            vec![],
        );
        let mut claimed = HashSet::new();
        let first = resolve_camera_selector(None, &snap, &claimed).expect("first");
        assert_eq!(first.index(), 0);
        claimed.insert(0);
        let second = resolve_camera_selector(None, &snap, &claimed).expect("second");
        assert_eq!(second.index(), 1);
    }

    #[test]
    fn index_resolves_native_capture_index() {
        let snap = CameraDiscoverySnapshot::from_parts(vec![capable(3, "lab", "", "")], vec![]);
        let camera =
            resolve_camera_selector(Some(&CameraSelector::Index(3)), &snap, &HashSet::new())
                .expect("index");
        assert_eq!(camera.index(), 3);
    }

    #[test]
    fn text_matches_name_description_and_misc_case_insensitively() {
        let snap = CameraDiscoverySnapshot::from_parts(
            vec![
                capable(0, "EMEET", "", ""),
                capable(1, "", "hp hd camera", ""),
                capable(2, "", "", "serial-abc"),
            ],
            vec![],
        );
        assert_eq!(
            resolve_camera_selector(
                Some(&CameraSelector::Text("emeet".into())),
                &snap,
                &HashSet::new()
            )
            .expect("name")
            .index(),
            0
        );
        assert_eq!(
            resolve_camera_selector(
                Some(&CameraSelector::Text("HP HD".into())),
                &snap,
                &HashSet::new(),
            )
            .expect("desc")
            .index(),
            1
        );
        assert_eq!(
            resolve_camera_selector(
                Some(&CameraSelector::Text("SERIAL-ABC".into())),
                &snap,
                &HashSet::new(),
            )
            .expect("misc")
            .index(),
            2
        );
    }

    #[test]
    fn failures_name_selector_and_candidates() {
        let snap = CameraDiscoverySnapshot::from_parts(vec![capable(0, "one", "", "")], vec![]);
        let missing = resolve_camera_selector(
            Some(&CameraSelector::Text("missing".into())),
            &snap,
            &HashSet::new(),
        )
        .expect_err("missing");
        assert!(missing.to_string().contains("no-match"));
        assert!(missing.to_string().contains("`missing`"));
        assert!(missing.to_string().contains("one[0]"));

        let ambiguous = CameraDiscoverySnapshot::from_parts(
            vec![capable(0, "dup", "x", ""), capable(1, "Dup", "", "")],
            vec![],
        );
        let err = resolve_camera_selector(
            Some(&CameraSelector::Text("dup".into())),
            &ambiguous,
            &HashSet::new(),
        )
        .expect_err("ambiguous");
        assert!(err.to_string().contains("ambiguous"));

        let unsupported = CameraDiscoverySnapshot::from_parts(
            vec![],
            vec![UnsupportedCameraNode::new(4, "depth", "", "")],
        );
        let err = resolve_camera_selector(
            Some(&CameraSelector::Index(4)),
            &unsupported,
            &HashSet::new(),
        )
        .expect_err("unsupported index");
        assert!(err.to_string().contains("unsupported-format"));
        assert!(err.to_string().contains("`4`"));

        let err = resolve_camera_selector(
            Some(&CameraSelector::Index(9)),
            &unsupported,
            &HashSet::new(),
        )
        .expect_err("bad index");
        assert!(err.to_string().contains("unavailable-index"));
    }
}
