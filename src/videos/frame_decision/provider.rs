//! Jev-only frame decision evaluator provider trait.

use crate::videos::frame_decision::config::{
    FrameDecisionEvaluatorCapabilities, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig,
};
use crate::videos::frame_decision::failure::FrameDecisionEvaluationFailure;
use crate::videos::frame_decision::request::FrameDecisionEvaluationRequest;
use crate::videos::frame_decision::result::KeyedFrameDecisionEvaluation;

/// Evaluates lab predicates against exact saved image evidence using Jev only.
///
/// Implementations must not consume Luna descriptions, chat output, embedding
/// similarity, or generated criteria. They do not arbitrate or repair decisions.
pub trait FrameDecisionEvaluator: Send + Sync {
    /// Resolved configuration for this evaluator instance.
    fn config(&self) -> &FrameDecisionEvaluatorProviderConfig;

    /// Active limits for this evaluator instance.
    fn limits(&self) -> FrameDecisionEvaluatorLimits;

    /// Advertised production capabilities.
    #[must_use]
    fn capabilities(&self) -> FrameDecisionEvaluatorCapabilities {
        FrameDecisionEvaluatorCapabilities::from_config(self.config(), self.limits())
    }

    /// Evaluates one batch of keyed image evidence requests.
    fn evaluate_frames(
        &self,
        requests: Vec<FrameDecisionEvaluationRequest>,
    ) -> impl std::future::Future<
        Output = Result<Vec<KeyedFrameDecisionEvaluation>, FrameDecisionEvaluationFailure>,
    > + Send;
}
