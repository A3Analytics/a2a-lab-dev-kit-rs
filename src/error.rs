//! Stable errors returned by lab providers and protocol adapters.

use std::fmt;

/// Stable video availability failure.
///
/// Protocol adapters preserve [`Self::as_str`] without parsing the human message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoAvailability {
    /// The requested range contains no retained recording.
    NoRecording,
    /// The requested range crosses a gap between retained intervals.
    RecordingGap,
    /// A prepared clip handle has expired.
    ClipExpired,
    /// Live follow is not currently available for the source.
    LiveUnavailable,
}

impl VideoAvailability {
    /// Stable machine-readable reason.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoRecording => "no_recording",
            Self::RecordingGap => "recording_gap",
            Self::ClipExpired => "clip_expired",
            Self::LiveUnavailable => "live_unavailable",
        }
    }

    /// Parses a reason produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "no_recording" => Some(Self::NoRecording),
            "recording_gap" => Some(Self::RecordingGap),
            "clip_expired" => Some(Self::ClipExpired),
            "live_unavailable" => Some(Self::LiveUnavailable),
            _ => None,
        }
    }
}

impl fmt::Display for VideoAvailability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Failure returned by the lab dev kit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum A2aLabError {
    /// A caller-supplied value failed validation.
    #[error("invalid {field}: {message}")]
    Invalid {
        /// Name of the rejected field.
        field: &'static str,
        /// Human-readable explanation.
        message: String,
    },
    /// The requested resource does not exist.
    #[error("{kind} `{id}` was not found")]
    NotFound {
        /// Kind of resource, such as `log source` or `task run`.
        kind: &'static str,
        /// Identifier that was requested.
        id: String,
    },
    /// The provider could not serve the request.
    #[error("{message}")]
    Unavailable {
        /// Provider explanation.
        message: String,
    },
    /// The network transport failed.
    #[error("transport error: {message}")]
    Transport {
        /// Transport explanation.
        message: String,
    },
    /// A protocol payload did not match the expected contract.
    #[error("protocol error: {message}")]
    Protocol {
        /// Protocol explanation.
        message: String,
    },
    /// A video request failed for a stable availability reason.
    #[error("{availability}: {message}")]
    Video {
        /// Machine-readable availability reason.
        availability: VideoAvailability,
        /// Human-readable explanation.
        message: String,
    },
}

impl A2aLabError {
    /// Creates an input-validation error.
    #[must_use]
    pub fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            message: message.into(),
        }
    }

    /// Creates a missing-resource error.
    #[must_use]
    pub fn not_found(kind: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            id: id.into(),
        }
    }

    /// Creates a provider-unavailable error.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    /// Creates a transport error.
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
        }
    }

    /// Creates a protocol error.
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol {
            message: message.into(),
        }
    }

    /// Creates a video availability failure.
    #[must_use]
    pub fn video(availability: VideoAvailability, message: impl Into<String>) -> Self {
        Self::Video {
            availability,
            message: message.into(),
        }
    }

    /// The requested range contains no retained recording.
    #[must_use]
    pub fn no_recording(message: impl Into<String>) -> Self {
        Self::video(VideoAvailability::NoRecording, message)
    }

    /// The requested range crosses a gap between retained intervals.
    #[must_use]
    pub fn recording_gap(message: impl Into<String>) -> Self {
        Self::video(VideoAvailability::RecordingGap, message)
    }

    /// A prepared clip handle has expired.
    #[must_use]
    pub fn clip_expired(message: impl Into<String>) -> Self {
        Self::video(VideoAvailability::ClipExpired, message)
    }

    /// Live follow is not currently available.
    #[must_use]
    pub fn live_unavailable(message: impl Into<String>) -> Self {
        Self::video(VideoAvailability::LiveUnavailable, message)
    }

    /// Returns the video availability reason when this error is one.
    #[must_use]
    pub const fn availability(&self) -> Option<VideoAvailability> {
        match self {
            Self::Video { availability, .. } => Some(*availability),
            _ => None,
        }
    }

    /// Returns the stable machine-readable code.
    ///
    /// Video availability failures use [`VideoAvailability::as_str`] so adapters
    /// can preserve the reason without parsing [`Self`]'s display text.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Invalid { .. } => "invalid",
            Self::NotFound { .. } => "not_found",
            Self::Unavailable { .. } => "unavailable",
            Self::Transport { .. } => "transport",
            Self::Protocol { .. } => "protocol",
            Self::Video { availability, .. } => availability.as_str(),
        }
    }

    /// Parses a code produced by [`Self::code`].
    pub fn from_code(code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        if let Some(availability) = VideoAvailability::parse(code) {
            return Self::video(availability, message);
        }
        match code {
            "invalid" => Self::invalid("request", message),
            "not_found" => Self::not_found("resource", message),
            "unavailable" => Self::unavailable(message),
            "transport" => Self::transport(message),
            _ => Self::protocol(message),
        }
    }
}
