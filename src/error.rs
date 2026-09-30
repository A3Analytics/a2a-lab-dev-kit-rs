//! Stable errors returned by lab providers and protocol adapters.

/// Failure returned by the lab SDK.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SdkError {
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
}

impl SdkError {
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

    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Invalid { .. } => "invalid",
            Self::NotFound { .. } => "not_found",
            Self::Unavailable { .. } => "unavailable",
            Self::Transport { .. } => "transport",
            Self::Protocol { .. } => "protocol",
        }
    }

    /// Parses a code produced by [`Self::code`].
    pub fn from_code(code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        match code {
            "invalid" => Self::invalid("request", message),
            "not_found" => Self::not_found("resource", message),
            "unavailable" => Self::unavailable(message),
            "transport" => Self::transport(message),
            _ => Self::protocol(message),
        }
    }
}
