//! Jev-only evaluator capabilities and production registration.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::videos::digest::Sha256Digest;
use crate::videos::frame_decision::predicate::LabPredicate;
use crate::videos::frame_decision::record::{
    DEFAULT_JEV_MODEL_REVISION, JEV_PRODUCER_ID, JevProducerIdentity,
};

/// Sole authorized production decision evaluator provider id.
pub const JEV_EVALUATOR_PROVIDER_ID: &str = "jev";

/// Minimum image edge per TypeSafe Jev image bounds.
pub const JEV_MIN_EDGE_PIXELS: u32 = 280;

/// Maximum image edge per TypeSafe Jev image bounds.
pub const JEV_MAX_EDGE_PIXELS: u32 = 1568;

/// Per-image byte cap for Jev requests.
pub const JEV_MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Expanded request byte cap for Jev image batches.
pub const JEV_MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

/// Disallowed evaluator backends that must not register as decision producers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RejectedDecisionEvaluatorBackend {
    /// Luna or other description model.
    LunaDescription,
    /// Bedrock chat or vision model.
    BedrockLlm,
    /// OpenAI chat or vision model.
    OpenaiLlm,
    /// Another generic LLM/VLM decision adapter.
    GenericLlm,
    /// Embedding retrieval encoder.
    EmbeddingRetrieval,
}

impl RejectedDecisionEvaluatorBackend {
    /// Stable rejection label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LunaDescription => "luna_description",
            Self::BedrockLlm => "bedrock_llm",
            Self::OpenaiLlm => "openai_llm",
            Self::GenericLlm => "generic_llm",
            Self::EmbeddingRetrieval => "embedding_retrieval",
        }
    }
}

/// Rejects LLM/description backends at evaluator registration.
pub fn reject_decision_evaluator_backend(
    backend: RejectedDecisionEvaluatorBackend,
) -> Result<(), A2aLabError> {
    let message = match backend {
        RejectedDecisionEvaluatorBackend::LunaDescription => {
            "Luna descriptions cannot register as frame decision evaluators"
        }
        RejectedDecisionEvaluatorBackend::BedrockLlm => {
            "Bedrock LLM models cannot register as frame decision evaluators"
        }
        RejectedDecisionEvaluatorBackend::OpenaiLlm => {
            "OpenAI LLM models cannot register as frame decision evaluators"
        }
        RejectedDecisionEvaluatorBackend::GenericLlm => {
            "only TypeSafe Jev may register as a frame decision evaluator"
        }
        RejectedDecisionEvaluatorBackend::EmbeddingRetrieval => {
            "embedding retrieval cannot register as a frame decision evaluator"
        }
    };
    Err(A2aLabError::invalid("decision_evaluator", message))
}

/// Resolved Jev evaluator settings independent from description and chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionEvaluatorProviderConfig {
    producer: JevProducerIdentity,
    config_digest: Sha256Digest,
    threshold_policy_digest: Sha256Digest,
    supported_predicates: Vec<LabPredicate>,
}

impl FrameDecisionEvaluatorProviderConfig {
    /// Builds Jev-only production evaluator config.
    pub fn new(
        model_revision: impl AsRef<str>,
        config_digest: Sha256Digest,
        threshold_policy_digest: Sha256Digest,
        supported_predicates: Vec<LabPredicate>,
    ) -> Result<Self, A2aLabError> {
        if supported_predicates.is_empty() {
            return Err(A2aLabError::invalid(
                "supported_predicates",
                "at least one lab predicate is required",
            ));
        }
        Ok(Self {
            producer: JevProducerIdentity::new(model_revision)?,
            config_digest,
            threshold_policy_digest,
            supported_predicates,
        })
    }

    /// Pilot defaults with the three initial lab predicates pinned at v1.
    pub fn pilot(
        config_digest: Sha256Digest,
        threshold_policy_digest: Sha256Digest,
    ) -> Result<Self, A2aLabError> {
        Self::new(
            DEFAULT_JEV_MODEL_REVISION,
            config_digest,
            threshold_policy_digest,
            vec![
                LabPredicate::pipette_tip_attached_v1(),
                LabPredicate::foreign_object_present_v1(),
                LabPredicate::plate_misaligned_v1(),
            ],
        )
    }

    /// Authorized Jev producer identity.
    #[must_use]
    pub const fn producer(&self) -> &JevProducerIdentity {
        &self.producer
    }

