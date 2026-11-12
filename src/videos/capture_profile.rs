//! Portable selection of a capture format and sensor rate.
//!
//! Output rate and sensor rate are separate. Auto keeps an advertised
//! same-resolution MJPEG profile. An explicit raw request never drops to a
//! smaller resolution.

use crate::error::A2aLabError;
use crate::videos::capture::{CaptureFormat, CaptureMode};

/// Default delivery rate. Sensor rate is negotiated independently.
pub const DEFAULT_CAPTURE_OUTPUT_FPS: u32 = 5;

/// Caller preference for the raw capture format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureFormatPreference {
    /// Same-resolution MJPEG when advertised, otherwise same-resolution YUY2.
    Auto,
    /// Encoded JPEG access units only.
    Mjpeg,
    /// Packed YUY2/YUYV only.
    Yuy2,
}

/// Positive rational frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureRate {
    numerator: u32,
    denominator: u32,
}

impl CaptureRate {
    /// Accepts a positive rational. Zero and a zero denominator are rejected.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, A2aLabError> {
        if denominator == 0 {
            return Err(A2aLabError::invalid("rate", "denominator must be positive"));
        }
        if numerator == 0 {
            return Err(A2aLabError::invalid("rate", "numerator must be positive"));
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Whole frames per second.
    pub fn fps(frames: u32) -> Result<Self, A2aLabError> {
        Self::new(frames, 1)
    }

    /// Default 5 FPS output rate.
    #[must_use]
    pub fn output_default() -> Self {
        Self {
            numerator: DEFAULT_CAPTURE_OUTPUT_FPS,
            denominator: 1,
        }
    }

    /// Numerator of the rational rate.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Denominator of the rational rate.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    fn cmp_rate(self, other: Self) -> std::cmp::Ordering {
        let left = u64::from(self.numerator) * u64::from(other.denominator);
        let right = u64::from(other.numerator) * u64::from(self.denominator);
        left.cmp(&right)
    }
}

/// One device-advertised mode, including its sensor rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvertisedCaptureProfile {
    mode: CaptureMode,
    sensor_rate: CaptureRate,
}

impl AdvertisedCaptureProfile {
    /// Stores a validated mode and sensor rate.
    #[must_use]
    pub const fn new(mode: CaptureMode, sensor_rate: CaptureRate) -> Self {
        Self { mode, sensor_rate }
    }

    /// Advertised dimensions and raw format.
    #[must_use]
    pub const fn mode(self) -> CaptureMode {
        self.mode
    }

    /// Advertised sensor rate.
    #[must_use]
    pub const fn sensor_rate(self) -> CaptureRate {
        self.sensor_rate
    }
}

/// What the caller asked the policy to select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureProfileRequest {
    preference: CaptureFormatPreference,
    width: u32,
    height: u32,
    output_rate: CaptureRate,
}

impl CaptureProfileRequest {
    /// Requests exact dimensions. `output_rate` stays the delivery rate.
    pub fn new(
        preference: CaptureFormatPreference,
        width: u32,
        height: u32,
        output_rate: CaptureRate,
    ) -> Result<Self, A2aLabError> {
        if width == 0 {
            return Err(A2aLabError::invalid("width", "must be positive"));
        }
        if height == 0 {
            return Err(A2aLabError::invalid("height", "must be positive"));
        }
        if preference == CaptureFormatPreference::Yuy2 && !width.is_multiple_of(2) {
            return Err(A2aLabError::invalid(
                "width",
                "packed YUY2 width must be even",
            ));
        }
        Ok(Self {
            preference,
            width,
            height,
            output_rate,
        })
    }

    /// Auto preference at the default 5 FPS output rate.
    pub fn auto(width: u32, height: u32) -> Result<Self, A2aLabError> {
        Self::new(
            CaptureFormatPreference::Auto,
            width,
            height,
            CaptureRate::output_default(),
        )
    }

    /// Format preference.
    #[must_use]
    pub const fn preference(self) -> CaptureFormatPreference {
        self.preference
    }

    /// Requested width.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Requested height.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Delivery rate. This is not the sensor rate.
    #[must_use]
    pub const fn output_rate(self) -> CaptureRate {
        self.output_rate
    }
}

/// Chosen sensor profile plus the unchanged output rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedCaptureProfile {
    mode: CaptureMode,
    sensor_rate: CaptureRate,
    output_rate: CaptureRate,
}

impl SelectedCaptureProfile {
    /// Records a device-negotiated mode without running selection again.
    #[must_use]
    pub const fn negotiated(
        mode: CaptureMode,
        sensor_rate: CaptureRate,
        output_rate: CaptureRate,
    ) -> Self {
        Self {
            mode,
            sensor_rate,
            output_rate,
        }
    }

    /// Selected raw mode.
    #[must_use]
    pub const fn mode(self) -> CaptureMode {
        self.mode
    }

    /// Sensor rate the device is asked to run.
    #[must_use]
    pub const fn sensor_rate(self) -> CaptureRate {
        self.sensor_rate
    }

