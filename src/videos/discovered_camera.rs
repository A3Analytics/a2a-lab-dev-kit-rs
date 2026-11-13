//! One discovered camera and its usable capture profiles.

use crate::videos::AdvertisedCaptureProfile;

/// Human-readable metadata for a camera that did not advertise MJPEG or YUYV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedCameraNode {
    index: u32,
    human_name: String,
    description: String,
    misc: String,
}

impl UnsupportedCameraNode {
    /// Stores metadata for a skipped zero-profile node.
    #[must_use]
    pub fn new(
        index: u32,
        human_name: impl Into<String>,
        description: impl Into<String>,
        misc: impl Into<String>,
    ) -> Self {
        Self {
            index,
            human_name: human_name.into(),
            description: description.into(),
            misc: misc.into(),
        }
    }

    /// Native capture index for the node.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Display name reported by the backend.
    #[must_use]
    pub fn human_name(&self) -> &str {
        &self.human_name
    }

    /// Backend-specific description text.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Backend-specific misc identifiers.
    #[must_use]
    pub fn misc(&self) -> &str {
        &self.misc
    }
}

/// A camera advertising at least one MJPEG or YUYV profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCamera {
    index: u32,
    human_name: String,
    description: String,
    misc: String,
    profiles: Vec<AdvertisedCaptureProfile>,
}

impl DiscoveredCamera {
    /// Stores a capable camera row.
    #[must_use]
    pub fn new(
        index: u32,
        human_name: impl Into<String>,
        description: impl Into<String>,
        misc: impl Into<String>,
        profiles: Vec<AdvertisedCaptureProfile>,
    ) -> Self {
        Self {
            index,
            human_name: human_name.into(),
            description: description.into(),
            misc: misc.into(),
            profiles,
        }
    }

    /// Native capture index used by [`crate::config::CameraSelector::Index`].
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Display name reported by the backend.
    #[must_use]
    pub fn human_name(&self) -> &str {
        &self.human_name
    }

    /// Backend-specific description text.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Backend-specific misc identifiers.
    #[must_use]
    pub fn misc(&self) -> &str {
        &self.misc
    }

    /// Usable MJPEG/YUYV profiles retained after capability filtering.
    #[must_use]
    pub fn profiles(&self) -> &[AdvertisedCaptureProfile] {
        &self.profiles
    }
}

/// Capable cameras plus skipped zero-profile nodes for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CameraDiscoverySnapshot {
    capable: Vec<DiscoveredCamera>,
    unsupported: Vec<UnsupportedCameraNode>,
}

impl CameraDiscoverySnapshot {
    /// Empty snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            capable: Vec::new(),
            unsupported: Vec::new(),
        }
    }

    /// Cameras advertising MJPEG or YUYV.
    #[must_use]
    pub fn capable(&self) -> &[DiscoveredCamera] {
        &self.capable
    }

    /// Nodes omitted because they advertised no usable profiles.
    #[must_use]
    pub fn unsupported(&self) -> &[UnsupportedCameraNode] {
        &self.unsupported
    }

    /// Builds a snapshot from capable and skipped nodes.
    #[must_use]
    pub fn from_parts(
        capable: Vec<DiscoveredCamera>,
        unsupported: Vec<UnsupportedCameraNode>,
    ) -> Self {
        Self {
            capable,
            unsupported,
        }
    }
}
