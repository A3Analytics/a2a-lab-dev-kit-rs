//! Bounded Jev evaluation requests with image-only evidence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::{DecisionIdempotencyKey, JevEvaluationId};
use crate::videos::digest::Sha256Digest;
use crate::videos::frame_decision::config::FrameDecisionEvaluatorLimits;
use crate::videos::frame_decision::failure::FrameDecisionEvaluationFailure;
use crate::videos::frame_decision::predicate::{
    DecisionEvidenceItem, DecisionEvidenceKind, DecisionSubject, LabPredicate,
};
use crate::videos::frame_decision::record::reject_decision_provenance;
use crate::videos::frame_decision::record::{
    DEFAULT_JEV_MODEL_REVISION, RejectedDecisionProvenance,
};
use crate::videos::index_provenance::FrameRoiTransform;
use crate::videos::saved_frame::SavedFrameRef;

/// Disallowed non-image inputs that must never reach Jev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenDecisionInput {
    /// Luna or other persisted description text.
    LunaDescription,
    /// LLM-generated criteria or rubric text.
    GeneratedCriteria,
    /// Chat assistant output.
    ChatOutput,
    /// ANN or embedding similarity scores.
    EmbeddingSimilarity,
}

impl ForbiddenDecisionInput {
    /// Stable token for failure reporting.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LunaDescription => "luna_description",
            Self::GeneratedCriteria => "generated_criteria",
            Self::ChatOutput => "chat_output",
            Self::EmbeddingSimilarity => "embedding_similarity",
        }
    }
}

/// Rejects a forbidden decision input before routing to Jev.
pub fn reject_forbidden_decision_input(
    input: ForbiddenDecisionInput,
) -> Result<(), FrameDecisionEvaluationFailure> {
    let message = match input {
        ForbiddenDecisionInput::LunaDescription => "Luna descriptions are retrieval metadata only",
        ForbiddenDecisionInput::GeneratedCriteria => {
            "generated criteria cannot supply Jev decision evidence"
        }
        ForbiddenDecisionInput::ChatOutput => "chat output cannot supply Jev decision evidence",
        ForbiddenDecisionInput::EmbeddingSimilarity => {
            "embedding similarity cannot supply Jev decision evidence"
        }
    };
    Err(FrameDecisionEvaluationFailure::ForbiddenInput {
        kind: input.as_str().to_owned(),
        message: message.to_owned(),
    })
}

/// Operator-authored reference image digest for criteria context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatorReferenceImage {
    digest: Sha256Digest,
    label: String,
    roi: FrameRoiTransform,
}

impl OperatorReferenceImage {
    /// Validates a bounded operator reference still.
    pub fn new(
        digest: Sha256Digest,
        label: impl AsRef<str>,
        roi: FrameRoiTransform,
    ) -> Result<Self, A2aLabError> {
        let label = label.as_ref().trim();
        if label.is_empty() || label.len() > 64 {
            return Err(A2aLabError::invalid(
                "reference_label",
                "must be 1..=64 visible characters",
            ));
        }
        Ok(Self {
            digest,
            label: label.to_owned(),
            roi,
        })
    }

    /// Digest of the operator-supplied still image.
    #[must_use]
    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Short operator label for audit trails.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// ROI on the oriented reference still.
    #[must_use]
    pub const fn roi(&self) -> FrameRoiTransform {
        self.roi
    }
}

/// Operator-authored criteria bundle referenced by digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatorDecisionCriteria {
    criteria_digest: Sha256Digest,
    prompt_digest: Sha256Digest,
    references: Vec<OperatorReferenceImage>,
}

impl OperatorDecisionCriteria {
    /// Builds validated operator criteria.
    pub fn new(
        criteria_digest: Sha256Digest,
        prompt_digest: Sha256Digest,
        references: Vec<OperatorReferenceImage>,
    ) -> Result<Self, A2aLabError> {
        if references.is_empty() {
            return Err(A2aLabError::invalid(
                "references",
                "at least one operator reference image is required",
            ));
        }
        Ok(Self {
            criteria_digest,
            prompt_digest,
            references,
        })
    }

    /// Digest of the operator criteria bundle.
    #[must_use]
    pub const fn criteria_digest(&self) -> &Sha256Digest {
        &self.criteria_digest
    }

    /// Digest of the Jev instruction prompt.
    #[must_use]
    pub const fn prompt_digest(&self) -> &Sha256Digest {
        &self.prompt_digest
    }

    /// Operator reference stills.
    #[must_use]
    pub fn references(&self) -> &[OperatorReferenceImage] {
        &self.references
    }
}

