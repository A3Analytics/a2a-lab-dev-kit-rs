//! v2 `semantic_indexing` file contract: Luna descriptions, text embeddings, Jev decisions.

use jiff::{Span, SpanRelativeTo};
use serde_json::Value;

use crate::error::A2aLabError;
use crate::{
    DEFAULT_JEV_MODEL_REVISION, EmbeddingMetric, FrameDescriptionBackend,
    JEV_EVALUATOR_PROVIDER_ID, MAX_SEMANTIC_CANDIDATES, MAX_SEMANTIC_NEIGHBOR_FRAMES,
    MAX_SEMANTIC_TOP_K, ModelSpaceIdentity, ProvenanceGeneration, SemanticProvenanceSelector,
    VectorNormalization, parse_decision_evaluator_provider,
};

use super::duration::ConfigDuration;
use super::{object_at, parse_u32, reject_unknown_keys};

/// Default semantic description backend when the file omits `description.provider`.
pub const DEFAULT_SEMANTIC_DESCRIPTION_BACKEND: FrameDescriptionBackend =
    FrameDescriptionBackend::Bedrock;

/// Supported v2 configuration schema version.
pub const A2A_LAB_CONFIG_SCHEMA_VERSION_V2: u32 = 2;

/// Parsed v2 semantic-indexing block independent from chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticIndexingConfig {
    /// Luna description overrides separate from chat.
    pub description: SemanticDescriptionConfig,
    /// Text-embedding provider/model/space; never used for decisions.
    pub embeddings: Option<SemanticEmbeddingsConfig>,
    /// Jev-only decision evaluator settings.
    pub decisions: SemanticDecisionsConfig,
    /// Resolved per-source schedules; populated from `videos.sources`.
    pub schedules: Vec<SemanticStreamSchedule>,
    /// Finite worker, search, ROI, storage, and decision budgets.
    pub limits: SemanticIndexingLimits,
}

/// One stream's independent semantic stage schedules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticStreamSchedule {
    pub(crate) stream: String,
    pub(crate) description: SemanticSchedule,
    pub(crate) embeddings: SemanticSchedule,
    pub(crate) description_model: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_configured: bool,
}

impl SemanticStreamSchedule {
    /// Creates the default every-saved-frame policy for one stream.
    #[must_use]
    pub fn every_saved_frame(stream: impl Into<String>) -> Self {
        Self {
            stream: stream.into(),
            description: SemanticSchedule::EverySavedFrame,
            embeddings: SemanticSchedule::EverySavedFrame,
            description_model: None,
            embedding_model: None,
            embedding_configured: false,
        }
    }

    /// Stable source/stream identifier.
    #[must_use]
    pub fn stream(&self) -> &str {
        &self.stream
    }

    /// Frame-description schedule.
    #[must_use]
    pub const fn description(&self) -> SemanticSchedule {
        self.description
    }

    /// Text-embedding schedule.
    #[must_use]
    pub const fn embeddings(&self) -> SemanticSchedule {
        self.embeddings
    }

    /// Optional source-specific description model.
    #[must_use]
    pub fn description_model(&self) -> Option<&str> {
        self.description_model.as_deref()
    }

    /// Optional source-specific embedding model.
    #[must_use]
    pub fn embedding_model(&self) -> Option<&str> {
        self.embedding_model.as_deref()
    }

    /// Whether an embedding schedule/model key was explicitly configured.
    #[must_use]
    pub const fn embedding_configured(&self) -> bool {
        self.embedding_configured
    }
}

/// A semantic stage sampling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticSchedule {
    /// Select every retained saved frame.
    EverySavedFrame,
    /// Select the first retained saved frame at or after each due instant.
    Interval { micros: u64 },
}

impl SemanticSchedule {
    /// Interval in microseconds, or `None` for every-frame selection.
    #[must_use]
    pub const fn interval_micros(self) -> Option<u64> {
        match self {
            Self::EverySavedFrame => None,
            Self::Interval { micros } => Some(micros),
        }
    }
}

/// Frame-description file settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDescriptionConfig {
    backend: FrameDescriptionBackend,
    model: String,
    credentials: Option<CredentialReference>,
}

impl SemanticDescriptionConfig {
    /// Configured description backend.
    #[must_use]
    pub const fn backend(&self) -> FrameDescriptionBackend {
        self.backend
    }

    /// Resolved description model (Luna default when omitted in the file).
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Optional credential reference separate from chat.
    #[must_use]
    pub fn credentials(&self) -> Option<&CredentialReference> {
        self.credentials.as_ref()
    }
}

/// Text-embedding file settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticEmbeddingsConfig {
    provider: String,
    model: String,
    space: EmbeddingSpaceConfig,
    credentials: Option<CredentialReference>,
}

impl SemanticEmbeddingsConfig {
    /// Operator-selected embedding provider id.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Resolved embedding model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Compatible vector index space.
    #[must_use]
    pub const fn space(&self) -> &EmbeddingSpaceConfig {
        &self.space
    }

    /// Optional credential reference separate from chat.
    #[must_use]
    pub fn credentials(&self) -> Option<&CredentialReference> {
        self.credentials.as_ref()
    }

