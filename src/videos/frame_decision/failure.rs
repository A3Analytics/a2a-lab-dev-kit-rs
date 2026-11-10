//! Typed failures for Jev-only frame decision evaluator boundaries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Failure reported by frame decision evaluators and routers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "reason", content = "detail")]
pub enum FrameDecisionEvaluationFailure {
    /// A configured bound was exceeded before or during evaluation.
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
    /// Jev or the configured worker could not serve the request.
    ProviderUnavailable {
        /// Provider detail.
        message: String,
    },
    /// The request references a predicate this evaluator does not support.
    UnsupportedPredicate {
        /// Predicate id.
        predicate_id: String,
        /// Schema version.
        predicate_version: u32,
    },
    /// Evidence or criteria failed Jev-only validation.
    InvalidEvidence {
        /// Detail.
        message: String,
    },
    /// A disallowed non-image decision input was supplied.
    ForbiddenInput {
        /// Stable forbidden kind.
        kind: String,
        /// Detail.
        message: String,
    },
    /// Jev returned a response that failed provenance or structured validation.
    InvalidProvenance {
        /// Detail.
        message: String,
    },
    /// The evaluator returned fewer keyed results than requested.
    PartialResponse {
        /// Detail.
        message: String,
    },
    /// Response key does not match the requested evaluation identity.
    KeyMismatch {
        /// Detail.
        message: String,
    },
}

impl FrameDecisionEvaluationFailure {
    /// Stable machine-readable reason.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LimitExceeded { .. } => "limit_exceeded",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::UnsupportedPredicate { .. } => "unsupported_predicate",
            Self::InvalidEvidence { .. } => "invalid_evidence",
            Self::ForbiddenInput { .. } => "forbidden_input",
            Self::InvalidProvenance { .. } => "invalid_provenance",
            Self::PartialResponse { .. } => "partial_response",
            Self::KeyMismatch { .. } => "key_mismatch",
        }
    }

    /// Maps to a lab error for adapters that do not surface this enum directly.
    #[must_use]
    pub fn into_lab_error(self) -> A2aLabError {
        let message = self.to_string();
        match &self {
            Self::LimitExceeded { limit, .. } => {
                A2aLabError::invalid("frame_decision", format!("{limit}: {message}"))
            }
            Self::DeadlineExceeded | Self::Cancelled | Self::ProviderUnavailable { .. } => {
                A2aLabError::unavailable(message)
            }
            Self::UnsupportedPredicate { .. }
            | Self::InvalidEvidence { .. }
            | Self::ForbiddenInput { .. }
            | Self::InvalidProvenance { .. }
            | Self::PartialResponse { .. }
            | Self::KeyMismatch { .. } => A2aLabError::protocol(message),
        }
    }
}

impl fmt::Display for FrameDecisionEvaluationFailure {
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
            Self::UnsupportedPredicate {
                predicate_id,
                predicate_version,
            } => write!(
                formatter,
                "unsupported_predicate: {predicate_id} v{predicate_version}"
            ),
            Self::InvalidEvidence { message } => write!(formatter, "invalid_evidence: {message}"),
            Self::ForbiddenInput { kind, message } => {
                write!(formatter, "forbidden_input ({kind}): {message}")
            }
            Self::InvalidProvenance { message } => {
                write!(formatter, "invalid_provenance: {message}")
            }
            Self::PartialResponse { message } => write!(formatter, "partial_response: {message}"),
            Self::KeyMismatch { message } => write!(formatter, "key_mismatch: {message}"),
        }
    }
}

impl std::error::Error for FrameDecisionEvaluationFailure {}