    /// Delivery rate, unchanged by sensor negotiation.
    #[must_use]
    pub const fn output_rate(self) -> CaptureRate {
        self.output_rate
    }
}

/// Why a profile could not be selected or accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureProfileError {
    /// No advertised profile can satisfy the request.
    NoSuitableProfile,
    /// The request is valid, but the device does not advertise that mode.
    Unsupported {
        /// Profiles the device did advertise.
        available: Vec<AdvertisedCaptureProfile>,
    },
    /// The device rejected a profile this policy had selected.
    DeviceRejected {
        /// Profile that was submitted to the device.
        profile: SelectedCaptureProfile,
    },
}

/// Selects one advertised profile without exact width/height matching.
pub fn select_capture_profile_flexible(
    preference: CaptureFormatPreference,
    output_rate: CaptureRate,
    advertised: &[AdvertisedCaptureProfile],
) -> Result<SelectedCaptureProfile, CaptureProfileError> {
    if advertised.is_empty() {
        return Err(CaptureProfileError::NoSuitableProfile);
    }
    let mut matches: Vec<(usize, AdvertisedCaptureProfile)> = advertised
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, profile)| format_allowed(preference, profile.mode.format()))
        .collect();
    if preference == CaptureFormatPreference::Auto {
        let mjpeg: Vec<_> = matches
            .iter()
            .copied()
            .filter(|(_, profile)| profile.mode.format() == CaptureFormat::Mjpeg)
            .collect();
        if !mjpeg.is_empty() {
            matches = mjpeg;
        }
    }
    if matches.is_empty() {
        return Err(CaptureProfileError::NoSuitableProfile);
    }
    let native = matches
        .iter()
        .copied()
        .filter(|(_, profile)| profile.sensor_rate.cmp_rate(output_rate).is_eq())
        .max_by_key(|(index, profile)| {
            let pixels = u64::from(profile.mode.width()) * u64::from(profile.mode.height());
            (pixels, usize::MAX - *index)
        });
    let chosen = native.or_else(|| {
        matches
            .iter()
            .copied()
            .filter(|(_, profile)| profile.sensor_rate.cmp_rate(output_rate).is_ge())
            .min_by(|(left_index, left), (right_index, right)| {
                left.sensor_rate
                    .cmp_rate(right.sensor_rate)
                    .then_with(|| {
                        let left_pixels =
                            u64::from(left.mode.width()) * u64::from(left.mode.height());
                        let right_pixels =
                            u64::from(right.mode.width()) * u64::from(right.mode.height());
                        right_pixels.cmp(&left_pixels)
                    })
                    .then(left_index.cmp(right_index))
            })
    });
    let Some((_, profile)) = chosen else {
        return Err(CaptureProfileError::NoSuitableProfile);
    };
    Ok(SelectedCaptureProfile {
        mode: profile.mode,
        sensor_rate: profile.sensor_rate,
        output_rate,
    })
}

/// Selects one advertised profile. Does not open a device.
pub fn select_capture_profile(
    request: CaptureProfileRequest,
    advertised: &[AdvertisedCaptureProfile],
) -> Result<SelectedCaptureProfile, CaptureProfileError> {
    if advertised.is_empty() {
        return Err(CaptureProfileError::NoSuitableProfile);
    }
    let mut matches: Vec<(usize, AdvertisedCaptureProfile)> = advertised
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, profile)| {
            profile.mode.width() == request.width
                && profile.mode.height() == request.height
                && format_allowed(request.preference, profile.mode.format())
        })
        .collect();
    if request.preference == CaptureFormatPreference::Auto {
        let mjpeg: Vec<_> = matches
            .iter()
            .copied()
            .filter(|(_, profile)| profile.mode.format() == CaptureFormat::Mjpeg)
            .collect();
        if !mjpeg.is_empty() {
            matches = mjpeg;
        }
    }
    if matches.is_empty() {
        return Err(CaptureProfileError::Unsupported {
            available: advertised.to_vec(),
        });
    }
    let native = matches
        .iter()
        .copied()
        .filter(|(_, profile)| profile.sensor_rate.cmp_rate(request.output_rate).is_eq())
        .min_by_key(|(index, _)| *index);
    let chosen = native.or_else(|| {
        matches
            .iter()
            .copied()
            .filter(|(_, profile)| profile.sensor_rate.cmp_rate(request.output_rate).is_ge())
            .min_by(|(left_index, left), (right_index, right)| {
                left.sensor_rate
                    .cmp_rate(right.sensor_rate)
                    .then(left_index.cmp(right_index))
            })
    });
    let Some((_, profile)) = chosen else {
        return Err(CaptureProfileError::NoSuitableProfile);
    };
    Ok(SelectedCaptureProfile {
        mode: profile.mode,
        sensor_rate: profile.sensor_rate,
        output_rate: request.output_rate,
    })
}

/// Records a device accept/reject without choosing a different profile.
pub fn confirm_selected_profile(
    profile: SelectedCaptureProfile,
    accepted: bool,
) -> Result<SelectedCaptureProfile, CaptureProfileError> {
    if accepted {
        Ok(profile)
    } else {
        Err(CaptureProfileError::DeviceRejected { profile })
    }
}