    /// Builds the runtime model-space identity after file validation.
    pub fn model_space_identity(&self) -> Result<ModelSpaceIdentity, A2aLabError> {
        ModelSpaceIdentity::new(
            &self.provider,
            &self.model,
            self.space.dimensions,
            self.space.normalization,
            self.space.metric,
            self.space.generation,
        )
    }
}

/// Embedding index space fields from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingSpaceConfig {
    pub dimensions: u32,
    pub normalization: VectorNormalization,
    pub metric: EmbeddingMetric,
    pub generation: ProvenanceGeneration,
}

/// Jev decision evaluator file settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDecisionsConfig {
    provider: String,
    model: String,
    credentials: Option<CredentialReference>,
}

impl SemanticDecisionsConfig {
    /// Always `jev` after validation.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Pinned Jev model revision.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Optional credential reference separate from chat.
    #[must_use]
    pub fn credentials(&self) -> Option<&CredentialReference> {
        self.credentials.as_ref()
    }
}

/// Non-secret credential reference by environment variable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialReference {
    env_var: String,
}

impl CredentialReference {
    /// Referenced environment variable name (not a secret value).
    #[must_use]
    pub fn env_var(&self) -> &str {
        &self.env_var
    }
}

/// How on-demand decision jobs cover saved frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionCoveragePolicy {
    /// Evaluate every saved frame when a durable job exists.
    AllSavedFrames,
    /// Evaluate only explicit operator or runner requests.
    OnDemand,
    /// Sample a bounded fraction per source/time window.
    Sampled,
}

/// Combined finite limits for indexing workers, search, ROI, storage, and decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticIndexingLimits {
    pub worker: SemanticWorkerLimits,
    pub search: SemanticIndexingSearchLimits,
    pub roi: SemanticRoiLimits,
    pub storage: SemanticStorageLimits,
    pub decisions: SemanticDecisionLimits,
}

/// Indexing worker batch, concurrency, backlog, retry, and deadline bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticWorkerLimits {
    pub max_batch_items: u32,
    pub max_concurrency: u32,
    pub max_queue_backlog: u32,
    pub max_retry_attempts: u32,
    pub job_deadline: ConfigDuration,
    pub retry_backoff: ConfigDuration,
}

/// Search query window and ranking budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticIndexingSearchLimits {
    pub top_k: u32,
    pub candidate_limit: u32,
    pub max_neighbor_frames: u32,
    pub query_window: ConfigDuration,
}

/// ROI extraction bounds for saved-frame evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRoiLimits {
    pub min_edge_pixels: u32,
    pub max_edge_pixels: u32,
    pub max_rois_per_frame: u32,
}

impl Default for SemanticRoiLimits {
    fn default() -> Self {
        Self {
            min_edge_pixels: 280,
            max_edge_pixels: 1568,
            max_rois_per_frame: 8,
        }
    }
}

/// On-disk vector index budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticStorageLimits {
    pub vector_index_path: String,
    pub max_disk_bytes: u64,
}

/// Jev evaluation concurrency, deadlines, and coverage policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDecisionLimits {
    pub max_batch_items: u32,
    pub max_concurrency: u32,
    pub job_deadline: ConfigDuration,
    pub coverage_policy: DecisionCoveragePolicy,
}

pub(crate) fn parse_semantic_indexing(
    value: Option<&Value>,
) -> Result<Option<SemanticIndexingConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = object_at("semantic_indexing", value)?;
    reject_forbidden_semantic_keys(map)?;
    reject_unknown_keys(
        map,
        "semantic_indexing",
        &["description", "embeddings", "decisions", "limits"],
    )?;
    let description = parse_description(map.get("description"))?;
    let embeddings = match map.get("embeddings") {
        None | Some(Value::Null) => None,
        Some(_) => Some(parse_embeddings(map.get("embeddings"))?),
    };
    let decisions = parse_decisions(map.get("decisions"))?;
    let limits = parse_limits(map.get("limits"))?;
    let config = SemanticIndexingConfig {
        description,
        embeddings,
        decisions,
        schedules: Vec::new(),
        limits,
    };
    config.validate_before_startup()?;
    Ok(Some(config))
}

pub(crate) fn parse_schedule(
    value: Option<&Value>,
    field: &'static str,
) -> Result<SemanticSchedule, A2aLabError> {
    let Some(value) = value else {
        return Ok(SemanticSchedule::EverySavedFrame);
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(field, "must be a schedule string"));
    };
    if text == "every_saved_frame" {
        return Ok(SemanticSchedule::EverySavedFrame);
    }
    let span: Span = text.parse().map_err(|_| {
        A2aLabError::invalid(
            field,
            "must be every_saved_frame or a positive finite duration",
        )
    })?;
    if span.get_years() != 0 || span.get_months() != 0 {
        return Err(A2aLabError::invalid(
            field,
            "must be every_saved_frame or a positive finite duration",
        ));
    }
    let duration = span
        .to_duration(SpanRelativeTo::days_are_24_hours())
        .map_err(|_| {
            A2aLabError::invalid(
                field,
                "must be every_saved_frame or a positive finite duration",
            )
        })?;
    let micros = u64::try_from(duration.as_micros()).map_err(|_| {
        A2aLabError::invalid(
            field,
            "must be every_saved_frame or a positive finite duration",
        )
    })?;
    if micros == 0 {
        return Err(A2aLabError::invalid(
            field,
            "must be every_saved_frame or a positive finite duration",
        ));
    }
    Ok(SemanticSchedule::Interval { micros })
}