/// One bounded evaluation against exact saved frame evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameDecisionEvaluationRequest {
    evaluation_id: JevEvaluationId,
    idempotency_key: DecisionIdempotencyKey,
    target: SavedFrameRef,
    predicate: LabPredicate,
    subject: DecisionSubject,
    evidence: Vec<DecisionEvidenceItem>,
    criteria: OperatorDecisionCriteria,
    model_input_transform: FrameRoiTransform,
    declared_request_bytes: usize,
    max_edge_pixels: u32,
}

impl FrameDecisionEvaluationRequest {
    /// Creates an evaluation request; an omitted model transform means the
    /// full oriented frame.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_optional_model_input_transform(
        limits: FrameDecisionEvaluatorLimits,
        evaluation_id: JevEvaluationId,
        target: SavedFrameRef,
        predicate: LabPredicate,
        subject: DecisionSubject,
        evidence: Vec<DecisionEvidenceItem>,
        criteria: OperatorDecisionCriteria,
        model_input_transform: Option<FrameRoiTransform>,
        declared_request_bytes: usize,
        max_edge_pixels: u32,
    ) -> Result<Self, FrameDecisionEvaluationFailure> {
        Self::new(
            limits,
            evaluation_id,
            target,
            predicate,
            subject,
            evidence,
            criteria,
            model_input_transform.unwrap_or_else(FrameRoiTransform::full_frame),
            declared_request_bytes,
            max_edge_pixels,
        )
    }

    /// Creates a request after limit and Jev-only evidence checks.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        limits: FrameDecisionEvaluatorLimits,
        evaluation_id: JevEvaluationId,
        target: SavedFrameRef,
        predicate: LabPredicate,
        subject: DecisionSubject,
        evidence: Vec<DecisionEvidenceItem>,
        criteria: OperatorDecisionCriteria,
        model_input_transform: FrameRoiTransform,
        declared_request_bytes: usize,
        max_edge_pixels: u32,
    ) -> Result<Self, FrameDecisionEvaluationFailure> {
        if evidence.is_empty() {
            return Err(FrameDecisionEvaluationFailure::InvalidEvidence {
                message: "at least one saved-frame evidence item is required".to_owned(),
            });
        }
        if evidence.len() > usize::try_from(limits.max_evidence_items()).unwrap_or(usize::MAX) {
            return Err(FrameDecisionEvaluationFailure::LimitExceeded {
                limit: "max_evidence_items".to_owned(),
                message: format!(
                    "evidence count {} exceeds {}",
                    evidence.len(),
                    limits.max_evidence_items()
                ),
            });
        }
        let reference_count = evidence
            .iter()
            .filter(|item| item.kind() == DecisionEvidenceKind::ReferenceImage)
            .count()
            + criteria.references().len();
        if reference_count > usize::try_from(limits.max_reference_images()).unwrap_or(usize::MAX) {
            return Err(FrameDecisionEvaluationFailure::LimitExceeded {
                limit: "max_reference_images".to_owned(),
                message: format!(
                    "reference images {reference_count} exceed {}",
                    limits.max_reference_images()
                ),
            });
        }
        if declared_request_bytes > limits.max_request_bytes() {
            return Err(FrameDecisionEvaluationFailure::LimitExceeded {
                limit: "max_request_bytes".to_owned(),
                message: format!(
                    "request bytes {declared_request_bytes} exceed {}",
                    limits.max_request_bytes()
                ),
            });
        }
        if max_edge_pixels > limits.max_edge_pixels() || max_edge_pixels < limits.min_edge_pixels()
        {
            return Err(FrameDecisionEvaluationFailure::LimitExceeded {
                limit: "edge_pixels".to_owned(),
                message: format!(
                    "edge pixels {max_edge_pixels} outside {}..={}",
                    limits.min_edge_pixels(),
                    limits.max_edge_pixels()
                ),
            });
        }
        if !subject.matches_predicate(&predicate) {
            return Err(FrameDecisionEvaluationFailure::InvalidEvidence {
                message: "subject does not match predicate schema".to_owned(),
            });
        }
        let frame_id = target.frame_id();
        for item in &evidence {
            if item.frame_id() != frame_id {
                return Err(FrameDecisionEvaluationFailure::InvalidEvidence {
                    message: "evidence must reference the target frame revision".to_owned(),
                });
            }
            if let Some(reference) = item.reference_digest()
                && is_forbidden_reference_digest(reference)
            {
                map_forbidden_digest(reference)?;
            }
        }
        for reference in criteria.references() {
            if is_forbidden_reference_digest(reference.digest()) {
                map_forbidden_digest(reference.digest())?;
            }
        }

        let idempotency_key = DecisionIdempotencyKey::new(format!(
            "{}:{}:{}",
            evaluation_id.as_str(),
            predicate.id(),
            subject_token(&subject)
        ))
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidEvidence {
            message: error.to_string(),
        })?;

        Ok(Self {
            evaluation_id,
            idempotency_key,
            target,
            predicate,
            subject,
            evidence,
            criteria,
            model_input_transform,
            declared_request_bytes,
            max_edge_pixels,
        })
    }

    /// Jev evaluation request id.
    #[must_use]
    pub const fn evaluation_id(&self) -> &JevEvaluationId {
        &self.evaluation_id
    }

    /// Idempotency key for durable append.
    #[must_use]
    pub const fn idempotency_key(&self) -> &DecisionIdempotencyKey {
        &self.idempotency_key
    }

    /// Target saved frame revision.
    #[must_use]
    pub const fn target(&self) -> &SavedFrameRef {
        &self.target
    }

    /// Lab predicate under evaluation.
    #[must_use]
    pub const fn predicate(&self) -> &LabPredicate {
        &self.predicate
    }

    /// Predicate subject instance.
    #[must_use]
    pub const fn subject(&self) -> &DecisionSubject {
        &self.subject
    }

    /// Exact image evidence items.
    #[must_use]
    pub fn evidence(&self) -> &[DecisionEvidenceItem] {
        &self.evidence
    }

    /// Operator-authored criteria and references.
    #[must_use]
    pub const fn criteria(&self) -> &OperatorDecisionCriteria {
        &self.criteria
    }

    /// Recorded model-input ROI transform.
    #[must_use]
    pub const fn model_input_transform(&self) -> FrameRoiTransform {
        self.model_input_transform
    }

    /// Declared serialized request size for limit accounting.
    #[must_use]
    pub const fn declared_request_bytes(&self) -> usize {
        self.declared_request_bytes
    }

    /// Declared longest edge after preprocessing.
    #[must_use]
    pub const fn max_edge_pixels(&self) -> u32 {
        self.max_edge_pixels
    }

    /// Stable key token for batch validation.
    #[must_use]
    pub fn key_token(&self) -> String {
        format!(
            "{}:{}:{}",
            self.evaluation_id().as_str(),
            self.predicate().id(),
            self.target().frame_id()
        )
    }
}

