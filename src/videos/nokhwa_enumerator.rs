//! Nokhwa-backed [`super::CameraEnumerator`].

use nokhwa::utils::{ApiBackend, CameraIndex};
use nokhwa::{nokhwa_initialize, query};

use crate::error::A2aLabError;
use crate::videos::{
    CameraDiscoverySnapshot, CameraEnumerator, DiscoveredCamera, UnsupportedCameraNode,
    nokhwa_profiles_for_index,
};

/// Queries native cameras and retains only MJPEG/YUYV-capable nodes.
#[derive(Debug, Clone, Copy, Default)]
pub struct NokhwaEnumerator;

impl CameraEnumerator for NokhwaEnumerator {
    fn discover(&self) -> Result<CameraDiscoverySnapshot, A2aLabError> {
        nokhwa_initialize(|_granted| {});
        let listed = query(ApiBackend::Auto).map_err(|error| {
            A2aLabError::invalid("camera", format!("discovery failed: {error}"))
        })?;
        let mut capable = Vec::new();
        let mut unsupported = Vec::new();
        for info in listed {
            let Some(index) = camera_index(info.index()) else {
                continue;
            };
            let human_name = info.human_name();
            let description = info.description().to_owned();
            let misc = info.misc();
            let profiles = nokhwa_profiles_for_index(index)?;
            if profiles.is_empty() {
                unsupported.push(UnsupportedCameraNode::new(
                    index,
                    human_name,
                    description,
                    misc,
                ));
                continue;
            }
            capable.push(DiscoveredCamera::new(
                index,
                human_name,
                description,
                misc,
                profiles,
            ));
        }
        Ok(CameraDiscoverySnapshot::from_parts(capable, unsupported))
    }
}

fn camera_index(index: &CameraIndex) -> Option<u32> {
    match index {
        CameraIndex::Index(value) => Some(*value),
        CameraIndex::String(_) => None,
    }
}