pub(crate) fn parse_semantic_indexing_required(
    value: Option<&Value>,
) -> Result<SemanticIndexingConfig, A2aLabError> {
    parse_semantic_indexing(value)?
        .ok_or_else(|| A2aLabError::invalid("semantic_indexing", "required field is missing"))
}

impl SemanticIndexingConfig {
    /// Adds resolved source schedules without exposing a JSON top-level map.
    #[must_use]
    pub fn with_schedules(mut self, schedules: Vec<SemanticStreamSchedule>) -> Self {
        self.schedules = schedules;
        self
    }

    /// Returns a source schedule, defaulting both stages to every saved frame.
    #[must_use]
    pub fn schedule_for(&self, stream: &str) -> SemanticStreamSchedule {
        self.schedules
            .iter()
            .find(|schedule| schedule.stream() == stream)
            .cloned()
            .unwrap_or_else(|| SemanticStreamSchedule::every_saved_frame(stream))
    }

    /// Active model space and description generation for semantic search clients.
    pub fn active_search_provenance(&self) -> Result<SemanticProvenanceSelector, A2aLabError> {
        let embeddings = self
            .embeddings
            .as_ref()
            .ok_or_else(|| A2aLabError::unavailable("semantic vector search is disabled"))?;
        let model_space = embeddings.model_space_identity()?;
        let description_generation = ProvenanceGeneration::new(1)?;
        SemanticProvenanceSelector::new(model_space, description_generation)
    }

    /// Rejects forbidden decision paths and non-Jev providers before runtime wiring.
    pub fn validate_before_startup(&self) -> Result<(), A2aLabError> {
        if self.decisions.provider != JEV_EVALUATOR_PROVIDER_ID {
            return Err(A2aLabError::invalid(
                "semantic_indexing.decisions.provider",
                "only Jev is supported for frame decisions",
            ));
        }
        if self.decisions.model != DEFAULT_JEV_MODEL_REVISION {
            return Err(A2aLabError::invalid(
                "semantic_indexing.decisions.model",
                "unsupported Jev model revision",
            ));
        }
        parse_decision_evaluator_provider(&self.decisions.provider, &self.decisions.model)?;
        self.limits.worker.check_finite()?;
        self.limits.search.check_finite()?;
        self.limits.roi.check_finite()?;
        self.limits.storage.check_finite()?;
        self.limits.decisions.check_finite()?;
        Ok(())
    }
}

impl SemanticWorkerLimits {
    fn check_finite(&self) -> Result<(), A2aLabError> {
        check_positive(
            "semantic_indexing.limits.worker.max_batch_items",
            self.max_batch_items,
        )?;
        check_positive(
            "semantic_indexing.limits.worker.max_concurrency",
            self.max_concurrency,
        )?;
        check_positive(
            "semantic_indexing.limits.worker.max_queue_backlog",
            self.max_queue_backlog,
        )?;
        check_positive(
            "semantic_indexing.limits.worker.max_retry_attempts",
            self.max_retry_attempts,
        )?;
        Ok(())
    }
}

impl SemanticIndexingSearchLimits {
    fn check_finite(&self) -> Result<(), A2aLabError> {
        check_positive_bounded(
            "semantic_indexing.limits.search.top_k",
            self.top_k,
            MAX_SEMANTIC_TOP_K,
        )?;
        check_positive_bounded(
            "semantic_indexing.limits.search.candidate_limit",
            self.candidate_limit,
            MAX_SEMANTIC_CANDIDATES,
        )?;
        check_positive_bounded(
            "semantic_indexing.limits.search.max_neighbor_frames",
            self.max_neighbor_frames,
            MAX_SEMANTIC_NEIGHBOR_FRAMES,
        )?;
        Ok(())
    }
}

impl SemanticRoiLimits {
    fn check_finite(&self) -> Result<(), A2aLabError> {
        check_positive(
            "semantic_indexing.limits.roi.min_edge_pixels",
            self.min_edge_pixels,
        )?;
        check_positive(
            "semantic_indexing.limits.roi.max_edge_pixels",
            self.max_edge_pixels,
        )?;
        if self.max_edge_pixels < self.min_edge_pixels {
            return Err(A2aLabError::invalid(
                "semantic_indexing.limits.roi.max_edge_pixels",
                "must be greater than or equal to min_edge_pixels",
            ));
        }
        check_positive(
            "semantic_indexing.limits.roi.max_rois_per_frame",
            self.max_rois_per_frame,
        )?;
        Ok(())
    }
}