    /// Runtime configuration digest.
    #[must_use]
    pub const fn config_digest(&self) -> &Sha256Digest {
        &self.config_digest
    }

    /// Threshold interpretation policy digest.
    #[must_use]
    pub const fn threshold_policy_digest(&self) -> &Sha256Digest {
        &self.threshold_policy_digest
    }

    /// Predicates this evaluator may evaluate in production.
    #[must_use]
    pub fn supported_predicates(&self) -> &[LabPredicate] {
        &self.supported_predicates
    }

    /// Stable provider id exposed to configuration layers.
    #[must_use]
    pub const fn provider_id(&self) -> &'static str {
        JEV_EVALUATOR_PROVIDER_ID
    }

    /// Whether a predicate is supported by this evaluator.
    #[must_use]
    pub fn supports_predicate(&self, predicate: &LabPredicate) -> bool {
        self.supported_predicates
            .iter()
            .any(|entry| entry == predicate)
    }
}

/// Finite bounds for Jev decision evaluation requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionEvaluatorLimits {
    max_batch_items: u32,
    max_evidence_items: u32,
    max_reference_images: u32,
    max_history_frames: u32,
    min_edge_pixels: u32,
    max_edge_pixels: u32,
    max_image_bytes: usize,
    max_request_bytes: usize,
    max_jev_response_bytes: usize,
    deadline: Duration,
}

impl FrameDecisionEvaluatorLimits {
    /// Validates evaluator limits.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_batch_items: u32,
        max_evidence_items: u32,
        max_reference_images: u32,
        max_history_frames: u32,
        min_edge_pixels: u32,
        max_edge_pixels: u32,
        max_image_bytes: usize,
        max_request_bytes: usize,
        max_jev_response_bytes: usize,
        deadline: Duration,
    ) -> Result<Self, A2aLabError> {
        if max_batch_items == 0 || max_evidence_items == 0 {
            return Err(A2aLabError::invalid(
                "max_items",
                "batch and evidence limits must be positive",
            ));
        }
        if min_edge_pixels == 0 || max_edge_pixels < min_edge_pixels {
            return Err(A2aLabError::invalid(
                "edge_pixels",
                "invalid Jev image edge bounds",
            ));
        }
        if max_image_bytes == 0 || max_request_bytes == 0 || max_jev_response_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        if deadline.is_zero() {
            return Err(A2aLabError::invalid("deadline", "must be positive"));
        }
        Ok(Self {
            max_batch_items,
            max_evidence_items,
            max_reference_images,
            max_history_frames,
            min_edge_pixels,
            max_edge_pixels,
            max_image_bytes,
            max_request_bytes,
            max_jev_response_bytes,
            deadline,
        })
    }

    /// Conservative defaults aligned with published Jev image bounds.
    #[must_use]
    pub fn pilot() -> Self {
        Self {
            max_batch_items: 8,
            max_evidence_items: 6,
            max_reference_images: 4,
            max_history_frames: 4,
            min_edge_pixels: JEV_MIN_EDGE_PIXELS,
            max_edge_pixels: JEV_MAX_EDGE_PIXELS,
            max_image_bytes: JEV_MAX_IMAGE_BYTES,
            max_request_bytes: JEV_MAX_REQUEST_BYTES,
            max_jev_response_bytes: 256 * 1024,
            deadline: Duration::from_secs(45),
        }
    }

    /// Maximum evaluations per batch call.
    #[must_use]
    pub const fn max_batch_items(self) -> u32 {
        self.max_batch_items
    }

    /// Maximum evidence items on one evaluation.
    #[must_use]
    pub const fn max_evidence_items(self) -> u32 {
        self.max_evidence_items
    }

    /// Maximum operator reference images per evaluation.
    #[must_use]
    pub const fn max_reference_images(self) -> u32 {
        self.max_reference_images
    }

    /// Maximum bounded prior-frame history items.
    #[must_use]
    pub const fn max_history_frames(self) -> u32 {
        self.max_history_frames
    }

    /// Minimum transformed edge length.
    #[must_use]
    pub const fn min_edge_pixels(self) -> u32 {
        self.min_edge_pixels
    }

    /// Maximum transformed edge length.
    #[must_use]
    pub const fn max_edge_pixels(self) -> u32 {
        self.max_edge_pixels
    }

    /// Maximum bytes for one image asset.
    #[must_use]
    pub const fn max_image_bytes(self) -> usize {
        self.max_image_bytes
    }

    /// Maximum bytes for one Jev request payload.
    #[must_use]
    pub const fn max_request_bytes(self) -> usize {
        self.max_request_bytes
    }

    /// Maximum raw Jev response bytes retained for provenance.
    #[must_use]
    pub const fn max_jev_response_bytes(self) -> usize {
        self.max_jev_response_bytes
    }

    /// Per-batch deadline.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }
}

