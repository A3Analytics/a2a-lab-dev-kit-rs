//! Compatibility adapter from the typed three-predicate live evaluator to the
//! generic synchronous JEV client.
//!
//! Downstream repositories construct [`TypedLabPredicateJevAdapter`] from a
//! published git revision of `a2a-lab-dev-kit`, never a path dependency. The
//! adapter accepts only the three v1 lab predicates, sends each validated
//! request through [`crate::images::ImageDecisionHttpClient`], and maps the
//! generic answer onto the existing coverage, outcome, provenance, and result
//! types. It does not append a [`super::record::FrameDecisionRecord`]. Generic
//! image decisions stay side-effect free.
//!
//! A full-frame model input omits the ROI and is the full source image. Answer
//! boxes are already normalized to that image by the generic client. HTTP, MCP,
//! and A2A remain lab transports. This module does not add settings pages,
//! whole-video indexing or backfill, `AgentCore`, Strands, an AWS SDK client, a
//! judge or other LLM fallback, or a profile 1.3.0 bump.

use std::collections::HashMap;

use crate::error::A2aLabError;
use crate::id::SavedFrameId;
use crate::images::{
    Image, ImageDecisionAnswer, ImageDecisionHttpClient, ImageDecisionHttpLimits,
    ImageDecisionModelInput, ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi,
    ImageTransportConfig,
};
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::FrameRoiTransform;

use super::config::{FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig};
use super::failure::FrameDecisionEvaluationFailure;
use super::predicate::LabPredicate;
use super::provider::FrameDecisionEvaluator;
use super::record::{
    DecisionEvaluationCoverage, JevEvaluationProvenance, JevStructuredOutput, LabProbabilityVector,
};
use super::request::{
    FrameDecisionEvaluationRequest, guard_against_description_evidence, subject_token,
};
use super::result::{
    DecisionThresholdPolicy, KeyedFrameDecisionEvaluation, interpret_structured_outcome,
};

/// Live evaluator that asks the generic JEV client one question per typed request.
#[derive(Debug, Clone)]
pub struct TypedLabPredicateJevAdapter {
    config: FrameDecisionEvaluatorProviderConfig,
    limits: FrameDecisionEvaluatorLimits,
    client: ImageDecisionHttpClient,
    threshold: DecisionThresholdPolicy,
    evaluated_at: UtcTimestamp,
    frames: HashMap<SavedFrameId, Image>,
}

impl TypedLabPredicateJevAdapter {
    /// Builds an adapter for a config whose predicates are a subset of the three v1 declarations.
    pub fn new(
        config: FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
        client: ImageDecisionHttpClient,
        evaluated_at: UtcTimestamp,
    ) -> Result<Self, A2aLabError> {
        guard_supported(config.supported_predicates())?;
        Ok(Self {
            config,
            limits,
            client,
            threshold: DecisionThresholdPolicy::pilot(),
            evaluated_at,
            frames: HashMap::new(),
        })
    }

    /// Aligns the generic HTTP client bounds with the typed evaluator limits.
    pub fn http_limits(
        limits: FrameDecisionEvaluatorLimits,
    ) -> Result<ImageDecisionHttpLimits, A2aLabError> {
        let max_image_bytes = u64::try_from(limits.max_image_bytes()).map_err(|_| {
            A2aLabError::invalid("max_image_bytes", "exceeds the decision client range")
        })?;
        ImageDecisionHttpLimits::new(
            ImageTransportConfig::new(max_image_bytes)?,
            limits.max_request_bytes(),
            limits.max_jev_response_bytes(),
            limits.deadline(),
        )
    }

    /// Binds saved-frame revisions to the images the generic client will send.
    #[must_use]
    pub fn with_frames(mut self, frames: HashMap<SavedFrameId, Image>) -> Self {
        self.frames = frames;
        self
    }

    /// Evaluation instant recorded in typed provenance.
    #[must_use]
    pub const fn evaluated_at(&self) -> UtcTimestamp {
        self.evaluated_at
    }

    /// Evaluates one typed request through the generic client.
    ///
    /// The returned evaluation does not append a ledger record.
    pub async fn evaluate_frame(
        &self,
        request: &FrameDecisionEvaluationRequest,
        image: &Image,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        self.ensure_predicate(request)?;
        guard_against_description_evidence(request.evidence())?;
        guard_image(self.limits, image)?;
        let generic = generic_request(&self.config, request, image)?;
        guard_encoded_request(&generic, self.limits)?;
        let result = self.client.decide(&generic).await.map_err(map_provider)?;
        self.map_result(request, &result)
    }