impl SemanticStorageLimits {
    fn check_finite(&self) -> Result<(), A2aLabError> {
        if self.vector_index_path.chars().all(char::is_whitespace) {
            return Err(A2aLabError::invalid(
                "semantic_indexing.limits.storage.vector_index_path",
                "must contain non-whitespace text",
            ));
        }
        if self.max_disk_bytes == 0 {
            return Err(A2aLabError::invalid(
                "semantic_indexing.limits.storage.max_disk_bytes",
                "must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl SemanticDecisionLimits {
    fn check_finite(&self) -> Result<(), A2aLabError> {
        check_positive(
            "semantic_indexing.limits.decisions.max_batch_items",
            self.max_batch_items,
        )?;
        check_positive(
            "semantic_indexing.limits.decisions.max_concurrency",
            self.max_concurrency,
        )?;
        Ok(())
    }
}

fn parse_description(value: Option<&Value>) -> Result<SemanticDescriptionConfig, A2aLabError> {
    let Some(value) = value else {
        return Ok(default_description());
    };
    let map = object_at("semantic_indexing.description", value)?;
    reject_unknown_keys(
        map,
        "semantic_indexing.description",
        &["provider", "model", "credentials"],
    )?;
    let backend = parse_description_provider(map.get("provider"))?;
    let model = parse_description_model(backend, map.get("model"))?;
    let credentials = parse_credentials(
        "semantic_indexing.description.credentials",
        map.get("credentials"),
    )?;
    Ok(SemanticDescriptionConfig {
        backend,
        model,
        credentials,
    })
}

fn default_description() -> SemanticDescriptionConfig {
    let backend = DEFAULT_SEMANTIC_DESCRIPTION_BACKEND;
    SemanticDescriptionConfig {
        backend,
        model: backend.default_model().to_owned(),
        credentials: None,
    }
}

fn parse_description_provider(
    value: Option<&Value>,
) -> Result<FrameDescriptionBackend, A2aLabError> {
    let Some(value) = value else {
        return Ok(DEFAULT_SEMANTIC_DESCRIPTION_BACKEND);
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.description.provider",
            "must be a string",
        ));
    };
    match text.as_str() {
        "bedrock" => Ok(FrameDescriptionBackend::Bedrock),
        "openai" => Ok(FrameDescriptionBackend::Openai),
        _ => Err(A2aLabError::invalid(
            "semantic_indexing.description.provider",
            "unsupported description provider",
        )),
    }
}

fn parse_description_model(
    backend: FrameDescriptionBackend,
    value: Option<&Value>,
) -> Result<String, A2aLabError> {
    let Some(value) = value else {
        return Ok(backend.default_model().to_owned());
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.description.model",
            "must be a string",
        ));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(text.clone())
    } else {
        Err(A2aLabError::invalid(
            "semantic_indexing.description.model",
            "model must contain non-whitespace text",
        ))
    }
}

fn parse_embeddings(value: Option<&Value>) -> Result<SemanticEmbeddingsConfig, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings",
            "required field is missing",
        ));
    };
    let map = object_at("semantic_indexing.embeddings", value)?;
    reject_unknown_keys(
        map,
        "semantic_indexing.embeddings",
        &["provider", "model", "space", "credentials"],
    )?;
    let provider =
        required_nonempty_string(map.get("provider"), "semantic_indexing.embeddings.provider")?;
    let model = required_nonempty_string(map.get("model"), "semantic_indexing.embeddings.model")?;
    let space = parse_embedding_space(map.get("space"))?;
    let credentials = parse_credentials(
        "semantic_indexing.embeddings.credentials",
        map.get("credentials"),
    )?;
    let config = SemanticEmbeddingsConfig {
        provider,
        model,
        space,
        credentials,
    };
    config.model_space_identity()?;
    Ok(config)
}

fn parse_embedding_space(value: Option<&Value>) -> Result<EmbeddingSpaceConfig, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space",
            "required field is missing",
        ));
    };
    let map = object_at("semantic_indexing.embeddings.space", value)?;
    reject_unknown_keys(
        map,
        "semantic_indexing.embeddings.space",
        &["dimensions", "normalization", "metric", "generation"],
    )?;
    let dimensions = match map.get("dimensions") {
        Some(value) => parse_u32("semantic_indexing.embeddings.space.dimensions", value)?,
        None => {
            return Err(A2aLabError::invalid(
                "semantic_indexing.embeddings.space.dimensions",
                "required field is missing",
            ));
        }
    };
    let normalization = parse_normalization(map.get("normalization"))?;
    let metric = parse_metric(map.get("metric"))?;
    let generation = match map.get("generation") {
        Some(value) => ProvenanceGeneration::new(u64::from(parse_u32(
            "semantic_indexing.embeddings.space.generation",
            value,
        )?))?,
        None => ProvenanceGeneration::new(1)?,
    };
    Ok(EmbeddingSpaceConfig {
        dimensions,
        normalization,
        metric,
        generation,
    })
}

fn parse_normalization(value: Option<&Value>) -> Result<VectorNormalization, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.normalization",
            "required field is missing",
        ));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.normalization",
            "must be a string",
        ));
    };
    match text.as_str() {
        "none" => Ok(VectorNormalization::None),
        "l2_unit" => Ok(VectorNormalization::L2Unit),
        _ => Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.normalization",
            "unsupported normalization",
        )),
    }
}

fn parse_metric(value: Option<&Value>) -> Result<EmbeddingMetric, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.metric",
            "required field is missing",
        ));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.metric",
            "must be a string",
        ));
    };
    match text.as_str() {
        "cosine" => Ok(EmbeddingMetric::Cosine),
        "dot" => Ok(EmbeddingMetric::Dot),
        "euclidean" => Ok(EmbeddingMetric::Euclidean),
        _ => Err(A2aLabError::invalid(
            "semantic_indexing.embeddings.space.metric",
            "unsupported similarity metric",
        )),
    }
}

