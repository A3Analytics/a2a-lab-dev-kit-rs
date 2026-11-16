//! Semantic search hits, retrieval scores, and indexing coverage.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::page::Page;
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::index_provenance::{FrameDescriptionProvenance, FrameVectorIndexAttachment};
use crate::videos::model_space::{EmbeddingMetric, ModelSpaceIdentity};
use crate::videos::saved_frame::SavedFrameRef;
use crate::videos::semantic_search::assessments::SemanticFrameHitAssessments;
use crate::videos::semantic_search::query::SemanticSearchScope;

/// Persisted indexing state for one saved revision inside the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticFrameIndexState {
    /// Manifest recorded; inference not finished.
    Pending,
    /// Description and vector acknowledged for retrieval.
    Ready,
    /// Retryable or terminal inference failure.
    Failed,
    /// Retention removed the revision from search.
    Expired,
}

/// Whether saved media bytes remain playable for a hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticFrameMediaAvailability {
    /// Authorized media is resolvable.
    Ready,
    /// Indexing or media reconciliation still in progress.
    Pending,
    /// Evidence cannot be served for this revision.
    Unavailable {
        /// Stable reason such as `retention_expired` or `payload_missing`.
        reason: String,
    },
}

/// ANN similarity returned to callers. Not a Jev decision probability.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticRetrievalScore {
    value: f32,
    metric: EmbeddingMetric,
}

impl SemanticRetrievalScore {
    /// Creates a finite retrieval score in the declared metric.
    pub fn new(value: f32, metric: EmbeddingMetric) -> Result<Self, A2aLabError> {
        if !value.is_finite() {
            return Err(A2aLabError::invalid(
                "retrieval_score",
                "must be a finite number",
            ));
        }
        Ok(Self { value, metric })
    }

    /// Similarity or distance in the active embedding metric.
    #[must_use]
    pub const fn value(self) -> f32 {
        self.value
    }

    /// Metric that produced `value`.
    #[must_use]
    pub const fn metric(self) -> EmbeddingMetric {
        self.metric
    }
}

/// Bounded neighbor frames for temporal playback around one hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticFramePlaybackNeighbors {
    window: TimeRange,
    frames: Vec<SavedFrameRef>,
}

impl SemanticFramePlaybackNeighbors {
    /// Creates neighbors when the list is within the declared cap.
    pub fn new(
        window: TimeRange,
        frames: Vec<SavedFrameRef>,
        max_frames: u32,
    ) -> Result<Self, A2aLabError> {
        window.check()?;
        if frames.len() > usize::try_from(max_frames).unwrap_or(usize::MAX) {
            return Err(A2aLabError::invalid(
                "neighbors",
                format!("must contain at most {max_frames} frames"),
            ));
        }
        Ok(Self { window, frames })
    }

    /// Half-open UTC playback window spanning the neighbors.
    #[must_use]
    pub const fn window(&self) -> TimeRange {
        self.window
    }

    /// Neighbor revisions in capture order.
    #[must_use]
    pub fn frames(&self) -> &[SavedFrameRef] {
        &self.frames
    }
}

/// One ranked semantic retrieval hit.
///
/// Generated description text is Luna retrieval metadata only. Authoritative Jev
/// assessments, when present, are carried in `authorized_assessments` and are
/// never mixed into retrieval scores or generated text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticFrameHit {
    frame: SavedFrameRef,
    captured_at: UtcTimestamp,
    /// Luna-generated search metadata; not a lab predicate assessment.
    generated_description: String,
    description_provenance: FrameDescriptionProvenance,
    model_space: ModelSpaceIdentity,
    retrieval: SemanticRetrievalScore,
    index: SemanticFrameIndexState,
    media_availability: SemanticFrameMediaAvailability,
    neighbors: SemanticFramePlaybackNeighbors,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorized_assessments: Option<SemanticFrameHitAssessments>,
}