    fn ensure_predicate(
        &self,
        request: &FrameDecisionEvaluationRequest,
    ) -> Result<(), FrameDecisionEvaluationFailure> {
        if self.config.supports_predicate(request.predicate()) {
            return Ok(());
        }
        Err(FrameDecisionEvaluationFailure::UnsupportedPredicate {
            predicate_id: request.predicate().id().to_owned(),
            predicate_version: request.predicate().version(),
        })
    }

    fn map_result(
        &self,
        request: &FrameDecisionEvaluationRequest,
        result: &ImageDecisionResult,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        let answer = result.answers().first().ok_or_else(|| {
            FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "generic decision returned no answers".to_owned(),
            }
        })?;
        let structured = structured_from_answer(answer)?;
        let outcome = interpret_structured_outcome(&structured, self.threshold)?;
        let digest = response_digest(result)?;
        let provenance = provenance_for(&self.config, self.evaluated_at, request, &digest)?;
        KeyedFrameDecisionEvaluation::new(
            request,
            DecisionEvaluationCoverage::Evaluated,
            Some(outcome),
            Some(structured),
            Some(provenance),
            Some(digest),
        )
    }
}

impl FrameDecisionEvaluator for TypedLabPredicateJevAdapter {
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
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let max_batch = usize::try_from(self.limits.max_batch_items()).unwrap_or(usize::MAX);
        if requests.len() > max_batch {
            return Err(limit_exceeded(
                "max_batch_items",
                format!(
                    "batch size {} exceeds {}",
                    requests.len(),
                    self.limits.max_batch_items()
                ),
            ));
        }
        let mut evaluations = Vec::with_capacity(requests.len());
        for request in &requests {
            let image = self.frame_image(request).ok_or_else(|| {
                FrameDecisionEvaluationFailure::InvalidEvidence {
                    message: "saved frame image is not bound for this evaluation".to_owned(),
                }
            })?;
            evaluations.push(self.evaluate_frame(request, &image).await?);
        }
        Ok(evaluations)
    }
}

impl TypedLabPredicateJevAdapter {
    fn frame_image(&self, request: &FrameDecisionEvaluationRequest) -> Option<Image> {
        self.frames.get(request.target().frame_id()).cloned()
    }
}

fn guard_supported(predicates: &[LabPredicate]) -> Result<(), A2aLabError> {
    let allowed = [
        LabPredicate::pipette_tip_attached_v1(),
        LabPredicate::foreign_object_present_v1(),
        LabPredicate::plate_misaligned_v1(),
    ];
    let mut seen = [false; 3];
    if predicates.is_empty() || predicates.len() > allowed.len() {
        return Err(unsupported_predicates());
    }
    for predicate in predicates {
        let Some(index) = allowed.iter().position(|item| item == predicate) else {
            return Err(unsupported_predicates());
        };
        if seen[index] {
            return Err(unsupported_predicates());
        }
        seen[index] = true;
    }
    Ok(())
}

fn unsupported_predicates() -> A2aLabError {
    A2aLabError::invalid(
        "supported_predicates",
        "typed live evaluator only supports the three v1 lab predicates",
    )
}

fn generic_request(
    config: &FrameDecisionEvaluatorProviderConfig,
    request: &FrameDecisionEvaluationRequest,
    image: &Image,
) -> Result<ImageDecisionRequest, FrameDecisionEvaluationFailure> {
    let model =
        ImageDecisionModelInput::new(config.producer().model_revision(), model_roi(request)?)
            .map_err(invalid_evidence)?;
    ImageDecisionRequest::new(image.clone(), decision_question(request), model)
        .map_err(invalid_evidence)
}

fn model_roi(
    request: &FrameDecisionEvaluationRequest,
) -> Result<Option<ImageDecisionRoi>, FrameDecisionEvaluationFailure> {
    let transform = request.model_input_transform();
    if is_full_frame(transform) {
        return Ok(None);
    }
    ImageDecisionRoi::new(transform.bounds())
        .map(Some)
        .map_err(invalid_evidence)
}

fn is_full_frame(transform: FrameRoiTransform) -> bool {
    transform.bounds().map(f32::to_bits)
        == FrameRoiTransform::full_frame().bounds().map(f32::to_bits)
}

fn decision_question(request: &FrameDecisionEvaluationRequest) -> String {
    format!(
        "{} v{} {}",
        request.predicate().id(),
        request.predicate().version(),
        subject_token(request.subject())
    )
}

