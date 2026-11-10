//! Immutable Jev-only frame decision records and provenance.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::A2aLabError;
use crate::id::{DecisionIdempotencyKey, FrameDecisionId, JevEvaluationId, SavedFrameId};
use crate::json_object::JsonObject;
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;
use crate::videos::frame_decision::predicate::{
    DecisionEvidenceItem, DecisionSubject, LabPredicate,
};
use crate::videos::index_provenance::FrameRoiTransform;
use crate::videos::saved_frame::SavedFrameRef;

/// Production Jev producer id.
pub const JEV_PRODUCER_ID: &str = "typesafe-jev";

/// Default pinned Jev model revision for contracts.
pub const DEFAULT_JEV_MODEL_REVISION: &str = "jev-1.14.0";

/// Lab frame decision outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// Predicate observed present.
    Present,
    /// Predicate observed absent.
    Absent,
    /// Observable evidence insufficient for present/absent.
    Unknown,
}

/// Evaluation job coverage distinct from decision outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEvaluationCoverage {
    /// Jev produced a decision outcome.
    Evaluated,
    /// Job never ran.
    NotEvaluated,
    /// Request failed before a decision.
    Failed,
    /// Job exceeded its deadline.
    TimedOut,
    /// Operator or runner cancelled the job.
    Cancelled,
    /// Predicate unsupported for the evidence.
    Unsupported,
}

impl DecisionEvaluationCoverage {
    /// Whether this coverage may be stored with a present/absent/unknown outcome.
    #[must_use]
    pub const fn permits_decision_outcome(self) -> bool {
        matches!(self, Self::Evaluated)
    }
}

/// Disallowed provenance origin for decision evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RejectedDecisionProvenance {
    /// Luna or other description text.
    DescriptionDerived,
    /// Chat or LLM generated assessment.
    LlmAssessment,
    /// Embedding retrieval similarity.
    EmbeddingSimilarity,
    /// Client producer label without Jev response digest.
    ProducerLabelOnly,
}

/// Authorized TypeSafe Jev producer identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JevProducerIdentity {
    producer_id: String,
    model_revision: String,
}

impl JevProducerIdentity {
    /// Validates the sole authorized production decision producer.
    pub fn new(model_revision: impl AsRef<str>) -> Result<Self, A2aLabError> {
        let model_revision = model_revision.as_ref().trim();
        if model_revision.is_empty() {
            return Err(A2aLabError::invalid(
                "model_revision",
                "Jev model revision must not be empty",
            ));
        }
        Ok(Self {
            producer_id: JEV_PRODUCER_ID.to_owned(),
            model_revision: model_revision.to_owned(),
        })
    }

    /// Default pinned revision.
    #[must_use]
    pub fn default_production() -> Self {
        Self {
            producer_id: JEV_PRODUCER_ID.to_owned(),
            model_revision: DEFAULT_JEV_MODEL_REVISION.to_owned(),
        }
    }

    /// Authorized producer id.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Resolved pinned Jev revision.
    #[must_use]
    pub fn model_revision(&self) -> &str {
        &self.model_revision
    }

    /// Rejects forged or non-Jev producer labels after deserialization.
    pub fn validate(&self) -> Result<(), A2aLabError> {
        if self.producer_id != JEV_PRODUCER_ID {
            return Err(A2aLabError::invalid(
                "producer",
                "only typesafe-jev may author production decisions",
            ));
        }
        if self.model_revision.trim().is_empty() {
            return Err(A2aLabError::invalid(
                "model_revision",
                "Jev model revision must not be empty",
            ));
        }
        Ok(())
    }
}

/// Normalized probability vector from Jev choice/score primitives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabProbabilityVector {
    labels: Vec<String>,
    values: Vec<f32>,
}

