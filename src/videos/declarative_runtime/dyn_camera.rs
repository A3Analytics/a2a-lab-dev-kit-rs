//! Type-erased [`ColorCamera`] for one pump implementation.

use crate::error::A2aLabError;
use crate::videos::{
    AdvertisedCaptureProfile, ColorCamera, RawCameraSample, SelectedCaptureProfile,
};

/// Shared [`ColorCamera`] handle for native and injected capture drains.
pub(crate) struct DynColorCamera(pub Box<dyn ColorCamera + Send>);

impl ColorCamera for DynColorCamera {
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
        self.0.advertised()
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError> {
        self.0.open(profile)
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError> {
        self.0.raw_frame()
    }

    fn stop(&mut self) -> Result<(), A2aLabError> {
        self.0.stop()
    }
}
