//! Deterministic Jev evaluator doubles for contract tests.

use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;
use crate::videos::frame_decision::config::{
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig,
};
use crate::videos::frame_decision::failure::FrameDecisionEvaluationFailure;
use crate::videos::frame_decision::predicate::LabPredicate;
use crate::videos::frame_decision::provider::FrameDecisionEvaluator;
use crate::videos::frame_decision::record::{
    DecisionEvaluationCoverage, DecisionOutcome, JevEvaluationProvenance, JevProducerIdentity,
    JevStructuredOutput,
};
use crate::videos::frame_decision::request::FrameDecisionEvaluationRequest;
use crate::videos::frame_decision::result::{
    DecisionThresholdPolicy, KeyedFrameDecisionEvaluation, interpret_structured_outcome,
    unsupported_predicate_result,
};
use crate::videos::index_provenance::FrameRoiTransform;

/// Scripted behaviors for [`FakeFrameDecisionEvaluator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeJevBehavior {
    /// Returns a keyed evaluated outcome per request.
    OkPresent,
    /// Abstains with unknown outcome while coverage remains evaluated.
    Abstain,
    /// Reports unsupported for predicates outside the partial support list.
    PartialPredicateSupport,
    /// Returns provenance that fails validation.
    InvalidProvenance,
    /// Omits the final keyed result.
    PartialResponse,
    /// Returns [`FrameDecisionEvaluationFailure::DeadlineExceeded`].
    Timeout,
    /// Returns [`FrameDecisionEvaluationFailure::ProviderUnavailable`].
    ProviderOutage,
}

/// Deterministic Jev evaluator with no LLM authority.
#[derive(Debug, Clone)]
pub struct FakeFrameDecisionEvaluator {
    config: FrameDecisionEvaluatorProviderConfig,
    limits: FrameDecisionEvaluatorLimits,
    behavior: FakeJevBehavior,
    partial_support: Vec<LabPredicate>,
    threshold: DecisionThresholdPolicy,
}

impl FakeFrameDecisionEvaluator {
    /// Creates a fake Jev evaluator using pilot threshold defaults.
    #[must_use]
    pub fn new(
        config: FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
        behavior: FakeJevBehavior,
    ) -> Self {
        Self {
            config,
            limits,
            behavior,
            partial_support: Vec::new(),
            threshold: DecisionThresholdPolicy::pilot(),
        }
    }

    /// Limits supported predicates for partial-support scenarios.
    #[must_use]
    pub fn with_partial_predicates(mut self, predicates: Vec<LabPredicate>) -> Self {
        self.partial_support = predicates;
        self.behavior = FakeJevBehavior::PartialPredicateSupport;
        self
    }
}

#[allow(clippy::unused_async_trait_impl)]
impl FrameDecisionEvaluator for FakeFrameDecisionEvaluator {
    fn config(&self) -> &FrameDecisionEvaluatorProviderConfig {
        &self.config
    }

    fn limits(&self) -> FrameDecisionEvaluatorLimits {
        self.limits
    }

    async fn evaluate_frames(
        &self,
        requests: Vec<FrameDecisionEvaluationRequest>,
    ) -> Result<Vec<KeyedFrameDecisionEvaluation>, FrameDecisionEvaluationFailure> {
        match self.behavior {
            FakeJevBehavior::Timeout => Err(FrameDecisionEvaluationFailure::DeadlineExceeded),
            FakeJevBehavior::ProviderOutage => {
                Err(FrameDecisionEvaluationFailure::ProviderUnavailable {
                    message: "jev unavailable".to_owned(),
                })
            }
            FakeJevBehavior::PartialResponse => {
                if requests.is_empty() {
                    return Ok(Vec::new());
                }
                let only = Self::ok_item(self, &requests[0])?;
                Ok(vec![only])
            }
            FakeJevBehavior::InvalidProvenance => {
                requests.iter().map(Self::invalid_provenance_item).collect()
            }
            FakeJevBehavior::PartialPredicateSupport => requests
                .iter()
                .map(|request| {
                    if self.partial_support.contains(request.predicate()) {
                        Self::ok_item(self, request)
                    } else {
                        Ok(unsupported_predicate_result(request, request.predicate()))
                    }
                })
                .collect(),
            FakeJevBehavior::Abstain | FakeJevBehavior::OkPresent => requests
                .iter()
                .map(|request| Self::ok_item(self, request))
                .collect(),
        }
    }
}

impl FakeFrameDecisionEvaluator {
    fn ok_item(
        &self,
        request: &FrameDecisionEvaluationRequest,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        let (noul, outcome_hint) = match self.behavior {
            FakeJevBehavior::Abstain => (Some(0.5), DecisionOutcome::Unknown),
            _ => (Some(0.91), DecisionOutcome::Present),
        };
        let structured = JevStructuredOutput::new(
            noul,
            None,
            None,
            None,
            None,
            Some(FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap()),
            None,
        )
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        })?;
        let outcome = if self.behavior == FakeJevBehavior::Abstain {
            interpret_structured_outcome(&structured, self.threshold)?
        } else {
            interpret_structured_outcome(&structured, self.threshold).unwrap_or(outcome_hint)
        };
        let provenance = JevEvaluationProvenance::new(
            request.target().captured_at(),
            UtcTimestamp::parse("2024-06-01T12:00:02Z").unwrap(),
            JevProducerIdentity::default_production(),
            request.evaluation_id().clone(),
            self.config.config_digest().clone(),
            request.criteria().prompt_digest().clone(),
            request.criteria().criteria_digest().clone(),
            self.config.threshold_policy_digest().clone(),
            request.model_input_transform(),
            Sha256Digest::hash_bytes(b"fake-jev-response"),
            DecisionEvaluationCoverage::Evaluated,
            Option::<&str>::None,
        )
        .map_err(|error| FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        })?;
        KeyedFrameDecisionEvaluation::new(
            request,
            DecisionEvaluationCoverage::Evaluated,
            Some(outcome),
            Some(structured),
            Some(provenance),
            Some(Sha256Digest::hash_bytes(b"fake-jev-response")),
        )
    }

    fn invalid_provenance_item(
        request: &FrameDecisionEvaluationRequest,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        KeyedFrameDecisionEvaluation::new(
            request,
            DecisionEvaluationCoverage::Evaluated,
            Some(DecisionOutcome::Present),
            Some(JevStructuredOutput::empty()),
            None,
            Some(Sha256Digest::hash_bytes(b"fake-jev-response")),
        )
    }
}