fn is_forbidden_reference_digest(digest: &Sha256Digest) -> bool {
    let marker = digest.as_str();
    marker == Sha256Digest::hash_bytes(b"description-derived").as_str()
        || marker == Sha256Digest::hash_bytes(b"generated-criteria").as_str()
        || marker == Sha256Digest::hash_bytes(b"chat-derived").as_str()
        || marker == Sha256Digest::hash_bytes(b"embedding-similarity").as_str()
}

fn map_forbidden_digest(digest: &Sha256Digest) -> Result<(), FrameDecisionEvaluationFailure> {
    let marker = digest.as_str();
    if marker == Sha256Digest::hash_bytes(b"description-derived").as_str() {
        reject_forbidden_decision_input(ForbiddenDecisionInput::LunaDescription)?;
    }
    if marker == Sha256Digest::hash_bytes(b"generated-criteria").as_str() {
        reject_forbidden_decision_input(ForbiddenDecisionInput::GeneratedCriteria)?;
    }
    if marker == Sha256Digest::hash_bytes(b"chat-derived").as_str() {
        reject_forbidden_decision_input(ForbiddenDecisionInput::ChatOutput)?;
    }
    reject_forbidden_decision_input(ForbiddenDecisionInput::EmbeddingSimilarity)
}

fn subject_token(subject: &DecisionSubject) -> String {
    match subject {
        DecisionSubject::PipetteChannel(entry) => {
            format!("pipette:{}:{}", entry.pipetter_id(), entry.channel())
        }
        DecisionSubject::ForeignObject(entry) => format!("foreign:{}", entry.deck_region()),
        DecisionSubject::PlateSlot(entry) => {
            format!("plate:{}:{}", entry.plate_id(), entry.deck_slot())
        }
    }
}

/// Rejects LLM description evidence that bypassed digest markers.
pub fn guard_against_description_evidence(
    evidence: &[DecisionEvidenceItem],
) -> Result<(), FrameDecisionEvaluationFailure> {
    for item in evidence {
        if let Some(reference) = item.reference_digest()
            && reference.as_str() == Sha256Digest::hash_bytes(b"description-derived").as_str()
        {
            let _ = reject_decision_provenance(RejectedDecisionProvenance::DescriptionDerived);
            return reject_forbidden_decision_input(ForbiddenDecisionInput::LunaDescription);
        }
    }
    Ok(())
}

/// Pilot-only model revision guard for contracts.
#[must_use]
pub const fn default_pinned_jev_revision() -> &'static str {
    DEFAULT_JEV_MODEL_REVISION
}