impl LabProbabilityVector {
    /// Validates label-aligned probabilities.
    pub fn new(labels: Vec<String>, values: Vec<f32>) -> Result<Self, A2aLabError> {
        if labels.is_empty() || labels.len() != values.len() {
            return Err(A2aLabError::invalid(
                "probabilities",
                "labels and values must be non-empty and equal length",
            ));
        }
        if labels.iter().any(|label| label.trim().is_empty()) {
            return Err(A2aLabError::invalid(
                "probabilities",
                "labels must not be empty",
            ));
        }
        for value in &values {
            validate_unit_interval("probabilities", *value)?;
        }
        Ok(Self { labels, values })
    }

    /// Option or level labels.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Aligned probability mass.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

/// Structured Jev outputs persisted with the decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JevStructuredOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    noul_probability: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    option_distribution: Option<LabProbabilityVector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    level_distribution: Option<LabProbabilityVector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    rubric_score: Option<f32>,
    /// Vendor confidence summarizing a distribution; not class probability.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    vendor_confidence: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    evidence_box: Option<FrameRoiTransform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    structured_value: Option<JsonObject>,
}

impl JevStructuredOutput {
    /// Empty structured output.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            noul_probability: None,
            option_distribution: None,
            level_distribution: None,
            rubric_score: None,
            vendor_confidence: None,
            evidence_box: None,
            structured_value: None,
        }
    }

    /// Builds structured Jev output with optional primitive fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        noul_probability: Option<f32>,
        option_distribution: Option<LabProbabilityVector>,
        level_distribution: Option<LabProbabilityVector>,
        rubric_score: Option<f32>,
        vendor_confidence: Option<f32>,
        evidence_box: Option<FrameRoiTransform>,
        structured_value: Option<JsonObject>,
    ) -> Result<Self, A2aLabError> {
        let output = Self {
            noul_probability,
            option_distribution,
            level_distribution,
            rubric_score,
            vendor_confidence,
            evidence_box,
            structured_value,
        };
        output.validate()?;
        Ok(output)
    }

    /// Validates ranges on optional Jev primitive outputs.
    pub fn validate(&self) -> Result<(), A2aLabError> {
        if let Some(value) = self.noul_probability {
            validate_unit_interval("noul_probability", value)?;
        }
        if let Some(distribution) = &self.option_distribution {
            distribution.clone().validate_lengths()?;
        }
        if let Some(distribution) = &self.level_distribution {
            distribution.clone().validate_lengths()?;
        }
        if let Some(value) = self.rubric_score
            && !value.is_finite()
        {
            return Err(A2aLabError::invalid(
                "rubric_score",
                "must be a finite number",
            ));
        }
        if let Some(value) = self.vendor_confidence {
            validate_unit_interval("vendor_confidence", value)?;
        }
        Ok(())
    }

    /// Optional Noul proposition probability.
    #[must_use]
    pub const fn noul_probability(&self) -> Option<f32> {
        self.noul_probability
    }

    /// Choice option distribution.
    #[must_use]
    pub const fn option_distribution(&self) -> Option<&LabProbabilityVector> {
        self.option_distribution.as_ref()
    }

    /// Score level distribution.
    #[must_use]
    pub const fn level_distribution(&self) -> Option<&LabProbabilityVector> {
        self.level_distribution.as_ref()
    }

    /// Rubric position when supplied.
    #[must_use]
    pub const fn rubric_score(&self) -> Option<f32> {
        self.rubric_score
    }

    /// Vendor confidence separate from class probability.
    #[must_use]
    pub const fn vendor_confidence(&self) -> Option<f32> {
        self.vendor_confidence
    }

    /// Optional Box primitive evidence region.
    #[must_use]
    pub const fn evidence_box(&self) -> Option<FrameRoiTransform> {
        self.evidence_box
    }

    /// Predicate-specific structured payload.
    #[must_use]
    pub const fn structured_value(&self) -> Option<&JsonObject> {
        self.structured_value.as_ref()
    }
}