fn parse_decisions(value: Option<&Value>) -> Result<SemanticDecisionsConfig, A2aLabError> {
    let Some(value) = value else {
        return Ok(default_decisions());
    };
    let map = object_at("semantic_indexing.decisions", value)?;
    reject_forbidden_decision_keys(map)?;
    reject_unknown_keys(
        map,
        "semantic_indexing.decisions",
        &["provider", "model", "credentials"],
    )?;
    let provider = match map.get("provider") {
        Some(value) => {
            required_nonempty_string(Some(value), "semantic_indexing.decisions.provider")?
        }
        None => JEV_EVALUATOR_PROVIDER_ID.to_owned(),
    };
    let model = match map.get("model") {
        Some(value) => required_nonempty_string(Some(value), "semantic_indexing.decisions.model")?,
        None => DEFAULT_JEV_MODEL_REVISION.to_owned(),
    };
    let credentials = parse_credentials(
        "semantic_indexing.decisions.credentials",
        map.get("credentials"),
    )?;
    Ok(SemanticDecisionsConfig {
        provider,
        model,
        credentials,
    })
}

fn default_decisions() -> SemanticDecisionsConfig {
    SemanticDecisionsConfig {
        provider: JEV_EVALUATOR_PROVIDER_ID.to_owned(),
        model: DEFAULT_JEV_MODEL_REVISION.to_owned(),
        credentials: None,
    }
}

fn reject_forbidden_semantic_keys(map: &serde_json::Map<String, Value>) -> Result<(), A2aLabError> {
    const FORBIDDEN: &[&str] = &[
        "fallback",
        "judge",
        "routing",
        "arbitration",
        "use_description_for_decisions",
        "chat",
    ];
    for key in map.keys() {
        if FORBIDDEN.contains(&key.as_str()) {
            return Err(A2aLabError::invalid(
                forbidden_semantic_field(key),
                "unsupported semantic indexing setting",
            ));
        }
    }
    Ok(())
}

fn forbidden_semantic_field(key: &str) -> &'static str {
    match key {
        "fallback" => "semantic_indexing.fallback",
        "judge" => "semantic_indexing.judge",
        "routing" => "semantic_indexing.routing",
        "arbitration" => "semantic_indexing.arbitration",
        "use_description_for_decisions" => "semantic_indexing.use_description_for_decisions",
        "chat" => "semantic_indexing.chat",
        _ => "semantic_indexing",
    }
}

fn reject_forbidden_decision_keys(map: &serde_json::Map<String, Value>) -> Result<(), A2aLabError> {
    const FORBIDDEN: &[&str] = &[
        "fallback",
        "fallback_provider",
        "fallback_model",
        "judge",
        "judge_model",
        "routing",
        "router",
        "arbitration",
        "use_description",
        "description_input",
        "embedding_similarity",
        "llm_interpretation",
        "chat_model",
    ];
    for key in map.keys() {
        if FORBIDDEN.contains(&key.as_str()) {
            return Err(A2aLabError::invalid(
                forbidden_decision_field(key),
                "unsupported decision setting",
            ));
        }
    }
    Ok(())
}

fn forbidden_decision_field(key: &str) -> &'static str {
    match key {
        "fallback" => "semantic_indexing.decisions.fallback",
        "fallback_provider" => "semantic_indexing.decisions.fallback_provider",
        "fallback_model" => "semantic_indexing.decisions.fallback_model",
        "judge" => "semantic_indexing.decisions.judge",
        "judge_model" => "semantic_indexing.decisions.judge_model",
        "routing" => "semantic_indexing.decisions.routing",
        "router" => "semantic_indexing.decisions.router",
        "arbitration" => "semantic_indexing.decisions.arbitration",
        "use_description" => "semantic_indexing.decisions.use_description",
        "description_input" => "semantic_indexing.decisions.description_input",
        "embedding_similarity" => "semantic_indexing.decisions.embedding_similarity",
        "llm_interpretation" => "semantic_indexing.decisions.llm_interpretation",
        "chat_model" => "semantic_indexing.decisions.chat_model",
        _ => "semantic_indexing.decisions",
    }
}

fn parse_limits(value: Option<&Value>) -> Result<SemanticIndexingLimits, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.limits",
            "required field is missing",
        ));
    };
    let map = object_at("semantic_indexing.limits", value)?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits",
        &["worker", "search", "roi", "storage", "decisions"],
    )?;
    Ok(SemanticIndexingLimits {
        worker: parse_worker_limits(map.get("worker"))?,
        search: parse_search_limits(map.get("search"))?,
        roi: map.get("roi").map_or_else(
            || Ok(SemanticRoiLimits::default()),
            |value| parse_roi_limits(Some(value)),
        )?,
        storage: parse_storage_limits(map.get("storage"))?,
        decisions: parse_decision_limits(map.get("decisions"))?,
    })
}

