//! Hardware-free camera and discovery overrides for tests.

use std::collections::HashMap;

use crate::videos::{CameraEnumerator, ColorCamera};

/// Injectable producers used instead of native discovery and open.
#[derive(Default)]
pub struct VideoRuntimeInjections {
    cameras: HashMap<String, Box<dyn ColorCamera + Send>>,
    enumerator: Option<Box<dyn CameraEnumerator + Send>>,
}

impl VideoRuntimeInjections {
    /// No injected backends.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Registers one injected camera for a configured source id.
    #[must_use]
    pub fn with_camera(
        mut self,
        source_id: impl Into<String>,
        camera: Box<dyn ColorCamera + Send>,
    ) -> Self {
        self.cameras.insert(source_id.into(), camera);
        self
    }

    /// Replaces native camera discovery.
    #[must_use]
    pub fn with_enumerator(mut self, enumerator: Box<dyn CameraEnumerator + Send>) -> Self {
        self.enumerator = Some(enumerator);
        self
    }

    pub(crate) fn take_camera(&mut self, source_id: &str) -> Option<Box<dyn ColorCamera + Send>> {
        self.cameras.remove(source_id)
    }

    pub(crate) fn enumerator(&self) -> Option<&(dyn CameraEnumerator + Send)> {
        self.enumerator.as_deref()
    }

    pub(crate) fn skips_native_discovery(&self) -> bool {
        !self.cameras.is_empty() || self.enumerator.is_some()
    }
}
