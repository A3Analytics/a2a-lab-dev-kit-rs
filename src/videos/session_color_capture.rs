//! [`ColorCaptureSource`] over one negotiated [`CaptureSession`].

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;
use crate::videos::{
    CaptureCapabilities, CaptureClock, CaptureFormat, CaptureFormatPreference, CaptureIdentity,
    CaptureProfileRequest, CaptureRate, CaptureSession, CaptureTiming, CapturedColorFrame,
    ColorCamera, ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange,
    SelectedCaptureProfile,
};

/// Drains one open capture session into the bounded runner.
pub struct SessionColorCapture<C: ColorCamera> {
    session: CaptureSession<C>,
    source_id: VideoSourceId,
    input_preference: CaptureFormatPreference,
    sequence: u64,
    capabilities: CaptureCapabilities,
}

impl<C: ColorCamera> SessionColorCapture<C> {
    /// Negotiates and opens `camera` for `request`.
    pub fn open(
        camera: C,
        request: CaptureProfileRequest,
        source_id: VideoSourceId,
        input_preference: CaptureFormatPreference,
    ) -> Result<Self, A2aLabError> {
        let mut session = CaptureSession::new(camera, request);
        session.open()?;
        let selected = session.selected().ok_or_else(|| {
            A2aLabError::invalid("camera", "capture session has no negotiated profile")
        })?;
        let capabilities = capabilities_from_selected(selected)?;
        Ok(Self {
            session,
            source_id,
            input_preference,
            sequence: 0,
            capabilities,
        })
    }

    /// Opens from an already negotiated session (tests and injection).
    #[must_use]
    pub fn from_session(
        session: CaptureSession<C>,
        source_id: VideoSourceId,
        input_preference: CaptureFormatPreference,
        capabilities: CaptureCapabilities,
    ) -> Self {
        Self {
            session,
            source_id,
            input_preference,
            sequence: 0,
            capabilities,
        }
    }

    /// Mutable access for reconnect and shutdown.
    pub fn session_mut(&mut self) -> &mut CaptureSession<C> {
        &mut self.session
    }
}

impl<C: ColorCamera> ColorCaptureSource for SessionColorCapture<C> {
    fn capabilities(&self) -> &CaptureCapabilities {
        &self.capabilities
    }

    fn pull_frame(&mut self) -> Result<CapturedColorFrame, A2aLabError> {
        let captured_at = UtcTimestamp::now();
        let identity = CaptureIdentity::new(self.source_id.clone(), self.sequence);
        self.sequence = self.sequence.saturating_add(1);
        let selected = self.session.selected().ok_or_else(|| {
            A2aLabError::invalid("camera", "capture session has no negotiated profile")
        })?;
        let sensor_period_ns = fps_period_ns(selected.capture_rate());
        let monotonic_ns = identity.sequence().saturating_mul(sensor_period_ns);
        let timing = CaptureTiming::new(captured_at, CaptureClock::monotonic(monotonic_ns));
        let negotiated = selected.mode().format();
        let color = color_properties(negotiated);
        let frame = self.session.read(identity, timing, color)?;
        if !format_matches(self.input_preference, negotiated) {
            return Err(A2aLabError::invalid(
                "format",
                "negotiated raw layout does not match the configured input format",
            ));
        }
        if frame.format() != negotiated {
            return Err(A2aLabError::invalid(
                "format",
                "raw sample layout does not match the negotiated input format",
            ));
        }
        Ok(frame)
    }
}

fn capabilities_from_selected(
    selected: SelectedCaptureProfile,
) -> Result<CaptureCapabilities, A2aLabError> {
    CaptureCapabilities::new(vec![selected.mode()])
}

fn fps_period_ns(rate: CaptureRate) -> u64 {
    let fps = u64::from(rate.numerator());
    let denom = u64::from(rate.denominator()).max(1);
    1_000_000_000u64
        .saturating_mul(denom)
        .saturating_div(fps.max(1))
}

fn color_properties(negotiated: CaptureFormat) -> ColorProperties {
    match negotiated {
        CaptureFormat::Yuy2 => ColorProperties::new(ColorMatrix::Bt601, ColorRange::Limited),
        CaptureFormat::Mjpeg => ColorProperties::new(ColorMatrix::Unknown, ColorRange::Unknown),
    }
}

fn format_matches(preference: CaptureFormatPreference, negotiated: CaptureFormat) -> bool {
    match preference {
        CaptureFormatPreference::Mjpeg => negotiated == CaptureFormat::Mjpeg,
        CaptureFormatPreference::Yuy2 => negotiated == CaptureFormat::Yuy2,
        CaptureFormatPreference::Auto => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiated_yuy2_supplies_encoder_color_metadata() {
        let color = color_properties(CaptureFormat::Yuy2);
        assert_eq!(color.matrix(), ColorMatrix::Bt601);
        assert_eq!(color.range(), ColorRange::Limited);
    }

    #[test]
    fn negotiated_mjpeg_keeps_unspecified_color_metadata() {
        let color = color_properties(CaptureFormat::Mjpeg);
        assert_eq!(color.matrix(), ColorMatrix::Unknown);
        assert_eq!(color.range(), ColorRange::Unknown);
    }
}
