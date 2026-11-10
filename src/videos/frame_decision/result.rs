//! Jev evaluation results, thresholding, and record materialization.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;
use crate::videos::frame_decision::config::FrameDecisionEvaluatorProviderConfig;
use crate::videos::frame_decision::failure::FrameDecisionEvaluationFailure;
use crate::videos::frame_decision::predicate::LabPredicate;
use crate::videos::frame_decision::record::{
    DecisionEvaluationCoverage, DecisionOutcome, FrameDecisionRecord, JevEvaluationProvenance,
    JevStructuredOutput,
};
use crate::videos::frame_decision::request::FrameDecisionEvaluationRequest;
use crate::videos::index_provenance::FrameRoiTransform;

/// Deterministic threshold policy for mapping Jev primitives to outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionThresholdPolicy {
    present_min_noul: f32,
    absent_max_noul: f32,
    present_choice_label: &'static str,
    absent_choice_label: &'static str,
}

impl DecisionThresholdPolicy {
    /// Validates unit-interval bounds with present strictly above absent band.
    pub fn new(
        present_min_noul: f32,
        absent_max_noul: f32,
        present_choice_label: &'static str,
        absent_choice_label: &'static str,
    ) -> Result<Self, A2aLabError> {
        if !(0.0..=1.0).contains(&present_min_noul)
            || !(0.0..=1.0).contains(&absent_max_noul)
            || !present_min_noul.is_finite()
            || !absent_max_noul.is_finite()
        {
            return Err(A2aLabError::invalid(
                "threshold",
                "noul bounds must be finite values in 0.0..=1.0",
            ));
        }
        if present_min_noul <= absent_max_noul {
            return Err(A2aLabError::invalid(
                "threshold",
                "present threshold must exceed absent threshold",
            ));
        }
        if present_choice_label.trim().is_empty() || absent_choice_label.trim().is_empty() {
            return Err(A2aLabError::invalid(
                "threshold",
                "choice labels must not be empty",
            ));
        }
        Ok(Self {
            present_min_noul,
            absent_max_noul,
            present_choice_label,
            absent_choice_label,
        })
    }

    /// Conservative pilot defaults requiring strong observability.
    #[must_use]
    pub fn pilot() -> Self {
        Self {
            present_min_noul: 0.85,
            absent_max_noul: 0.15,
            present_choice_label: "present",
            absent_choice_label: "absent",
        }
    }

    /// Minimum Noul probability treated as present.
    #[must_use]
    pub const fn present_min_noul(self) -> f32 {
        self.present_min_noul
    }

    /// Maximum Noul probability treated as absent.
    #[must_use]
    pub const fn absent_max_noul(self) -> f32 {
        self.absent_max_noul
    }
}

/// Maps typed Jev structured output into a lab outcome when coverage is evaluated.
pub fn interpret_structured_outcome(
    structured: &JevStructuredOutput,
    policy: DecisionThresholdPolicy,
) -> Result<DecisionOutcome, FrameDecisionEvaluationFailure> {
    structured
        .validate()
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        })?;
    if let Some(noul) = structured.noul_probability() {
        if noul >= policy.present_min_noul() {
            return Ok(DecisionOutcome::Present);
        }
        if noul <= policy.absent_max_noul() {
            return Ok(DecisionOutcome::Absent);
        }
        return Ok(DecisionOutcome::Unknown);
    }
    if let Some(distribution) = structured.option_distribution() {
        let labels = distribution.labels();
        let values = distribution.values();
        let mut present = None;
        let mut absent = None;
        for (label, value) in labels.iter().zip(values.iter()) {
            if label == policy.present_choice_label {
                present = Some(*value);
            }
            if label == policy.absent_choice_label {
                absent = Some(*value);
            }
        }
        if let (Some(p), Some(a)) = (present, absent) {
            if p >= policy.present_min_noul() && a <= policy.absent_max_noul() {
                return Ok(DecisionOutcome::Present);
            }
            if a >= policy.present_min_noul() && p <= policy.absent_max_noul() {
                return Ok(DecisionOutcome::Absent);
            }
        }
    }
    Ok(DecisionOutcome::Unknown)
}