impl LabProbabilityVector {
    fn validate_lengths(&self) -> Result<(), A2aLabError> {
        Self::new(self.labels.clone(), self.values.clone()).map(|_| ())
    }
}

/// Capture and evaluation provenance for one Jev decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JevEvaluationProvenance {
    captured_at: UtcTimestamp,
    evaluated_at: UtcTimestamp,
    producer: JevProducerIdentity,
    evaluation_id: JevEvaluationId,
    config_digest: Sha256Digest,
    prompt_digest: Sha256Digest,
    criteria_digest: Sha256Digest,
    threshold_policy_digest: Sha256Digest,
    model_input_transform: FrameRoiTransform,
    jev_response_digest: Sha256Digest,
    coverage: DecisionEvaluationCoverage,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    calibration_id: Option<String>,
}

impl JevEvaluationProvenance {
    /// Validates Jev-only provenance with separated digests and times.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        captured_at: UtcTimestamp,
        evaluated_at: UtcTimestamp,
        producer: JevProducerIdentity,
        evaluation_id: JevEvaluationId,
        config_digest: Sha256Digest,
        prompt_digest: Sha256Digest,
        criteria_digest: Sha256Digest,
        threshold_policy_digest: Sha256Digest,
        model_input_transform: FrameRoiTransform,
        jev_response_digest: Sha256Digest,
        coverage: DecisionEvaluationCoverage,
        calibration_id: Option<impl AsRef<str>>,
    ) -> Result<Self, A2aLabError> {
        let calibration_id = calibration_id.map(|value| value.as_ref().to_owned());
        let candidate = Self {
            captured_at,
            evaluated_at,
            producer,
            evaluation_id,
            config_digest,
            prompt_digest,
            criteria_digest,
            threshold_policy_digest,
            model_input_transform,
            jev_response_digest,
            coverage,
            calibration_id,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Validates Jev authority, coverage, and timeline after deserialization.
    pub fn validate(&self) -> Result<(), A2aLabError> {
        self.producer.validate()?;
        if self.evaluated_at < self.captured_at {
            return Err(A2aLabError::invalid(
                "evaluated_at",
                "must not precede capture time",
            ));
        }
        if !self.coverage.permits_decision_outcome() {
            return Err(A2aLabError::invalid(
                "coverage",
                "failed or not-evaluated jobs cannot produce decision records",
            ));
        }
        if let Some(id) = &self.calibration_id
            && id.trim().is_empty()
        {
            return Err(A2aLabError::invalid(
                "calibration_id",
                "must not be empty when present",
            ));
        }
        Ok(())
    }

    /// Frame capture instant.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Jev evaluation instant.
    #[must_use]
    pub const fn evaluated_at(&self) -> UtcTimestamp {
        self.evaluated_at
    }

    /// Authorized Jev producer identity.
    #[must_use]
    pub const fn producer(&self) -> &JevProducerIdentity {
        &self.producer
    }

    /// Evaluation request id.
    #[must_use]
    pub const fn evaluation_id(&self) -> &JevEvaluationId {
        &self.evaluation_id
    }

    /// Runtime configuration digest.
    #[must_use]
    pub const fn config_digest(&self) -> &Sha256Digest {
        &self.config_digest
    }

    /// Criteria/instruction digest.
    #[must_use]
    pub const fn prompt_digest(&self) -> &Sha256Digest {
        &self.prompt_digest
    }

    /// Reference/criteria bundle digest.
    #[must_use]
    pub const fn criteria_digest(&self) -> &Sha256Digest {
        &self.criteria_digest
    }

    /// Threshold interpretation policy digest.
    #[must_use]
    pub const fn threshold_policy_digest(&self) -> &Sha256Digest {
        &self.threshold_policy_digest
    }

    /// Recorded model-input ROI transform.
    #[must_use]
    pub const fn model_input_transform(&self) -> FrameRoiTransform {
        self.model_input_transform
    }

    /// Digest of the raw Jev response used to authorize append.
    #[must_use]
    pub const fn jev_response_digest(&self) -> &Sha256Digest {
        &self.jev_response_digest
    }

    /// Coverage state for the evaluation job.
    #[must_use]
    pub const fn coverage(&self) -> DecisionEvaluationCoverage {
        self.coverage
    }

    /// Optional calibration identity for rubric scores.
    #[must_use]
    pub fn calibration_id(&self) -> Option<&str> {
        self.calibration_id.as_deref()
    }
}

/// Immutable production frame decision bound to one saved revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionRecord {
    decision_id: FrameDecisionId,
    idempotency_key: DecisionIdempotencyKey,
    frame: SavedFrameRef,
    predicate: LabPredicate,
    subject: DecisionSubject,
    evidence: Vec<DecisionEvidenceItem>,
    outcome: DecisionOutcome,
    structured_output: JevStructuredOutput,
    provenance: JevEvaluationProvenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    supersedes: Option<FrameDecisionId>,
}