fn parse_worker_limits(value: Option<&Value>) -> Result<SemanticWorkerLimits, A2aLabError> {
    let map = limits_object(value, "semantic_indexing.limits.worker")?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits.worker",
        &[
            "max_batch_items",
            "max_concurrency",
            "max_queue_backlog",
            "max_retry_attempts",
            "job_deadline",
            "retry_backoff",
        ],
    )?;
    Ok(SemanticWorkerLimits {
        max_batch_items: required_positive_u32(
            map.get("max_batch_items"),
            "semantic_indexing.limits.worker.max_batch_items",
        )?,
        max_concurrency: required_positive_u32(
            map.get("max_concurrency"),
            "semantic_indexing.limits.worker.max_concurrency",
        )?,
        max_queue_backlog: required_positive_u32(
            map.get("max_queue_backlog"),
            "semantic_indexing.limits.worker.max_queue_backlog",
        )?,
        max_retry_attempts: required_positive_u32(
            map.get("max_retry_attempts"),
            "semantic_indexing.limits.worker.max_retry_attempts",
        )?,
        job_deadline: required_duration(
            map.get("job_deadline"),
            "semantic_indexing.limits.worker.job_deadline",
        )?,
        retry_backoff: required_duration(
            map.get("retry_backoff"),
            "semantic_indexing.limits.worker.retry_backoff",
        )?,
    })
}

fn parse_search_limits(value: Option<&Value>) -> Result<SemanticIndexingSearchLimits, A2aLabError> {
    let map = limits_object(value, "semantic_indexing.limits.search")?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits.search",
        &[
            "top_k",
            "candidate_limit",
            "max_neighbor_frames",
            "query_window",
        ],
    )?;
    Ok(SemanticIndexingSearchLimits {
        top_k: required_positive_u32(map.get("top_k"), "semantic_indexing.limits.search.top_k")?,
        candidate_limit: required_positive_u32(
            map.get("candidate_limit"),
            "semantic_indexing.limits.search.candidate_limit",
        )?,
        max_neighbor_frames: required_positive_u32(
            map.get("max_neighbor_frames"),
            "semantic_indexing.limits.search.max_neighbor_frames",
        )?,
        query_window: required_duration(
            map.get("query_window"),
            "semantic_indexing.limits.search.query_window",
        )?,
    })
}

fn parse_roi_limits(value: Option<&Value>) -> Result<SemanticRoiLimits, A2aLabError> {
    let map = limits_object(value, "semantic_indexing.limits.roi")?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits.roi",
        &["min_edge_pixels", "max_edge_pixels", "max_rois_per_frame"],
    )?;
    Ok(SemanticRoiLimits {
        min_edge_pixels: required_positive_u32(
            map.get("min_edge_pixels"),
            "semantic_indexing.limits.roi.min_edge_pixels",
        )?,
        max_edge_pixels: required_positive_u32(
            map.get("max_edge_pixels"),
            "semantic_indexing.limits.roi.max_edge_pixels",
        )?,
        max_rois_per_frame: required_positive_u32(
            map.get("max_rois_per_frame"),
            "semantic_indexing.limits.roi.max_rois_per_frame",
        )?,
    })
}

fn parse_storage_limits(value: Option<&Value>) -> Result<SemanticStorageLimits, A2aLabError> {
    let map = limits_object(value, "semantic_indexing.limits.storage")?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits.storage",
        &["vector_index_path", "max_disk_bytes"],
    )?;
    let vector_index_path = required_nonempty_string(
        map.get("vector_index_path"),
        "semantic_indexing.limits.storage.vector_index_path",
    )?;
    let max_disk_bytes = match map.get("max_disk_bytes") {
        Some(value) => positive_u64("semantic_indexing.limits.storage.max_disk_bytes", value)?,
        None => {
            return Err(A2aLabError::invalid(
                "semantic_indexing.limits.storage.max_disk_bytes",
                "required field is missing",
            ));
        }
    };
    Ok(SemanticStorageLimits {
        vector_index_path,
        max_disk_bytes,
    })
}

fn parse_decision_limits(value: Option<&Value>) -> Result<SemanticDecisionLimits, A2aLabError> {
    let map = limits_object(value, "semantic_indexing.limits.decisions")?;
    reject_unknown_keys(
        map,
        "semantic_indexing.limits.decisions",
        &[
            "max_batch_items",
            "max_concurrency",
            "job_deadline",
            "coverage_policy",
        ],
    )?;
    Ok(SemanticDecisionLimits {
        max_batch_items: required_positive_u32(
            map.get("max_batch_items"),
            "semantic_indexing.limits.decisions.max_batch_items",
        )?,
        max_concurrency: required_positive_u32(
            map.get("max_concurrency"),
            "semantic_indexing.limits.decisions.max_concurrency",
        )?,
        job_deadline: required_duration(
            map.get("job_deadline"),
            "semantic_indexing.limits.decisions.job_deadline",
        )?,
        coverage_policy: parse_coverage_policy(map.get("coverage_policy"))?,
    })
}

fn parse_coverage_policy(value: Option<&Value>) -> Result<DecisionCoveragePolicy, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.limits.decisions.coverage_policy",
            "required field is missing",
        ));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "semantic_indexing.limits.decisions.coverage_policy",
            "must be a string",
        ));
    };
    match text.as_str() {
        "all_saved_frames" => Ok(DecisionCoveragePolicy::AllSavedFrames),
        "on_demand" => Ok(DecisionCoveragePolicy::OnDemand),
        "sampled" => Ok(DecisionCoveragePolicy::Sampled),
        _ => Err(A2aLabError::invalid(
            "semantic_indexing.limits.decisions.coverage_policy",
            "unsupported coverage policy",
        )),
    }
}