/// Keyed Jev response for one evaluation request.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyedFrameDecisionEvaluation {
    request_key: String,
    coverage: DecisionEvaluationCoverage,
    outcome: Option<DecisionOutcome>,
    structured_output: Option<JevStructuredOutput>,
    provenance: Option<JevEvaluationProvenance>,
    jev_response_digest: Option<Sha256Digest>,
}

impl KeyedFrameDecisionEvaluation {
    /// Builds a response with consistency checks.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &FrameDecisionEvaluationRequest,
        coverage: DecisionEvaluationCoverage,
        outcome: Option<DecisionOutcome>,
        structured_output: Option<JevStructuredOutput>,
        provenance: Option<JevEvaluationProvenance>,
        jev_response_digest: Option<Sha256Digest>,
    ) -> Result<Self, FrameDecisionEvaluationFailure> {
        validate_coverage_consistency(
            coverage,
            outcome,
            structured_output.as_ref(),
            provenance.as_ref(),
        )?;
        if let Some(digest) = &jev_response_digest
            && digest.as_str().is_empty()
        {
            return Err(FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "jev response digest must not be empty".to_owned(),
            });
        }
        Ok(Self {
            request_key: request.key_token(),
            coverage,
            outcome,
            structured_output,
            provenance,
            jev_response_digest,
        })
    }

    /// Request correlation token.
    #[must_use]
    pub fn request_key(&self) -> &str {
        &self.request_key
    }

    /// Job coverage distinct from present/absent/unknown outcomes.
    #[must_use]
    pub const fn coverage(&self) -> DecisionEvaluationCoverage {
        self.coverage
    }

    /// Interpreted outcome when coverage is evaluated.
    #[must_use]
    pub const fn outcome(&self) -> Option<DecisionOutcome> {
        self.outcome
    }

    /// Typed Jev structured output when present.
    #[must_use]
    pub const fn structured_output(&self) -> Option<&JevStructuredOutput> {
        self.structured_output.as_ref()
    }

    /// Authorized provenance when evaluation completed.
    #[must_use]
    pub const fn provenance(&self) -> Option<&JevEvaluationProvenance> {
        self.provenance.as_ref()
    }

    /// Raw response digest used to authorize append.
    #[must_use]
    pub const fn jev_response_digest(&self) -> Option<&Sha256Digest> {
        self.jev_response_digest.as_ref()
    }

    /// Materializes an immutable decision record when coverage is evaluated.
    pub fn into_record(
        self,
        request: &FrameDecisionEvaluationRequest,
        config: &FrameDecisionEvaluatorProviderConfig,
        captured_at: UtcTimestamp,
        evaluated_at: UtcTimestamp,
        model_input_transform: FrameRoiTransform,
    ) -> Result<FrameDecisionRecord, FrameDecisionEvaluationFailure> {
        if !self.coverage.permits_decision_outcome() {
            return Err(FrameDecisionEvaluationFailure::InvalidProvenance {
                message: format!(
                    "coverage {:?} cannot materialize a decision record",
                    self.coverage
                ),
            });
        }
        let outcome =
            self.outcome
                .ok_or_else(|| FrameDecisionEvaluationFailure::InvalidProvenance {
                    message: "evaluated coverage requires an outcome".to_owned(),
                })?;
        let structured = self.structured_output.ok_or_else(|| {
            FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "evaluated coverage requires structured output".to_owned(),
            }
        })?;
        if self.provenance.is_none() {
            return Err(FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "evaluated coverage requires provenance".to_owned(),
            });
        }
        let jev_response_digest = self.jev_response_digest.ok_or_else(|| {
            FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "evaluated coverage requires jev response digest".to_owned(),
            }
        })?;
        let provenance = JevEvaluationProvenance::new(
            captured_at,
            evaluated_at,
            config.producer().clone(),
            request.evaluation_id().clone(),
            config.config_digest().clone(),
            request.criteria().prompt_digest().clone(),
            request.criteria().criteria_digest().clone(),
            config.threshold_policy_digest().clone(),
            model_input_transform,
            jev_response_digest,
            DecisionEvaluationCoverage::Evaluated,
            Option::<&str>::None,
        )
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        })?;
        FrameDecisionRecord::new(
            request.idempotency_key().clone(),
            request.target().clone(),
            request.predicate().clone(),
            request.subject().clone(),
            request.evidence().to_vec(),
            outcome,
            structured,
            provenance,
            None,
        )
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        })
    }
}

