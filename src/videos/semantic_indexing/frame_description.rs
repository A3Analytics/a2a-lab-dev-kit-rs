//! Bounded keyed frame-description provider contract (Luna defaults).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::{FrameDescriptionProvenance, FrameRoiTransform};
use crate::videos::model_space::ProvenanceGeneration;
use crate::videos::saved_frame::SavedFrameRef;
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;

/// Default Bedrock Luna model for frame descriptions.
pub const DEFAULT_BEDROCK_DESCRIPTION_MODEL: &str = "global.openai.gpt-5.6-luna";

/// Default OpenAI Luna model for frame descriptions.
pub const DEFAULT_OPENAI_DESCRIPTION_MODEL: &str = "gpt-5.6-luna";

/// Supported description providers with Luna defaults independent of chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameDescriptionBackend {
    /// Amazon Bedrock.
    Bedrock,
    /// OpenAI.
    Openai,
}

impl FrameDescriptionBackend {
    /// Stable provider id.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bedrock => "bedrock",
            Self::Openai => "openai",
        }
    }

    /// Luna model identifier for this backend when none is configured.
    #[must_use]
    pub const fn default_model(self) -> &'static str {
        match self {
            Self::Bedrock => DEFAULT_BEDROCK_DESCRIPTION_MODEL,
            Self::Openai => DEFAULT_OPENAI_DESCRIPTION_MODEL,
        }
    }
}

/// Resolved frame-description settings separate from chat and Jev.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDescriptionProviderConfig {
    backend: FrameDescriptionBackend,
    model: String,
    prompt_digest: Sha256Digest,
    preprocessing_digest: Sha256Digest,
}

impl FrameDescriptionProviderConfig {
    /// Builds config with the backend's Luna default when `model` is `None`.
    pub fn new(
        backend: FrameDescriptionBackend,
        model: Option<impl AsRef<str>>,
        prompt_digest: Sha256Digest,
        preprocessing_digest: Sha256Digest,
    ) -> Result<Self, A2aLabError> {
        let model = model.map_or_else(
            || backend.default_model().to_owned(),
            |value| value.as_ref().to_owned(),
        );
        if model.trim().is_empty() {
            return Err(A2aLabError::invalid(
                "model",
                "description model must not be empty",
            ));
        }
        Ok(Self {
            backend,
            model,
            prompt_digest,
            preprocessing_digest,
        })
    }

    /// Selected backend.
    #[must_use]
    pub const fn backend(&self) -> FrameDescriptionBackend {
        self.backend
    }

    /// Resolved description model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Versioned prompt digest.
    #[must_use]
    pub const fn prompt_digest(&self) -> &Sha256Digest {
        &self.prompt_digest
    }

    /// Recorded preprocessing policy digest.
    #[must_use]
    pub const fn preprocessing_digest(&self) -> &Sha256Digest {
        &self.preprocessing_digest
    }

    /// Configured provider id.
    #[must_use]
    pub fn provider(&self) -> &str {
        self.backend.as_str()
    }
}

/// Finite bounds for description requests and outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDescriptionLimits {
    max_batch_items: u32,
    max_input_bytes: usize,
    max_pixels: u64,
    max_output_bytes: usize,
    max_output_tokens: u32,
    deadline: Duration,
}

impl FrameDescriptionLimits {
    /// Validates description limits.
    pub fn new(
        max_batch_items: u32,
        max_input_bytes: usize,
        max_pixels: u64,
        max_output_bytes: usize,
        max_output_tokens: u32,
        deadline: Duration,
    ) -> Result<Self, A2aLabError> {
        if max_batch_items == 0 {
            return Err(A2aLabError::invalid("max_batch_items", "must be positive"));
        }
        if max_input_bytes == 0 || max_output_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        if max_pixels == 0 {
            return Err(A2aLabError::invalid("max_pixels", "must be positive"));
        }
        if max_output_tokens == 0 {
            return Err(A2aLabError::invalid(
                "max_output_tokens",
                "must be positive",
            ));
        }
        if deadline.is_zero() {
            return Err(A2aLabError::invalid("deadline", "must be positive"));
        }
        Ok(Self {
            max_batch_items,
            max_input_bytes,
            max_pixels,
            max_output_bytes,
            max_output_tokens,
            deadline,
        })
    }

    /// Conservative defaults for lab indexing workers.
    #[must_use]
    pub fn pilot() -> Self {
        Self {
            max_batch_items: 8,
            max_input_bytes: 16 * 1024 * 1024,
            max_pixels: 2_560 * 1_568,
            max_output_bytes: 16 * 1024,
            max_output_tokens: 512,
            deadline: Duration::from_secs(30),
        }
    }