fn format_allowed(preference: CaptureFormatPreference, format: CaptureFormat) -> bool {
    match preference {
        CaptureFormatPreference::Auto => true,
        CaptureFormatPreference::Mjpeg => format == CaptureFormat::Mjpeg,
        CaptureFormatPreference::Yuy2 => format == CaptureFormat::Yuy2,
    }
}

/// Stable label for one raw capture format.
#[must_use]
pub fn format_capture_format_label(format: CaptureFormat) -> &'static str {
    match format {
        CaptureFormat::Mjpeg => "MJPEG",
        CaptureFormat::Yuy2 => "YUYV",
    }
}

/// Stable label for one profile preference.
#[must_use]
pub fn format_capture_preference_label(preference: CaptureFormatPreference) -> &'static str {
    match preference {
        CaptureFormatPreference::Auto => "auto",
        CaptureFormatPreference::Mjpeg => "MJPEG",
        CaptureFormatPreference::Yuy2 => "YUYV",
    }
}

/// One line describing the configured capture request.
#[must_use]
pub fn format_capture_profile_request(request: &CaptureProfileRequest) -> String {
    format!(
        "{} {}x{} output {}fps",
        format_capture_preference_label(request.preference()),
        request.width(),
        request.height(),
        request.output_rate().numerator()
    )
}

fn format_sensor_fps(rate: CaptureRate) -> String {
    if rate.denominator() == 1 {
        rate.numerator().to_string()
    } else {
        format!("{}/{}", rate.numerator(), rate.denominator())
    }
}

/// Sorted, semicolon-separated advertised modes for operator-facing diagnostics.
#[must_use]
pub fn format_advertised_capture_profiles(profiles: &[AdvertisedCaptureProfile]) -> String {
    if profiles.is_empty() {
        return "none".to_owned();
    }
    let mut lines = profiles
        .iter()
        .map(|profile| {
            format!(
                "{} {}x{}@{}fps",
                format_capture_format_label(profile.mode().format()),
                profile.mode().width(),
                profile.mode().height(),
                format_sensor_fps(profile.sensor_rate())
            )
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines.dedup();
    lines.join("; ")
}

/// Operator-facing text when the configured profile cannot be negotiated.
#[must_use]
pub fn describe_capture_profile_error(
    request: &CaptureProfileRequest,
    error: &CaptureProfileError,
    advertised: &[AdvertisedCaptureProfile],
) -> String {
    let available = format_advertised_capture_profiles(advertised);
    let configured = format_capture_profile_request(request);
    match error {
        CaptureProfileError::Unsupported { .. } | CaptureProfileError::NoSuitableProfile => {
            format!("configured profile {configured} is not advertised; available: {available}")
        }
        CaptureProfileError::DeviceRejected { profile } => {
            format!(
                "device rejected configured profile {configured}; negotiated {} {}x{}@{}fps; available: {available}",
                format_capture_format_label(profile.mode().format()),
                profile.mode().width(),
                profile.mode().height(),
                format_sensor_fps(profile.sensor_rate()),
            )
        }
    }
}

#[cfg(test)]
mod diagnostics_tests {
    use super::{
        AdvertisedCaptureProfile, CaptureFormat, CaptureFormatPreference, CaptureMode,
        CaptureProfileError, CaptureRate, describe_capture_profile_error,
        format_advertised_capture_profiles,
    };
    use crate::videos::CaptureProfileRequest;

    fn profile(
        format: CaptureFormat,
        width: u32,
        height: u32,
        fps: u32,
    ) -> AdvertisedCaptureProfile {
        AdvertisedCaptureProfile::new(
            CaptureMode::new(format, width, height).expect("mode"),
            CaptureRate::fps(fps).unwrap(),
        )
    }

    #[test]
    fn advertised_profile_list_is_sorted_and_deduped() {
        let advertised = [
            profile(CaptureFormat::Yuy2, 640, 480, 30),
            profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
            profile(CaptureFormat::Mjpeg, 1920, 1080, 30),
        ];
        let text = format_advertised_capture_profiles(&advertised);
        assert!(text.contains("MJPEG 1920x1080@30fps"));
        assert!(text.contains("YUYV 640x480@30fps"));
        assert_eq!(text.matches("MJPEG 1920x1080@30fps").count(), 1);
    }

    #[test]
    fn unsupported_request_lists_configured_and_available_profiles() {
        let advertised = [profile(CaptureFormat::Mjpeg, 1920, 1080, 30)];
        let request = CaptureProfileRequest::new(
            CaptureFormatPreference::Mjpeg,
            1280,
            720,
            CaptureRate::fps(5).unwrap(),
        )
        .unwrap();
        let message = describe_capture_profile_error(
            &request,
            &CaptureProfileError::Unsupported {
                available: advertised.to_vec(),
            },
            &advertised,
        );
        assert!(message.contains("configured profile MJPEG 1280x720 output 5fps"));
        assert!(message.contains("available: MJPEG 1920x1080@30fps"));
    }
}
