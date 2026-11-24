//! Typed three-predicate evaluator over the TypeSafe System One client.
//!
//! Downstream repositories construct [`TypedLabPredicateJevAdapter`] from a
//! published git revision of `a2a-lab-dev-kit`, never a path dependency. The
//! adapter accepts only the three v1 lab predicates, sends each one as a named
//! System One question, and maps the answer onto the existing coverage,
//! outcome, provenance, and result types. It does not append a
//! [`super::record::FrameDecisionRecord`].
//!
//! The uploaded image is the full frame. A configured ROI stays on the typed
//! result and is not sent as a crop. HTTP, MCP, and A2A remain lab transports.
//! This module does not add settings pages, whole-video indexing or backfill,
//! `AgentCore`, Strands, an AWS SDK client, a judge or other LLM fallback, or a
//! profile 1.3.0 bump.

use std::collections::BTreeMap;

use crate::config::LoadedLabConfig;
use crate::error::A2aLabError;
use crate::id::SavedFrameId;
use crate::images::{
    Image, ImageDecisionHttpLimits, ImageDecisionPrimitiveAnswer, ImageDecisionQuestion,
    ImageTransportConfig, SourceImageDecisionResult, TypeSafeSystemOneClient,
};
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;

use super::config::{FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig};
use super::failure::FrameDecisionEvaluationFailure;
use super::predicate::{
    LabPredicate, PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED,
    PREDICATE_PLATE_MISALIGNED,
};
use super::provider::FrameDecisionEvaluator;
use super::record::{
    DecisionEvaluationCoverage, JevEvaluationProvenance, JevStructuredOutput, LabProbabilityVector,
};
use super::request::{FrameDecisionEvaluationRequest, guard_against_description_evidence};
use super::result::{
    DecisionThresholdPolicy, KeyedFrameDecisionEvaluation, interpret_structured_outcome,
};

/// Live evaluator that asks System One one named question per typed request.
#[derive(Debug, Clone)]
pub struct TypedLabPredicateJevAdapter {
    config: FrameDecisionEvaluatorProviderConfig,
    limits: FrameDecisionEvaluatorLimits,
    client: TypeSafeSystemOneClient,
    model: String,
    threshold: DecisionThresholdPolicy,
    evaluated_at: UtcTimestamp,
    frames: std::collections::HashMap<SavedFrameId, Image>,
}

impl TypedLabPredicateJevAdapter {
    /// Builds an adapter for a config whose predicates are a subset of the three v1 declarations.
    ///
    /// `model` is the evaluator producer revision. [`Self::from_loaded`] uses the
    /// semantic decision model instead.
    pub fn new(
        config: FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
        client: TypeSafeSystemOneClient,
        evaluated_at: UtcTimestamp,
    ) -> Result<Self, A2aLabError> {
        let model = config.producer().model_revision().to_owned();
        Self::assemble(config, limits, client, model, evaluated_at)
    }

    /// Builds an adapter from semantic decision endpoint and model plus the shared TypeSafe key.
    ///
    /// The key is [`LoadedLabConfig::semantic_decision_api_key`]. A missing
    /// semantic block or key is an error and does not reveal a credential.
    pub fn from_loaded(
        loaded: &LoadedLabConfig,
        config: FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
        evaluated_at: UtcTimestamp,
    ) -> Result<Self, A2aLabError> {
        let decisions = loaded
            .config
            .semantic_indexing
            .as_ref()
            .map(|semantic| &semantic.decisions)
            .ok_or_else(|| {
                A2aLabError::invalid("semantic_indexing.decisions", "required field is missing")
            })?;
        let api_key = loaded
            .semantic_decision_api_key()?
            .ok_or_else(|| A2aLabError::invalid("typesafe.api_key", "required field is missing"))?;
        let client = TypeSafeSystemOneClient::new(
            decisions.endpoint(),
            api_key,
            Self::http_limits(limits)?,
        )?
        .with_preamble(decisions.preamble().map(str::to_owned));
        Self::assemble(
            config,
            limits,
            client,
            decisions.model().to_owned(),
            evaluated_at,
        )
    }

    /// Aligns the System One client bounds with the typed evaluator limits.
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

    /// Binds saved-frame revisions to the images the client will send.
    #[must_use]
    pub fn with_frames(mut self, frames: std::collections::HashMap<SavedFrameId, Image>) -> Self {
        self.frames = frames;
        self
    }

    /// Evaluation instant recorded in typed provenance.
    #[must_use]
    pub const fn evaluated_at(&self) -> UtcTimestamp {
        self.evaluated_at
    }