    /// Maximum items per batch call.
    #[must_use]
    pub const fn max_batch_items(self) -> u32 {
        self.max_batch_items
    }

    /// Maximum decoded image bytes accepted for description.
    #[must_use]
    pub const fn max_input_bytes(self) -> usize {
        self.max_input_bytes
    }

    /// Maximum pixel count accepted for description.
    #[must_use]
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }

    /// Maximum persisted description bytes.
    #[must_use]
    pub const fn max_output_bytes(self) -> usize {
        self.max_output_bytes
    }

    /// Maximum description tokens reported by the provider.
    #[must_use]
    pub const fn max_output_tokens(self) -> u32 {
        self.max_output_tokens
    }

    /// Per-batch deadline.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }
}

/// Immutable identity for one description job on a saved revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDescriptionKey {
    frame: SavedFrameRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    roi: Option<FrameRoiTransform>,
    generation: ProvenanceGeneration,
}

impl FrameDescriptionKey {
    /// Builds a description key.
    #[must_use]
    pub fn new(
        frame: SavedFrameRef,
        roi: Option<FrameRoiTransform>,
        generation: ProvenanceGeneration,
    ) -> Self {
        Self {
            frame,
            roi,
            generation,
        }
    }

    /// Saved frame revision.
    #[must_use]
    pub const fn frame(&self) -> &SavedFrameRef {
        &self.frame
    }

    /// Optional ROI on the oriented frame.
    #[must_use]
    pub const fn roi(&self) -> Option<FrameRoiTransform> {
        self.roi
    }

    /// Description provenance generation for this revision.
    #[must_use]
    pub const fn generation(&self) -> ProvenanceGeneration {
        self.generation
    }
}

/// One bounded describe request for a keyed revision.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameDescriptionRequest {
    key: FrameDescriptionKey,
    input_bytes: usize,
    pixels: u64,
}

impl FrameDescriptionRequest {
    /// Creates a request after limit checks.
    pub fn new(
        limits: FrameDescriptionLimits,
        key: FrameDescriptionKey,
        input_bytes: usize,
        pixels: u64,
    ) -> Result<Self, SemanticIndexingFailure> {
        if input_bytes > limits.max_input_bytes() {
            return Err(SemanticIndexingFailure::LimitExceeded {
                limit: "max_input_bytes".to_owned(),
                message: format!(
                    "input_bytes {input_bytes} exceeds {}",
                    limits.max_input_bytes()
                ),
            });
        }
        if pixels > limits.max_pixels() {
            return Err(SemanticIndexingFailure::LimitExceeded {
                limit: "max_pixels".to_owned(),
                message: format!("pixels {pixels} exceeds {}", limits.max_pixels()),
            });
        }
        Ok(Self {
            key,
            input_bytes,
            pixels,
        })
    }

    /// Target revision key.
    #[must_use]
    pub const fn key(&self) -> &FrameDescriptionKey {
        &self.key
    }

    /// Declared input byte size for limit accounting.
    #[must_use]
    pub fn input_bytes(self) -> usize {
        self.input_bytes
    }

    /// Declared pixel count for limit accounting.
    #[must_use]
    pub fn pixels(self) -> u64 {
        self.pixels
    }
}

/// Provider output for one keyed revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KeyedFrameDescription {
    key: FrameDescriptionKey,
    text: String,
    resolved_provider: String,
    resolved_model: String,
    prompt_digest: Sha256Digest,
    preprocessing_digest: Sha256Digest,
}

impl KeyedFrameDescription {
    /// Builds a response without output-length checks (contract tests only).
    pub(crate) fn unchecked(
        config: &FrameDescriptionProviderConfig,
        key: FrameDescriptionKey,
        text: String,
    ) -> Self {
        Self {
            key,
            text,
            resolved_provider: config.provider().to_owned(),
            resolved_model: config.model().to_owned(),
            prompt_digest: config.prompt_digest().clone(),
            preprocessing_digest: config.preprocessing_digest().clone(),
        }
    }

