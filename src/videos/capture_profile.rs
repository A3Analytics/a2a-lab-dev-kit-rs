//! Portable selection of a capture format and capture rate.
//!
//! Output rate and capture rate are separate. Omitted size or capture rate
//! selects the largest advertised resolution at or below 1920×1080, then the
//! lowest capture rate that still meets the live output cadence. Explicit
//! width and height may request 2K/4K. Auto keeps advertised MJPEG when
//! present. An explicit raw request never drops to a smaller resolution.

use crate::error::A2aLabError;
use crate::videos::capture::{CaptureFormat, CaptureMode};

/// Default delivery rate. Capture rate is negotiated independently.
pub const DEFAULT_CAPTURE_OUTPUT_FPS: u32 = 5;

/// Unpinned capture stays at or below this width. Explicit `width` may exceed it.
pub const DEFAULT_CAPTURE_MAX_WIDTH: u32 = 1920;

/// Unpinned capture stays at or below this height. Explicit `height` may exceed it.
pub const DEFAULT_CAPTURE_MAX_HEIGHT: u32 = 1080;

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

    /// Nominal spacing between capture frames in nanoseconds.
    pub fn period_ns(self) -> Result<u64, A2aLabError> {
        let numerator = u64::from(self.numerator);
        let denominator = u64::from(self.denominator);
        denominator
            .checked_mul(1_000_000_000)
            .and_then(|nanos| nanos.checked_div(numerator))
            .ok_or_else(|| {
                A2aLabError::invalid("rate", "capture period does not fit in nanoseconds")
            })
    }

    fn cmp_rate(self, other: Self) -> std::cmp::Ordering {
        let left = u64::from(self.numerator) * u64::from(other.denominator);
        let right = u64::from(other.numerator) * u64::from(self.denominator);
        left.cmp(&right)
    }
}

/// One device-advertised mode, including its capture rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvertisedCaptureProfile {
    mode: CaptureMode,
    capture_rate: CaptureRate,
}

impl AdvertisedCaptureProfile {
    /// Stores a validated mode and capture rate.
    #[must_use]
    pub const fn new(mode: CaptureMode, capture_rate: CaptureRate) -> Self {
        Self { mode, capture_rate }
    }

    /// Advertised dimensions and raw format.
    #[must_use]
    pub const fn mode(self) -> CaptureMode {
        self.mode
    }

    /// Advertised capture rate.
    #[must_use]
    pub const fn capture_rate(self) -> CaptureRate {
        self.capture_rate
    }
}

/// What the caller asked the policy to select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureProfileRequest {
    preference: CaptureFormatPreference,
    width: Option<u32>,
    height: Option<u32>,
    output_rate: CaptureRate,
    capture_rate: Option<CaptureRate>,
}

impl CaptureProfileRequest {
    /// Requests optional dimensions and capture rate. `output_rate` stays the delivery rate.
    pub fn new(
        preference: CaptureFormatPreference,
        width: Option<u32>,
        height: Option<u32>,
        output_rate: CaptureRate,
        capture_rate: Option<CaptureRate>,
    ) -> Result<Self, A2aLabError> {
        match (width, height) {
            (None, None) | (Some(_), Some(_)) => {}
            (Some(_), None) => {
                return Err(A2aLabError::invalid(
                    "height",
                    "required when videos.sources.width is set",
                ));
            }
            (None, Some(_)) => {
                return Err(A2aLabError::invalid(
                    "width",
                    "required when videos.sources.height is set",
                ));
            }
        }
        if let Some(0) = width {
            return Err(A2aLabError::invalid("width", "must be positive"));
        }
        if let Some(0) = height {
            return Err(A2aLabError::invalid("height", "must be positive"));
        }
        if preference == CaptureFormatPreference::Yuy2
            && let Some(width) = width
            && !width.is_multiple_of(2)
        {
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
            capture_rate,
        })
    }

    /// Exact dimensions and capture rate.
    pub fn exact(
        preference: CaptureFormatPreference,
        width: u32,
        height: u32,
        output_rate: CaptureRate,
        capture_rate: CaptureRate,
    ) -> Result<Self, A2aLabError> {
        Self::new(
            preference,
            Some(width),
            Some(height),
            output_rate,
            Some(capture_rate),
        )
    }

    /// Auto preference at the default 5 FPS output. Capture rate stays unpinned.
    pub fn auto(width: u32, height: u32) -> Result<Self, A2aLabError> {
        Self::new(
            CaptureFormatPreference::Auto,
            Some(width),
            Some(height),
            CaptureRate::output_default(),
            None,
        )
    }

    /// Format preference only. Size and capture rate stay unpinned.
    #[must_use]
    pub const fn unconstrained(
        preference: CaptureFormatPreference,
        output_rate: CaptureRate,
    ) -> Self {
        Self {
            preference,
            width: None,
            height: None,
            output_rate,
            capture_rate: None,
        }
    }

    /// Format preference.
    #[must_use]
    pub const fn preference(self) -> CaptureFormatPreference {
        self.preference
    }

    /// Requested width when the caller pinned one.
    #[must_use]
    pub const fn width(self) -> Option<u32> {
        self.width
    }

    /// Requested height when the caller pinned one.
    #[must_use]
    pub const fn height(self) -> Option<u32> {
        self.height
    }

    /// Delivery rate. This is not the capture rate.
    #[must_use]
    pub const fn output_rate(self) -> CaptureRate {
        self.output_rate
    }

    /// Exact capture rate when the caller pinned one.
    #[must_use]
    pub const fn capture_rate(self) -> Option<CaptureRate> {
        self.capture_rate
    }
}

