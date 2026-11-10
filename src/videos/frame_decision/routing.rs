//! Routes evaluation only to the registered Jev evaluator.

use crate::videos::frame_decision::failure::FrameDecisionEvaluationFailure;
use crate::videos::frame_decision::provider::FrameDecisionEvaluator;
use crate::videos::frame_decision::request::FrameDecisionEvaluationRequest;
use crate::videos::frame_decision::result::{
    KeyedFrameDecisionEvaluation, validate_evaluation_batch,
};

/// Counts evaluator routing for contract tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameDecisionRoutingAudit {
    jev_calls: u32,
}

impl FrameDecisionRoutingAudit {
    /// Jev evaluations attempted by the router.
    #[must_use]
    pub const fn jev_calls(self) -> u32 {
        self.jev_calls
    }
}

/// Production router with a single Jev evaluator and no alternate adapters.
pub struct FrameDecisionEvaluationRouter<E: FrameDecisionEvaluator> {
    evaluator: E,
    audit: FrameDecisionRoutingAudit,
}

impl<E: FrameDecisionEvaluator> FrameDecisionEvaluationRouter<E> {
    /// Wraps the sole authorized evaluator.
    #[must_use]
    pub fn new(evaluator: E) -> Self {
        Self {
            evaluator,
            audit: FrameDecisionRoutingAudit::default(),
        }
    }

    /// Routing audit counters since construction.
    #[must_use]
    pub const fn audit(&self) -> FrameDecisionRoutingAudit {
        self.audit
    }

    /// Evaluates requests exclusively through Jev.
    pub async fn evaluate_frames(
        &mut self,
        requests: Vec<FrameDecisionEvaluationRequest>,
    ) -> Result<Vec<KeyedFrameDecisionEvaluation>, FrameDecisionEvaluationFailure> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let limits = self.evaluator.limits();
        if requests.len() > usize::try_from(limits.max_batch_items()).unwrap_or(usize::MAX) {
            return Err(FrameDecisionEvaluationFailure::LimitExceeded {
                limit: "max_batch_items".to_owned(),
                message: format!(
                    "batch size {} exceeds {}",
                    requests.len(),
                    limits.max_batch_items()
                ),
            });
        }
        for request in &requests {
            if !self
                .evaluator
                .config()
                .supports_predicate(request.predicate())
            {
                return Err(FrameDecisionEvaluationFailure::UnsupportedPredicate {
                    predicate_id: request.predicate().id().to_owned(),
                    predicate_version: request.predicate().version(),
                });
            }
        }
        self.audit.jev_calls += 1;
        let responses = self.evaluator.evaluate_frames(requests.clone()).await?;
        validate_evaluation_batch(&requests, &responses)
    }
}