fn parse_credentials(
    path: &'static str,
    value: Option<&Value>,
) -> Result<Option<CredentialReference>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = object_at(path, value)?;
    reject_unknown_keys(map, path, &["env"])?;
    let env_field = match path {
        "semantic_indexing.description.credentials" => {
            "semantic_indexing.description.credentials.env"
        }
        "semantic_indexing.embeddings.credentials" => {
            "semantic_indexing.embeddings.credentials.env"
        }
        "semantic_indexing.decisions.credentials" => "semantic_indexing.decisions.credentials.env",
        _ => "semantic_indexing.credentials.env",
    };
    let env_var = required_nonempty_string(map.get("env"), env_field)?;
    validate_env_reference(&env_var, env_field)?;
    Ok(Some(CredentialReference { env_var }))
}

fn validate_env_reference(name: &str, field: &'static str) -> Result<(), A2aLabError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(invalid_env_reference(field));
    };
    if !first.is_ascii_uppercase() {
        return Err(invalid_env_reference(field));
    }
    if chars.all(|character| {
        character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
    }) {
        Ok(())
    } else {
        Err(invalid_env_reference(field))
    }
}

fn invalid_env_reference(field: &'static str) -> A2aLabError {
    A2aLabError::invalid(
        field,
        "credential env reference must be an uppercase identifier",
    )
}

fn limits_object<'a>(
    value: Option<&'a Value>,
    path: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(path, "required field is missing"));
    };
    object_at(path, value)
}

fn required_duration(
    field: Option<&Value>,
    path: &'static str,
) -> Result<ConfigDuration, A2aLabError> {
    let Some(value) = field else {
        return Err(A2aLabError::invalid(path, "required field is missing"));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            path,
            "must be a positive whole-second duration",
        ));
    };
    ConfigDuration::parse(path, text)
}

fn required_positive_u32(field: Option<&Value>, path: &'static str) -> Result<u32, A2aLabError> {
    let Some(value) = field else {
        return Err(A2aLabError::invalid(path, "required field is missing"));
    };
    let parsed = parse_u32(path, value)?;
    check_positive(path, parsed)?;
    Ok(parsed)
}

fn required_nonempty_string(
    field: Option<&Value>,
    path: &'static str,
) -> Result<String, A2aLabError> {
    let Some(value) = field else {
        return Err(A2aLabError::invalid(path, "required field is missing"));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(path, "must be a string"));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(text.clone())
    } else {
        Err(A2aLabError::invalid(
            path,
            "must contain non-whitespace text",
        ))
    }
}

fn positive_u64(field: &'static str, value: &Value) -> Result<u64, A2aLabError> {
    let parsed = match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| A2aLabError::invalid(field, "must be a positive integer")),
        _ => Err(A2aLabError::invalid(field, "must be a positive integer")),
    }?;
    if parsed == 0 {
        return Err(A2aLabError::invalid(field, "must be greater than zero"));
    }
    Ok(parsed)
}

fn check_positive(field: &'static str, value: u32) -> Result<(), A2aLabError> {
    if value == 0 {
        return Err(A2aLabError::invalid(field, "must be greater than zero"));
    }
    Ok(())
}

