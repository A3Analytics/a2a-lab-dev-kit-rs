//! Cross-platform camera discovery seam.

use crate::error::A2aLabError;
use crate::videos::CameraDiscoverySnapshot;

/// Backend that lists cameras with usable MJPEG or YUYV profiles.
pub trait CameraEnumerator {
    /// Returns capable cameras and skipped zero-profile nodes.
    fn discover(&self) -> Result<CameraDiscoverySnapshot, A2aLabError>;
}