/// Production capabilities advertised by a registered Jev evaluator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionEvaluatorCapabilities {
    provider_id: String,
    producer_id: String,
    model_revision: String,
    supported_predicates: Vec<LabPredicate>,
    limits: FrameDecisionEvaluatorLimits,
}

impl FrameDecisionEvaluatorCapabilities {
    /// Builds capabilities from config and limits.
    #[must_use]
    pub fn from_config(
        config: &FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
    ) -> Self {
        Self {
            provider_id: JEV_EVALUATOR_PROVIDER_ID.to_owned(),
            producer_id: JEV_PRODUCER_ID.to_owned(),
            model_revision: config.producer().model_revision().to_owned(),
            supported_predicates: config.supported_predicates().to_vec(),
            limits,
        }
    }

    /// Authorized evaluator provider id (`jev` only).
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Authorized Jev producer id.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Pinned Jev model revision.
    #[must_use]
    pub fn model_revision(&self) -> &str {
        &self.model_revision
    }

    /// Supported predicate declarations.
    #[must_use]
    pub fn supported_predicates(&self) -> &[LabPredicate] {
        &self.supported_predicates
    }

    /// Active request/output limits.
    #[must_use]
    pub const fn limits(&self) -> FrameDecisionEvaluatorLimits {
        self.limits
    }
}

/// Parses an external provider id and rejects non-Jev backends.
pub fn parse_decision_evaluator_provider(
    provider: impl AsRef<str>,
    model: impl AsRef<str>,
) -> Result<FrameDecisionEvaluatorProviderConfig, A2aLabError> {
    let provider = provider.as_ref().trim();
    let model = model.as_ref().trim();
    if provider == "bedrock" || model.contains("luna") {
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::BedrockLlm)?;
    }
    if provider == "openai" || model.starts_with("gpt-") {
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::OpenaiLlm)?;
    }
    if provider == "description" || provider == "luna" {
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::LunaDescription)?;
    }
    if provider == "embedding" || provider == "retrieval" {
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::EmbeddingRetrieval)?;
    }
    if provider != JEV_EVALUATOR_PROVIDER_ID {
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::GenericLlm)?;
    }
    FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"eval-config-unset"),
        Sha256Digest::hash_bytes(b"threshold-unset"),
    )
    .and_then(|mut config| {
        if model != DEFAULT_JEV_MODEL_REVISION && !model.is_empty() {
            config = FrameDecisionEvaluatorProviderConfig::new(
                model,
                config.config_digest().clone(),
                config.threshold_policy_digest().clone(),
                config.supported_predicates().to_vec(),
            )?;
        }
        Ok(config)
    })
}

/// Registered production Jev evaluator metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFrameDecisionEvaluator {
    capabilities: FrameDecisionEvaluatorCapabilities,
}

impl RegisteredFrameDecisionEvaluator {
    /// Registers a Jev evaluator after validating config and limits.
    pub fn register(
        config: &FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
    ) -> Result<Self, A2aLabError> {
        config.producer().validate()?;
        Ok(Self {
            capabilities: FrameDecisionEvaluatorCapabilities::from_config(config, limits),
        })
    }

    /// Advertised capabilities for runners and configuration.
    #[must_use]
    pub const fn capabilities(&self) -> &FrameDecisionEvaluatorCapabilities {
        &self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::videos::frame_decision::predicate::{
        PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED,
        PREDICATE_PLATE_MISALIGNED,
    };

    #[test]
    fn initial_predicate_ids_match_pilot_capabilities() {
        let config = FrameDecisionEvaluatorProviderConfig::pilot(
            Sha256Digest::hash_bytes(b"c"),
            Sha256Digest::hash_bytes(b"t"),
        )
        .unwrap();
        let ids: Vec<_> = config
            .supported_predicates()
            .iter()
            .map(LabPredicate::id)
            .collect();
        assert!(ids.contains(&PREDICATE_PIPETTE_TIP_ATTACHED));
        assert!(ids.contains(&PREDICATE_FOREIGN_OBJECT_PRESENT));
        assert!(ids.contains(&PREDICATE_PLATE_MISALIGNED));
    }
}