/// Chosen capture profile plus the unchanged output rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedCaptureProfile {
    mode: CaptureMode,
    capture_rate: CaptureRate,
    output_rate: CaptureRate,
}

impl SelectedCaptureProfile {
    /// Records a device-negotiated mode without running selection again.
    #[must_use]
    pub const fn negotiated(
        mode: CaptureMode,
        capture_rate: CaptureRate,
        output_rate: CaptureRate,
    ) -> Self {
        Self {
            mode,
            capture_rate,
            output_rate,
        }
    }

    /// Selected raw mode.
    #[must_use]
    pub const fn mode(self) -> CaptureMode {
        self.mode
    }

    /// Capture rate the device is asked to run.
    #[must_use]
    pub const fn capture_rate(self) -> CaptureRate {
        self.capture_rate
    }

    /// Delivery rate, unchanged by capture negotiation.
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
    select_capture_profile(
        CaptureProfileRequest::unconstrained(preference, output_rate),
        advertised,
    )
}

/// Advertised profiles that satisfy `request`, largest resolution then lowest capture rate.
pub fn rank_capture_profiles(
    request: CaptureProfileRequest,
    advertised: &[AdvertisedCaptureProfile],
) -> Result<Vec<SelectedCaptureProfile>, CaptureProfileError> {
    if advertised.is_empty() {
        return Err(CaptureProfileError::NoSuitableProfile);
    }
    let mut matches: Vec<(usize, AdvertisedCaptureProfile)> = advertised
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, profile)| {
            size_allowed(request, profile.mode)
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
        return Err(unmatched(request, advertised));
    }
    matches.retain(|(_, profile)| rate_allowed(request, profile.capture_rate));
    if matches.is_empty() {
        return Err(unmatched(request, advertised));
    }
    matches.sort_by(|(left_index, left), (right_index, right)| {
        let left_pixels = u64::from(left.mode.width()) * u64::from(left.mode.height());
        let right_pixels = u64::from(right.mode.width()) * u64::from(right.mode.height());
        right_pixels
            .cmp(&left_pixels)
            .then_with(|| left.capture_rate.cmp_rate(right.capture_rate))
            .then(left_index.cmp(right_index))
    });
    Ok(matches
        .into_iter()
        .map(|(_, profile)| SelectedCaptureProfile {
            mode: profile.mode,
            capture_rate: profile.capture_rate,
            output_rate: request.output_rate,
        })
        .collect())
}

/// Selects one advertised profile. Does not open a device.
pub fn select_capture_profile(
    request: CaptureProfileRequest,
    advertised: &[AdvertisedCaptureProfile],
) -> Result<SelectedCaptureProfile, CaptureProfileError> {
    let ranked = rank_capture_profiles(request, advertised)?;
    Ok(ranked[0])
}

fn size_allowed(request: CaptureProfileRequest, mode: CaptureMode) -> bool {
    match (request.width, request.height) {
        (Some(width), Some(height)) => mode.width() == width && mode.height() == height,
        (None, None) => {
            mode.width() <= DEFAULT_CAPTURE_MAX_WIDTH && mode.height() <= DEFAULT_CAPTURE_MAX_HEIGHT
        }
        _ => false,
    }
}

fn rate_allowed(request: CaptureProfileRequest, advertised: CaptureRate) -> bool {
    match request.capture_rate {
        Some(pinned) => advertised.cmp_rate(pinned).is_eq(),
        None => advertised.cmp_rate(request.output_rate).is_ge(),
    }
}

fn unmatched(
    request: CaptureProfileRequest,
    advertised: &[AdvertisedCaptureProfile],
) -> CaptureProfileError {
    if request.width.is_none()
        && request.height.is_none()
        && request.capture_rate.is_none()
        && request.preference == CaptureFormatPreference::Auto
    {
        CaptureProfileError::NoSuitableProfile
    } else {
        CaptureProfileError::Unsupported {
            available: advertised.to_vec(),
        }
    }
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
    let size = match (request.width(), request.height()) {
        (Some(width), Some(height)) => format!("{width}x{height}"),
        _ => "max".to_owned(),
    };
    let capture = request
        .capture_rate()
        .map_or_else(|| "min".to_owned(), format_capture_fps);
    format!(
        "{} {size}@{capture}fps output {}fps",
        format_capture_preference_label(request.preference()),
        request.output_rate().numerator()
    )
}

fn format_capture_fps(rate: CaptureRate) -> String {
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
                format_capture_fps(profile.capture_rate())
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
                format_capture_fps(profile.capture_rate()),
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
        let request = CaptureProfileRequest::exact(
            CaptureFormatPreference::Mjpeg,
            1280,
            720,
            CaptureRate::fps(5).unwrap(),
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
        assert!(message.contains("configured profile MJPEG 1280x720@5fps output 5fps"));
        assert!(message.contains("available: MJPEG 1920x1080@30fps"));
    }
}