/// Validates batch responses against requested keys.
pub fn validate_evaluation_batch(
    requests: &[FrameDecisionEvaluationRequest],
    responses: &[KeyedFrameDecisionEvaluation],
) -> Result<Vec<KeyedFrameDecisionEvaluation>, FrameDecisionEvaluationFailure> {
    if responses.len() != requests.len() {
        return Err(FrameDecisionEvaluationFailure::PartialResponse {
            message: format!(
                "expected {} keyed evaluations, got {}",
                requests.len(),
                responses.len()
            ),
        });
    }
    let mut expected: std::collections::HashSet<String> = std::collections::HashSet::new();
    for request in requests {
        let token = request.key_token();
        if !expected.insert(token) {
            return Err(FrameDecisionEvaluationFailure::KeyMismatch {
                message: "duplicate request key in evaluation batch".to_owned(),
            });
        }
    }
    let mut matched: std::collections::HashSet<String> = std::collections::HashSet::new();
    for response in responses {
        if !expected.contains(response.request_key()) {
            return Err(FrameDecisionEvaluationFailure::KeyMismatch {
                message: format!("response key {} was not requested", response.request_key()),
            });
        }
        if !matched.insert(response.request_key().to_owned()) {
            return Err(FrameDecisionEvaluationFailure::KeyMismatch {
                message: "duplicate response key in evaluation batch".to_owned(),
            });
        }
    }
    if matched.len() != expected.len() {
        return Err(FrameDecisionEvaluationFailure::PartialResponse {
            message: "one or more requested evaluation keys are missing".to_owned(),
        });
    }
    Ok(responses.to_vec())
}

fn validate_coverage_consistency(
    coverage: DecisionEvaluationCoverage,
    outcome: Option<DecisionOutcome>,
    structured: Option<&JevStructuredOutput>,
    provenance: Option<&JevEvaluationProvenance>,
) -> Result<(), FrameDecisionEvaluationFailure> {
    match coverage {
        DecisionEvaluationCoverage::Evaluated => {
            if outcome.is_none() || structured.is_none() || provenance.is_none() {
                return Err(FrameDecisionEvaluationFailure::InvalidProvenance {
                    message:
                        "evaluated coverage requires outcome, structured output, and provenance"
                            .to_owned(),
                });
            }
        }
        DecisionEvaluationCoverage::NotEvaluated
        | DecisionEvaluationCoverage::Failed
        | DecisionEvaluationCoverage::TimedOut
        | DecisionEvaluationCoverage::Cancelled
        | DecisionEvaluationCoverage::Unsupported => {
            if outcome.is_some() {
                return Err(FrameDecisionEvaluationFailure::InvalidProvenance {
                    message: format!(
                        "coverage {coverage:?} must not carry a present/absent/unknown outcome"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Predicate-level unsupported marker for partial evaluator support.
#[must_use]
pub fn unsupported_predicate_result(
    request: &FrameDecisionEvaluationRequest,
    _predicate: &LabPredicate,
) -> KeyedFrameDecisionEvaluation {
    KeyedFrameDecisionEvaluation {
        request_key: request.key_token(),
        coverage: DecisionEvaluationCoverage::Unsupported,
        outcome: None,
        structured_output: None,
        provenance: None,
        jev_response_digest: None,
    }
}