impl SemanticFrameHit {
    /// Validates a hit and its vector attachment coherence.
    pub fn new(
        frame: SavedFrameRef,
        generated_description: impl Into<String>,
        attachment: &FrameVectorIndexAttachment,
        retrieval: SemanticRetrievalScore,
        index: SemanticFrameIndexState,
        media_availability: SemanticFrameMediaAvailability,
        neighbors: SemanticFramePlaybackNeighbors,
        authorized_assessments: Option<SemanticFrameHitAssessments>,
    ) -> Result<Self, A2aLabError> {
        let generated_description = generated_description.into();
        if generated_description.trim().is_empty() {
            return Err(A2aLabError::invalid(
                "generated_description",
                "must be nonblank",
            ));
        }
        attachment.validate()?;
        if retrieval.metric() != attachment.model_space().metric() {
            return Err(A2aLabError::invalid(
                "retrieval.metric",
                "must match the attachment model-space metric",
            ));
        }
        let captured_at = frame.captured_at();
        if let Some(assessments) = &authorized_assessments
            && assessments.frame() != &frame
        {
            return Err(A2aLabError::invalid(
                "authorized_assessments.frame",
                "must match the hit frame revision",
            ));
        }
        Ok(Self {
            frame,
            captured_at,
            generated_description,
            description_provenance: attachment.description().clone(),
            model_space: attachment.model_space().clone(),
            retrieval,
            index,
            media_availability,
            neighbors,
            authorized_assessments,
        })
    }

    /// Exact saved revision returned to operators.
    #[must_use]
    pub const fn frame(&self) -> &SavedFrameRef {
        &self.frame
    }

    /// Original capture instant.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Luna description text used for retrieval display only.
    #[must_use]
    pub fn generated_description(&self) -> &str {
        &self.generated_description
    }

    /// Description provenance carried with the hit.
    #[must_use]
    pub const fn description_provenance(&self) -> &FrameDescriptionProvenance {
        &self.description_provenance
    }

    /// Active embedding index namespace.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }

    /// Vector similarity for ranking only.
    #[must_use]
    pub const fn retrieval(&self) -> SemanticRetrievalScore {
        self.retrieval
    }

    /// Indexing lifecycle state for the revision.
    #[must_use]
    pub const fn index(&self) -> SemanticFrameIndexState {
        self.index
    }

    /// Saved media availability for playback.
    #[must_use]
    pub const fn media_availability(&self) -> &SemanticFrameMediaAvailability {
        &self.media_availability
    }

    /// Bounded neighbor context for playback.
    #[must_use]
    pub const fn neighbors(&self) -> &SemanticFramePlaybackNeighbors {
        &self.neighbors
    }

    /// Optional bounded Jev assessments associated with this frame revision.
    #[must_use]
    pub const fn authorized_assessments(&self) -> Option<&SemanticFrameHitAssessments> {
        self.authorized_assessments.as_ref()
    }

    /// Returns a copy with attached assessments validated against this hit.
    pub fn with_authorized_assessments(
        self,
        authorized_assessments: Option<SemanticFrameHitAssessments>,
    ) -> Result<Self, A2aLabError> {
        if let Some(assessments) = &authorized_assessments
            && assessments.frame() != &self.frame
        {
            return Err(A2aLabError::invalid(
                "authorized_assessments.frame",
                "must match the hit frame revision",
            ));
        }
        Ok(Self {
            authorized_assessments,
            ..self
        })
    }
}

/// One page of semantic hits plus incomplete indexing signals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticFrameSearchResult {
    hits: Page<SemanticFrameHit>,
    incomplete_indexing: bool,
    candidate_budget_exhausted: bool,
}

impl SemanticFrameSearchResult {
    /// Packages ranked hits with explicit partial-result flags.
    #[must_use]
    pub fn new(
        hits: Page<SemanticFrameHit>,
        incomplete_indexing: bool,
        candidate_budget_exhausted: bool,
    ) -> Self {
        Self {
            hits,
            incomplete_indexing,
            candidate_budget_exhausted,
        }
    }

    /// Ranked page of hits.
    #[must_use]
    pub const fn hits(&self) -> &Page<SemanticFrameHit> {
        &self.hits
    }

    /// When true, an empty page is not evidence of absence.
    #[must_use]
    pub fn incomplete_indexing(&self) -> bool {
        self.incomplete_indexing
    }

    /// When true, decision filters or refill stopped before top-k was filled.
    #[must_use]
    pub fn candidate_budget_exhausted(&self) -> bool {
        self.candidate_budget_exhausted
    }
}