    /// Validates bounded text and digests for one keyed response.
    pub fn new(
        limits: FrameDescriptionLimits,
        config: &FrameDescriptionProviderConfig,
        key: FrameDescriptionKey,
        text: impl Into<String>,
    ) -> Result<Self, SemanticIndexingFailure> {
        let text = text.into();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(SemanticIndexingFailure::EmptyDescription);
        }
        if text.len() > limits.max_output_bytes() {
            return Err(SemanticIndexingFailure::OversizedOutput {
                message: format!(
                    "description length {} exceeds {}",
                    text.len(),
                    limits.max_output_bytes()
                ),
            });
        }
        Ok(Self {
            key,
            text,
            resolved_provider: config.provider().to_owned(),
            resolved_model: config.model().to_owned(),
            prompt_digest: config.prompt_digest().clone(),
            preprocessing_digest: config.preprocessing_digest().clone(),
        })
    }

    /// Target key.
    #[must_use]
    pub const fn key(&self) -> &FrameDescriptionKey {
        &self.key
    }

    /// Bounded description text (retrieval metadata only).
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Provider that produced this description.
    #[must_use]
    pub fn resolved_provider(&self) -> &str {
        &self.resolved_provider
    }

    /// Resolved model name.
    #[must_use]
    pub fn resolved_model(&self) -> &str {
        &self.resolved_model
    }

    /// Prompt digest carried with the description.
    #[must_use]
    pub const fn prompt_digest(&self) -> &Sha256Digest {
        &self.prompt_digest
    }

    /// Preprocessing digest carried with the description.
    #[must_use]
    pub const fn preprocessing_digest(&self) -> &Sha256Digest {
        &self.preprocessing_digest
    }

    /// Builds durable provenance for persistence layers.
    pub fn to_provenance(&self) -> Result<FrameDescriptionProvenance, A2aLabError> {
        FrameDescriptionProvenance::new(
            &self.resolved_provider,
            &self.resolved_model,
            self.prompt_digest.clone(),
            self.preprocessing_digest.clone(),
            self.key.roi,
            Sha256Digest::hash_bytes(self.text.as_bytes()),
            self.key.generation,
        )
    }
}

/// Rejects partial, duplicate, or shuffled batch responses.
pub fn validate_description_batch(
    limits: FrameDescriptionLimits,
    requests: &[FrameDescriptionRequest],
    responses: &[KeyedFrameDescription],
) -> Result<Vec<KeyedFrameDescription>, SemanticIndexingFailure> {
    if requests.len() > usize::try_from(limits.max_batch_items()).unwrap_or(usize::MAX) {
        return Err(SemanticIndexingFailure::LimitExceeded {
            limit: "max_batch_items".to_owned(),
            message: format!(
                "batch size {} exceeds {}",
                requests.len(),
                limits.max_batch_items()
            ),
        });
    }
    if responses.len() != requests.len() {
        return Err(SemanticIndexingFailure::PartialResponse {
            message: format!(
                "expected {} keyed descriptions, got {}",
                requests.len(),
                responses.len()
            ),
        });
    }

    let mut expected = std::collections::HashSet::new();
    for request in requests {
        let token = description_key_token(request.key());
        if !expected.insert(token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: "duplicate request key in description batch".to_owned(),
            });
        }
    }

    let mut matched = std::collections::HashSet::new();
    for response in responses {
        if response.text().trim().is_empty() {
            return Err(SemanticIndexingFailure::EmptyDescription);
        }
        if response.text().len() > limits.max_output_bytes() {
            return Err(SemanticIndexingFailure::OversizedOutput {
                message: format!(
                    "description length {} exceeds {}",
                    response.text().len(),
                    limits.max_output_bytes()
                ),
            });
        }
        let token = description_key_token(response.key());
        if !expected.contains(&token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: format!(
                    "response key {} was not requested",
                    response.key().frame().frame_id()
                ),
            });
        }
        if !matched.insert(token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: "duplicate response key in description batch".to_owned(),
            });
        }
    }

    if matched.len() != expected.len() {
        return Err(SemanticIndexingFailure::PartialResponse {
            message: "one or more requested description keys are missing".to_owned(),
        });
    }

    Ok(responses.to_vec())
}

pub(crate) fn description_key_token(key: &FrameDescriptionKey) -> String {
    let roi = key.roi().map_or_else(
        || "full".to_owned(),
        |value| format!("{:?}", value.bounds()),
    );
    format!(
        "{}:{}:{}",
        key.frame().frame_id(),
        key.generation().get(),
        roi
    )
}

/// Generates bounded visual descriptions for saved frame revisions.
///
/// Implementations must return retrieval metadata only. They do not append Jev
/// decisions or substitute for the decision evaluator.
pub trait FrameDescriptionProvider: Send + Sync {
    /// Resolved configuration for this provider instance.
    fn config(&self) -> &FrameDescriptionProviderConfig;

    /// Active limits for this provider instance.
    fn limits(&self) -> FrameDescriptionLimits;

    /// Describes one batch of keyed frame revisions.
    fn describe_frames(
        &self,
        requests: Vec<FrameDescriptionRequest>,
    ) -> impl std::future::Future<
        Output = Result<Vec<KeyedFrameDescription>, SemanticIndexingFailure>,
    > + Send;
}