impl FrameDecisionRecord {
    /// Validates and builds an immutable Jev decision record.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_key: DecisionIdempotencyKey,
        frame: SavedFrameRef,
        predicate: LabPredicate,
        subject: DecisionSubject,
        evidence: Vec<DecisionEvidenceItem>,
        outcome: DecisionOutcome,
        structured_output: JevStructuredOutput,
        provenance: JevEvaluationProvenance,
        supersedes: Option<FrameDecisionId>,
    ) -> Result<Self, A2aLabError> {
        let decision_id = derive_decision_id(frame.frame_id(), &idempotency_key)?;
        let candidate = Self {
            decision_id,
            idempotency_key,
            frame,
            predicate,
            subject,
            evidence,
            outcome,
            structured_output,
            provenance,
            supersedes,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Full production-record validation for trusted constructors and untrusted payloads.
    pub fn validate(&self) -> Result<(), A2aLabError> {
        if !self.subject.matches_predicate(&self.predicate) {
            return Err(A2aLabError::invalid(
                "subject",
                "does not match predicate schema",
            ));
        }
        if self.evidence.is_empty() {
            return Err(A2aLabError::invalid(
                "evidence",
                "at least one saved-frame evidence item is required",
            ));
        }
        self.structured_output.validate()?;
        self.provenance.validate()?;
        validate_evidence_scope(&self.frame, &self.evidence)?;
        validate_idempotency_alignment(&self.idempotency_key, &self.provenance)?;
        let expected_id = derive_decision_id(self.frame.frame_id(), &self.idempotency_key)?;
        if self.decision_id != expected_id {
            return Err(A2aLabError::invalid(
                "decision_id",
                "must match the derived frame decision identifier",
            ));
        }
        Ok(())
    }

    /// Stable decision identifier.
    #[must_use]
    pub const fn decision_id(&self) -> &FrameDecisionId {
        &self.decision_id
    }

    /// Replay/idempotency key for append operations.
    #[must_use]
    pub const fn idempotency_key(&self) -> &DecisionIdempotencyKey {
        &self.idempotency_key
    }

    /// Exact saved frame revision.
    #[must_use]
    pub const fn frame(&self) -> &SavedFrameRef {
        &self.frame
    }

    /// Predicate declaration.
    #[must_use]
    pub const fn predicate(&self) -> &LabPredicate {
        &self.predicate
    }

    /// Predicate subject schema instance.
    #[must_use]
    pub const fn subject(&self) -> &DecisionSubject {
        &self.subject
    }

    /// Image/reference evidence items.
    #[must_use]
    pub fn evidence(&self) -> &[DecisionEvidenceItem] {
        &self.evidence
    }

    /// Interpreted lab outcome.
    #[must_use]
    pub const fn outcome(&self) -> DecisionOutcome {
        self.outcome
    }

    /// Structured Jev primitive outputs.
    #[must_use]
    pub const fn structured_output(&self) -> &JevStructuredOutput {
        &self.structured_output
    }

    /// Evaluation provenance and digests.
    #[must_use]
    pub const fn provenance(&self) -> &JevEvaluationProvenance {
        &self.provenance
    }

    /// Prior decision id in a supersession chain.
    #[must_use]
    pub const fn supersedes(&self) -> Option<&FrameDecisionId> {
        self.supersedes.as_ref()
    }

    /// Canonical payload for idempotent equality checks.
    pub fn idempotent_fingerprint(&self) -> Result<Sha256Digest, A2aLabError> {
        let json =
            serde_json::to_vec(self).map_err(|error| A2aLabError::protocol(error.to_string()))?;
        Ok(Sha256Digest::hash_bytes(&json))
    }
}

/// Rejects non-image decision evidence origins.
pub fn reject_decision_provenance(origin: RejectedDecisionProvenance) -> Result<(), A2aLabError> {
    let message = match origin {
        RejectedDecisionProvenance::DescriptionDerived => {
            "description text cannot author frame decisions"
        }
        RejectedDecisionProvenance::LlmAssessment => {
            "LLM assessments cannot author frame decisions"
        }
        RejectedDecisionProvenance::EmbeddingSimilarity => {
            "embedding similarity cannot author frame decisions"
        }
        RejectedDecisionProvenance::ProducerLabelOnly => {
            "client producer label without Jev response digest is insufficient"
        }
    };
    Err(A2aLabError::invalid("provenance", message))
}

/// Derives a stable decision id from frame revision and idempotency key.
pub fn derive_decision_id(
    frame_id: &SavedFrameId,
    idempotency_key: &DecisionIdempotencyKey,
) -> Result<FrameDecisionId, A2aLabError> {
    let canonical = format!(
        "frame-decision/v1\0{}\0{}",
        frame_id.as_str(),
        idempotency_key.as_str()
    );
    let digest = Sha256::digest(canonical.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    FrameDecisionId::new(format!("fd:{hex}"))
}

fn validate_unit_interval(field: &'static str, value: f32) -> Result<(), A2aLabError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(A2aLabError::invalid(
            field,
            "must be a finite value in 0.0..=1.0",
        ));
    }
    Ok(())
}