fn check_positive_bounded(field: &'static str, value: u32, max: u32) -> Result<(), A2aLabError> {
    if value == 0 || value > max {
        return Err(A2aLabError::invalid(field, format!("must be 1..={max}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        A2aLabConfig, DEFAULT_BEDROCK_DESCRIPTION_MODEL, DEFAULT_OPENAI_DESCRIPTION_MODEL,
    };

    fn minimal_v2_suffix() -> &'static str {
        r#","semantic_indexing":{
            "embeddings":{"provider":"p","model":"m","space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine","generation":1}},
            "limits":{
                "worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},
                "search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},
                "roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},
                "storage":{"vector_index_path":"./semantic","max_disk_bytes":1024},
                "decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}
            }
        }}"#
    }

    #[test]
    fn v2_defaults_description_and_decisions() {
        let document = format!(r#"{{"schema_version":2{}}}"#, minimal_v2_suffix());
        let config = A2aLabConfig::parse(&document).unwrap();
        let semantic = config.semantic_indexing.as_ref().unwrap();
        assert_eq!(
            semantic.description.model(),
            DEFAULT_BEDROCK_DESCRIPTION_MODEL
        );
        assert_eq!(semantic.decisions.model(), DEFAULT_JEV_MODEL_REVISION);
    }

    #[test]
    fn description_only_mode_does_not_require_embedding_configuration() {
        let suffix = minimal_v2_suffix().replace(
            r#""embeddings":{"provider":"p","model":"m","space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine","generation":1}},"#,
            r#""embeddings":null,"#,
        );
        let config = A2aLabConfig::parse(&format!(r#"{{"schema_version":2{suffix}"#)).unwrap();
        let semantic = config.semantic_indexing.unwrap();
        assert!(semantic.embeddings.is_none());
    }

    #[test]
    fn v2_description_override_is_independent_of_chat() {
        let document = format!(
            r#"{{"schema_version":2,"chat":{{"provider":"openai","model":"gpt-chat"}},"semantic_indexing":{{
                "description":{{"provider":"openai","model":"{DEFAULT_OPENAI_DESCRIPTION_MODEL}"}},
                "embeddings":{{"provider":"emb","model":"text-embed","space":{{"dimensions":16,"normalization":"none","metric":"dot","generation":2}}}},
                "limits":{{
                    "worker":{{"max_batch_items":2,"max_concurrency":2,"max_queue_backlog":10,"max_retry_attempts":3,"job_deadline":"30s","retry_backoff":"1s"}},
                    "search":{{"top_k":5,"candidate_limit":100,"max_neighbor_frames":3,"query_window":"30m"}},
                    "roi":{{"min_edge_pixels":280,"max_edge_pixels":1024,"max_rois_per_frame":4}},
                    "storage":{{"vector_index_path":"./idx","max_disk_bytes":4096}},
                    "decisions":{{"max_batch_items":2,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"all_saved_frames"}}
                }}
            }}}}"#
        );
        let config = A2aLabConfig::parse(&document).unwrap();
        assert_eq!(
            config.chat.as_ref().unwrap().model.as_deref(),
            Some("gpt-chat")
        );
        assert_eq!(
            config
                .semantic_indexing
                .as_ref()
                .unwrap()
                .description
                .model(),
            DEFAULT_OPENAI_DESCRIPTION_MODEL
        );
    }

    #[test]
    fn source_schedule_parses_independent_finite_stage_intervals() {
        let document = format!(
            r#"{{"schema_version":2,"videos":{{"sources":[{{"id":"camera","format":"mjpeg","device_index":12,"semantic_indexing":{{"description":"1.5s","description_model":"global.openai.gpt-5.6-luna","embeddings":"1m","embedding_model":"amazon.titan-embed-text-v2:0"}}}}]}}{}"#,
            minimal_v2_suffix()
        );
        let config = A2aLabConfig::parse(&document).unwrap();
        let videos = config.videos.unwrap();
        let schedule = videos.sources[0].semantic_indexing.as_ref().unwrap();
        assert_eq!(schedule.stream(), "camera");
        assert_eq!(
            schedule.description_model(),
            Some("global.openai.gpt-5.6-luna")
        );
        assert_eq!(
            schedule.embedding_model(),
            Some("amazon.titan-embed-text-v2:0")
        );
        assert_eq!(
            schedule.description(),
            SemanticSchedule::Interval { micros: 1_500_000 }
        );
        assert_eq!(
            schedule.embeddings(),
            SemanticSchedule::Interval { micros: 60_000_000 }
        );
    }

    #[test]
    fn source_schedule_rejects_zero_and_unknown_stage() {
        let zero = format!(
            r#"{{"schema_version":2,"videos":{{"sources":[{{"id":"camera","format":"mjpeg","device_index":12,"semantic_indexing":{{"description":"0s"}}}}]}}{}"#,
            minimal_v2_suffix()
        );
        assert!(A2aLabConfig::parse(&zero).is_err());
        let unknown = format!(
            r#"{{"schema_version":2,"videos":{{"sources":[{{"id":"camera","format":"mjpeg","device_index":12,"semantic_indexing":{{"judge":"5s"}}}}]}}{}"#,
            minimal_v2_suffix()
        );
        assert!(A2aLabConfig::parse(&unknown).is_err());
    }

    #[test]
    fn v1_rejects_source_semantic_indexing() {
        let document = r#"{"schema_version":1,"videos":{"sources":[{"id":"camera","format":"mjpeg","device_index":12,"semantic_indexing":{"description":"5s"}}]}}"#;
        assert!(A2aLabConfig::parse(document).is_err());
    }

    #[test]
    fn credential_env_reference_allows_digits_after_first_character() {
        assert!(
            validate_env_reference("A2ALAB_JEV_API_KEY", "semantic_indexing.credentials.env")
                .is_ok()
        );
        assert!(validate_env_reference("AWS_REGION2", "semantic_indexing.credentials.env").is_ok());
    }

    #[test]
    fn credential_env_reference_rejects_invalid_identifiers() {
        let field = "semantic_indexing.credentials.env";
        assert!(validate_env_reference("2FAST", field).is_err());
        assert!(validate_env_reference("lowercase", field).is_err());
        assert!(validate_env_reference("HAS-DASH", field).is_err());
        assert!(validate_env_reference("HAS.dot", field).is_err());
    }

    #[test]
    fn rejects_non_jev_decision_provider_before_startup() {
        let document = r#"{"schema_version":2,"semantic_indexing":{
                "decisions":{"provider":"openai","model":"gpt-decide"},
                "embeddings":{"provider":"p","model":"m","space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine","generation":1}},
                "limits":{
                    "worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},
                    "search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},
                    "roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},
                    "storage":{"vector_index_path":"./semantic","max_disk_bytes":1024},
                    "decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}
                }
            }}"#;
        let error = A2aLabConfig::parse(document).unwrap_err();
        assert!(error.to_string().contains("semantic_indexing.decisions"));
        assert!(!error.to_string().contains("gpt-decide"));
    }
}