    /// Model sent on the System One request.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Evaluates one typed request through a named System One question.
    ///
    /// The returned evaluation does not append a ledger record. The configured
    /// ROI remains on the typed provenance. The uploaded image is not cropped.
    pub async fn evaluate_frame(
        &self,
        request: &FrameDecisionEvaluationRequest,
        image: &Image,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        self.ensure_predicate(request)?;
        guard_against_description_evidence(request.evidence())?;
        guard_image(self.limits, image)?;
        let questions = questions_for(request.predicate())?;
        let result = self
            .client
            .decide_questions(&self.model, image, &questions)
            .await
            .map_err(map_provider)?;
        self.map_result(request, &questions, &result)
    }

    fn assemble(
        config: FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
        client: TypeSafeSystemOneClient,
        model: String,
        evaluated_at: UtcTimestamp,
    ) -> Result<Self, A2aLabError> {
        guard_supported(config.supported_predicates())?;
        if model.trim().is_empty() {
            return Err(A2aLabError::invalid(
                "model",
                "Jev model revision must not be empty",
            ));
        }
        Ok(Self {
            config,
            limits,
            client,
            model,
            threshold: DecisionThresholdPolicy::pilot(),
            evaluated_at,
            frames: std::collections::HashMap::new(),
        })
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
        questions: &BTreeMap<String, ImageDecisionQuestion>,
        result: &SourceImageDecisionResult,
    ) -> Result<KeyedFrameDecisionEvaluation, FrameDecisionEvaluationFailure> {
        let name = questions.keys().next().ok_or_else(|| {
            FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "typed decision returned no answers".to_owned(),
            }
        })?;
        let answer = result.answers().get(name).ok_or_else(|| {
            FrameDecisionEvaluationFailure::InvalidProvenance {
                message: "typed decision returned no answers".to_owned(),
            }
        })?;
        let structured = structured_from_primitive(answer)?;
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

fn questions_for(
    predicate: &LabPredicate,
) -> Result<BTreeMap<String, ImageDecisionQuestion>, FrameDecisionEvaluationFailure> {
    let (name, instructions) = named_question(predicate)?;
    let mut questions = BTreeMap::new();
    questions.insert(name.to_owned(), noul_question(instructions));
    Ok(questions)
}

fn named_question(
    predicate: &LabPredicate,
) -> Result<(&'static str, &'static str), FrameDecisionEvaluationFailure> {
    let question = match (predicate.id(), predicate.version()) {
        (PREDICATE_PIPETTE_TIP_ATTACHED, 1) => ("pipette_tip_attached", "pipette tip attached"),
        (PREDICATE_FOREIGN_OBJECT_PRESENT, 1) => {
            ("foreign_object_present", "foreign object present")
        }
        (PREDICATE_PLATE_MISALIGNED, 1) => ("plate_misaligned", "plate misaligned"),
        _ => {
            return Err(FrameDecisionEvaluationFailure::UnsupportedPredicate {
                predicate_id: predicate.id().to_owned(),
                predicate_version: predicate.version(),
            });
        }
    };
    Ok(question)
}

fn noul_question(instructions: &str) -> ImageDecisionQuestion {
    ImageDecisionQuestion::Noul {
        instructions: instructions.to_owned(),
        criteria: None,
    }
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

fn structured_from_primitive(
    answer: &ImageDecisionPrimitiveAnswer,
) -> Result<JevStructuredOutput, FrameDecisionEvaluationFailure> {
    match answer {
        ImageDecisionPrimitiveAnswer::Noul { noul } => {
            JevStructuredOutput::new(Some(*noul), None, None, None, None, None, None)
                .map_err(invalid_provenance)
        }
        ImageDecisionPrimitiveAnswer::Choice {
            probabilities,
            confidence,
            ..
        } => {
            let (labels, values): (Vec<_>, Vec<_>) = probabilities
                .iter()
                .map(|(label, value)| (label.clone(), *value))
                .unzip();
            let distribution =
                LabProbabilityVector::new(labels, values).map_err(invalid_provenance)?;
            JevStructuredOutput::new(
                None,
                Some(distribution),
                None,
                None,
                Some(*confidence),
                None,
                None,
            )
            .map_err(invalid_provenance)
        }
        ImageDecisionPrimitiveAnswer::Score {
            score,
            legend,
            probabilities,
            confidence,
        } => {
            let distribution = LabProbabilityVector::new(legend.clone(), probabilities.clone())
                .map_err(invalid_provenance)?;
            JevStructuredOutput::new(
                None,
                None,
                Some(distribution),
                Some(*score),
                Some(*confidence),
                None,
                None,
            )
            .map_err(invalid_provenance)
        }
    }
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
    result: &SourceImageDecisionResult,
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
            FrameDecisionEvaluationFailure::LimitExceeded {
                limit: typed_limit(&limit),
                message,
            }
        }
        ImageDecisionProviderError::DeadlineExceeded { .. } => {
            FrameDecisionEvaluationFailure::DeadlineExceeded
        }
    }
}

fn typed_limit(limit: &str) -> String {
    match limit {
        "request" => "max_request_bytes",
        "image" => "max_image_bytes",
        other => other,
    }
    .to_owned()
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