fn validate_evidence_scope(
    frame: &SavedFrameRef,
    evidence: &[DecisionEvidenceItem],
) -> Result<(), A2aLabError> {
    let frame_id = frame.frame_id();
    for item in evidence {
        if item.frame_id() != frame_id {
            return Err(A2aLabError::invalid(
                "evidence",
                "every evidence item must reference the record frame revision",
            ));
        }
        if let Some(reference) = item.reference_digest()
            && reference.as_str() == Sha256Digest::hash_bytes(b"description-derived").as_str()
        {
            return reject_decision_provenance(RejectedDecisionProvenance::DescriptionDerived);
        }
    }
    Ok(())
}

fn validate_idempotency_alignment(
    key: &DecisionIdempotencyKey,
    provenance: &JevEvaluationProvenance,
) -> Result<(), A2aLabError> {
    let evaluation = provenance.evaluation_id().as_str();
    if !key.as_str().starts_with(evaluation) {
        return Err(A2aLabError::invalid(
            "idempotency_key",
            "must begin with the Jev evaluation id",
        ));
    }
    Ok(())
}

/// Fingerprints a batch of records for deterministic tests.
#[must_use]
pub fn decision_history_fingerprint(records: &[FrameDecisionRecord]) -> BTreeMap<String, String> {
    records
        .iter()
        .map(|record| {
            (
                record.decision_id().as_str().to_owned(),
                record
                    .idempotent_fingerprint()
                    .map(|digest| digest.hex().to_owned())
                    .unwrap_or_default(),
            )
        })
        .collect()
}