fn guard_image(
    limits: FrameDecisionEvaluatorLimits,
    image: &Image,
) -> Result<(), FrameDecisionEvaluationFailure> {
    if image.data().len() > limits.max_image_bytes() {
        return Err(limit_exceeded(
            "max_image_bytes",
            format!(
                "image bytes {} exceed {}",
                image.data().len(),
                limits.max_image_bytes()
            ),
        ));
    }
    let edge = image.descriptor().width().max(image.descriptor().height());
    if edge < limits.min_edge_pixels() || edge > limits.max_edge_pixels() {
        return Err(limit_exceeded(
            "edge_pixels",
            format!(
                "edge pixels {edge} outside {}..={}",
                limits.min_edge_pixels(),
                limits.max_edge_pixels()
            ),
        ));
    }
    Ok(())
}

fn guard_encoded_request(
    request: &ImageDecisionRequest,
    limits: FrameDecisionEvaluatorLimits,
) -> Result<(), FrameDecisionEvaluationFailure> {
    let encoded = serde_json::to_vec(request).map_err(|error| {
        FrameDecisionEvaluationFailure::InvalidEvidence {
            message: error.to_string(),
        }
    })?;
    if encoded.len() > limits.max_request_bytes() {
        return Err(limit_exceeded(
            "max_request_bytes",
            format!(
                "request bytes {} exceed {}",
                encoded.len(),
                limits.max_request_bytes()
            ),
        ));
    }
    Ok(())
}

fn structured_from_answer(
    answer: &ImageDecisionAnswer,
) -> Result<JevStructuredOutput, FrameDecisionEvaluationFailure> {
    let evidence_box = answer
        .source_box()
        .map(|roi| FrameRoiTransform::new(roi.bounds()))
        .transpose()
        .map_err(invalid_provenance)?;
    let distribution = if answer.probability().is_none() {
        choice_distribution(answer.text())?
    } else {
        None
    };
    JevStructuredOutput::new(
        answer.probability(),
        distribution,
        None,
        None,
        None,
        evidence_box,
        None,
    )
    .map_err(invalid_provenance)
}

fn choice_distribution(
    text: Option<&str>,
) -> Result<Option<LabProbabilityVector>, FrameDecisionEvaluationFailure> {
    let (present, absent) = match text {
        Some("present") => (1.0, 0.0),
        Some("absent") => (0.0, 1.0),
        _ => return Ok(None),
    };
    LabProbabilityVector::new(
        vec!["present".to_owned(), "absent".to_owned()],
        vec![present, absent],
    )
    .map(Some)
    .map_err(invalid_provenance)
}

fn provenance_for(
    config: &FrameDecisionEvaluatorProviderConfig,
    evaluated_at: UtcTimestamp,
    request: &FrameDecisionEvaluationRequest,
    digest: &Sha256Digest,
) -> Result<JevEvaluationProvenance, FrameDecisionEvaluationFailure> {
    JevEvaluationProvenance::new(
        request.target().captured_at(),
        evaluated_at,
        config.producer().clone(),
        request.evaluation_id().clone(),
        config.config_digest().clone(),
        request.criteria().prompt_digest().clone(),
        request.criteria().criteria_digest().clone(),
        config.threshold_policy_digest().clone(),
        request.model_input_transform(),
        digest.clone(),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .map_err(invalid_provenance)
}

fn response_digest(
    result: &ImageDecisionResult,
) -> Result<Sha256Digest, FrameDecisionEvaluationFailure> {
    let bytes = serde_json::to_vec(result).map_err(|error| {
        FrameDecisionEvaluationFailure::InvalidProvenance {
            message: error.to_string(),
        }
    })?;
    Ok(Sha256Digest::hash_bytes(&bytes))
}

fn map_provider(
    error: crate::images::ImageDecisionProviderError,
) -> FrameDecisionEvaluationFailure {
    use crate::images::ImageDecisionProviderError;
    match error {
        ImageDecisionProviderError::Unavailable { message } => {
            FrameDecisionEvaluationFailure::ProviderUnavailable { message }
        }
        ImageDecisionProviderError::InvalidAnswer { message } => {
            FrameDecisionEvaluationFailure::InvalidProvenance { message }
        }
        ImageDecisionProviderError::LimitExceeded { limit, message } => {
            FrameDecisionEvaluationFailure::LimitExceeded { limit, message }
        }
        ImageDecisionProviderError::DeadlineExceeded { .. } => {
            FrameDecisionEvaluationFailure::DeadlineExceeded
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn invalid_evidence(error: A2aLabError) -> FrameDecisionEvaluationFailure {
    FrameDecisionEvaluationFailure::InvalidEvidence {
        message: error.to_string(),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn invalid_provenance(error: A2aLabError) -> FrameDecisionEvaluationFailure {
    FrameDecisionEvaluationFailure::InvalidProvenance {
        message: error.to_string(),
    }
}

fn limit_exceeded(limit: &str, message: String) -> FrameDecisionEvaluationFailure {
    FrameDecisionEvaluationFailure::LimitExceeded {
        limit: limit.to_owned(),
        message,
    }
}
