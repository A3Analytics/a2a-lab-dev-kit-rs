//! Negotiated color capture over an injected camera.
//!
//! Profile selection stays in [`crate::videos::select_capture_profile`]. This
//! module opens that profile, reads one raw buffer, and copies it into a
//! [`CapturedColorFrame`] without decoding MJPEG or YUY2. A padded or unknown
//! layout fails instead of being reinterpreted.

use crate::error::A2aLabError;
use crate::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureIdentity, CaptureProfileRequest, CaptureTiming,
    CapturedColorFrame, ColorProperties, SelectedCaptureProfile, confirm_selected_profile,
    select_capture_profile,
};

/// One raw camera buffer before it becomes a capture frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCameraSample {
    format: CaptureFormat,
    width: u32,
    height: u32,
    stride: u32,
    bytes: Vec<u8>,
}

impl RawCameraSample {
    /// Stores a raw buffer. `stride` is bytes per row and is ignored for MJPEG.
    #[must_use]
    pub fn new(
        format: CaptureFormat,
        width: u32,
        height: u32,
        stride: u32,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            format,
            width,
            height,
            stride,
            bytes,
        }
    }
}

/// Device boundary used by negotiation. Hardware types stay behind this trait.
pub trait ColorCamera {
    /// Profiles the device currently advertises.
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError>;

    /// Opens `profile` and returns the mode the device actually accepted.
    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError>;

    /// Returns the newest raw buffer. Does not decode pixels.
    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError>;

    /// Stops the stream.
    fn stop(&mut self) -> Result<(), A2aLabError>;
}

/// One negotiated capture session.
pub struct CaptureSession<C> {
    camera: C,
    request: CaptureProfileRequest,
    selected: Option<SelectedCaptureProfile>,
}

impl<C: ColorCamera> CaptureSession<C> {
    /// Stores the request. The camera stays closed until [`Self::open`].
    #[must_use]
    pub const fn new(camera: C, request: CaptureProfileRequest) -> Self {
        Self {
            camera,
            request,
            selected: None,
        }
    }

    /// The profile accepted by the device, once open.
    #[must_use]
    pub const fn selected(&self) -> Option<SelectedCaptureProfile> {
        self.selected
    }

    /// Selects an advertised profile and opens it.
    pub fn open(&mut self) -> Result<SelectedCaptureProfile, A2aLabError> {
        let advertised = self.camera.advertised()?;
        let selected = select_capture_profile(self.request, &advertised)
            .map_err(|error| profile_error(&error))?;
        let negotiated = self.camera.open(&selected)?;
        let accepted = negotiated.mode() == selected.mode()
            && negotiated.sensor_rate() == selected.sensor_rate();
        let confirmed =
            confirm_selected_profile(selected, accepted).map_err(|error| profile_error(&error))?;
        self.selected = Some(confirmed);
        Ok(confirmed)
    }

    /// Copies one raw frame. MJPEG stays JPEG and YUY2 stays packed.
    pub fn read(
        &mut self,
        identity: CaptureIdentity,
        timing: CaptureTiming,
        color: ColorProperties,
    ) -> Result<CapturedColorFrame, A2aLabError> {
        if self.selected.is_none() {
            return Err(A2aLabError::invalid("camera", "capture session is closed"));
        }
        captured_frame(&self.camera.raw_frame()?, identity, timing, color)
    }

    /// Stops the device and forgets the negotiated profile.
    pub fn stop(&mut self) -> Result<(), A2aLabError> {
        self.camera.stop()?;
        self.selected = None;
        Ok(())
    }

    /// Stops and opens the same request again.
    pub fn reconnect(&mut self) -> Result<SelectedCaptureProfile, A2aLabError> {
        self.stop()?;
        self.open()
    }
}

/// Copies a raw sample into the shared capture contract.
pub fn captured_frame(
    sample: &RawCameraSample,
    identity: CaptureIdentity,
    timing: CaptureTiming,
    color: ColorProperties,
) -> Result<CapturedColorFrame, A2aLabError> {
    match sample.format {
        CaptureFormat::Mjpeg => CapturedColorFrame::mjpeg(
            identity,
            timing,
            sample.width,
            sample.height,
            &sample.bytes,
            color,
        ),
        CaptureFormat::Yuy2 => {
            let row = sample
                .width
                .checked_mul(2)
                .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 row size overflow"))?;
            if sample.stride < row || sample.height == 0 {
                return Err(A2aLabError::invalid(
                    "stride",
                    "YUY2 stride does not cover the frame",
                ));
            }
            let needed = usize::try_from(sample.stride)
                .ok()
                .and_then(|stride| {
                    usize::try_from(sample.height)
                        .ok()
                        .map(|height| stride * height)
                })
                .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 buffer size overflow"))?;
            if sample.bytes.len() != needed {
                return Err(A2aLabError::invalid(
                    "stride",
                    "YUY2 buffer length does not match the padded stride",
                ));
            }
            CapturedColorFrame::yuy2(
                identity,
                timing,
                sample.width,
                sample.height,
                sample.stride as usize,
                &sample.bytes,
                color,
                None,
            )
        }
    }
}

fn profile_error(error: &crate::videos::CaptureProfileError) -> A2aLabError {
    A2aLabError::invalid("profile", format!("{error:?}"))
}