/// Per-state counts inside a scoped coverage report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SemanticIndexingStateCounts {
    pending: u64,
    ready: u64,
    failed: u64,
    expired: u64,
    #[serde(default)]
    skipped: u64,
}

impl SemanticIndexingStateCounts {
    /// Creates counts for one scope snapshot.
    #[must_use]
    pub const fn new(pending: u64, ready: u64, failed: u64, expired: u64) -> Self {
        Self {
            pending,
            ready,
            failed,
            expired,
            skipped: 0,
        }
    }

    /// Adds terminal skipped job counts.
    #[must_use]
    pub const fn with_skipped(self, skipped: u64) -> Self {
        Self { skipped, ..self }
    }

    /// Manifest rows awaiting inference.
    #[must_use]
    pub const fn pending(self) -> u64 {
        self.pending
    }

    /// Searchable revisions with acknowledged vectors.
    #[must_use]
    pub const fn ready(self) -> u64 {
        self.ready
    }

    /// Failed or retryable inference rows.
    #[must_use]
    pub const fn failed(self) -> u64 {
        self.failed
    }

    /// Retention-expired or tombstoned revisions.
    #[must_use]
    pub const fn expired(self) -> u64 {
        self.expired
    }

    /// Terminal skipped eligibility that will not re-enter the queue.
    #[must_use]
    pub const fn skipped(self) -> u64 {
        self.skipped
    }

    /// Total accounted revisions.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.pending + self.ready + self.failed + self.expired + self.skipped
    }
}

/// Capture-time boundaries describing indexing freshness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SemanticIndexingFreshness {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oldest_pending: Option<UtcTimestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    newest_ready: Option<UtcTimestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    newest_failed: Option<UtcTimestamp>,
}

impl SemanticIndexingFreshness {
    /// Reports capture-time boundaries inside the scope.
    #[must_use]
    pub const fn new(
        oldest_pending: Option<UtcTimestamp>,
        newest_ready: Option<UtcTimestamp>,
        newest_failed: Option<UtcTimestamp>,
    ) -> Self {
        Self {
            oldest_pending,
            newest_ready,
            newest_failed,
        }
    }

    /// Oldest capture among pending rows when any exist.
    #[must_use]
    pub const fn oldest_pending(self) -> Option<UtcTimestamp> {
        self.oldest_pending
    }

    /// Newest capture among ready rows when any exist.
    #[must_use]
    pub const fn newest_ready(self) -> Option<UtcTimestamp> {
        self.newest_ready
    }

    /// Newest capture among failed rows when any exist.
    #[must_use]
    pub const fn newest_failed(self) -> Option<UtcTimestamp> {
        self.newest_failed
    }
}

/// Scoped semantic indexing coverage for search readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticIndexingCoverage {
    scope: SemanticSearchScope,
    model_space: ModelSpaceIdentity,
    counts: SemanticIndexingStateCounts,
    freshness: SemanticIndexingFreshness,
    scope_complete: bool,
}

impl SemanticIndexingCoverage {
    /// Creates a coverage snapshot.
    #[must_use]
    pub fn new(
        scope: SemanticSearchScope,
        model_space: ModelSpaceIdentity,
        counts: SemanticIndexingStateCounts,
        freshness: SemanticIndexingFreshness,
        scope_complete: bool,
    ) -> Self {
        Self {
            scope,
            model_space,
            counts,
            freshness,
            scope_complete,
        }
    }

    /// Inspected source and UTC range.
    #[must_use]
    pub const fn scope(&self) -> &SemanticSearchScope {
        &self.scope
    }

    /// Model space for the coverage snapshot.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }

    /// Pending/ready/failed/expired counts.
    #[must_use]
    pub const fn counts(&self) -> SemanticIndexingStateCounts {
        self.counts
    }

    /// Capture-time freshness boundaries.
    #[must_use]
    pub const fn freshness(&self) -> &SemanticIndexingFreshness {
        &self.freshness
    }

    /// When false, incomplete indexing must be surfaced alongside empty search pages.
    #[must_use]
    pub fn scope_complete(&self) -> bool {
        self.scope_complete
    }

    /// Pending or failed rows remain inside the scope.
    #[must_use]
    pub fn indexing_incomplete(&self) -> bool {
        !self.scope_complete || self.counts.pending > 0
    }
}
