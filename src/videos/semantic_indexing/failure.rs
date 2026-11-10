//! Typed failures for semantic indexing provider boundaries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Failure reported by frame-description or text-embedding providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "reason", content = "detail")]
pub enum SemanticIndexingFailure {
    /// A configured bound was exceeded before or during provider work.
    LimitExceeded {
        /// Which limit tripped.
        limit: String,
        /// Human-readable detail.
        message: String,
    },
    /// The caller or provider deadline elapsed.
    DeadlineExceeded,
    /// The caller cancelled the operation.
    Cancelled,
    /// The configured provider could not serve the request.
    ProviderUnavailable {
        /// Provider detail.
        message: String,
    },
    /// A response key does not match the requested frame revision or ROI.
    KeyMismatch {
        /// Detail including frame ids when available.
        message: String,
    },
    /// The provider returned fewer keyed results than requested.
    PartialResponse {
        /// Detail.
        message: String,
    },
    /// Description text was empty after trimming.
    EmptyDescription,
    /// Description or vector output exceeded configured limits.
    OversizedOutput {
        /// Detail.
        message: String,
    },
    /// Embedding provenance does not match the configured model space.
    IncompatibleModelSpace {
        /// Detail.
        message: String,
    },
    /// Stored vector values violate dimension or metric policy.
    InvalidVector {
        /// Detail.
        message: String,
    },
}

impl SemanticIndexingFailure {
    /// Stable machine-readable reason.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LimitExceeded { .. } => "limit_exceeded",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::KeyMismatch { .. } => "key_mismatch",
            Self::PartialResponse { .. } => "partial_response",
            Self::EmptyDescription => "empty_description",
            Self::OversizedOutput { .. } => "oversized_output",
            Self::IncompatibleModelSpace { .. } => "incompatible_model_space",
            Self::InvalidVector { .. } => "invalid_vector",
        }
    }

    /// Maps to a lab error for adapters that do not surface this enum directly.
    #[must_use]
    pub fn into_lab_error(self) -> A2aLabError {
        let message = self.to_string();
        match &self {
            Self::LimitExceeded { limit, .. } => {
                A2aLabError::invalid("semantic_indexing", format!("{limit}: {message}"))
            }
            Self::DeadlineExceeded | Self::Cancelled | Self::ProviderUnavailable { .. } => {
                A2aLabError::unavailable(message)
            }
            Self::KeyMismatch { .. }
            | Self::PartialResponse { .. }
            | Self::EmptyDescription
            | Self::OversizedOutput { .. }
            | Self::IncompatibleModelSpace { .. }
            | Self::InvalidVector { .. } => A2aLabError::protocol(message),
        }
    }
}

impl fmt::Display for SemanticIndexingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { limit, message } => {
                write!(formatter, "limit_exceeded ({limit}): {message}")
            }
            Self::DeadlineExceeded => formatter.write_str("deadline_exceeded"),
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::ProviderUnavailable { message } => {
                write!(formatter, "provider_unavailable: {message}")
            }
            Self::KeyMismatch { message } => write!(formatter, "key_mismatch: {message}"),
            Self::PartialResponse { message } => write!(formatter, "partial_response: {message}"),
            Self::EmptyDescription => formatter.write_str("empty_description"),
            Self::OversizedOutput { message } => write!(formatter, "oversized_output: {message}"),
            Self::IncompatibleModelSpace { message } => {
                write!(formatter, "incompatible_model_space: {message}")
            }
            Self::InvalidVector { message } => write!(formatter, "invalid_vector: {message}"),
        }
    }
}

impl std::error::Error for SemanticIndexingFailure {}
