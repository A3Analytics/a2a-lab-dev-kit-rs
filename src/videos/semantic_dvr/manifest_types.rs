//! Semantic frame manifest types.

use crate::{
    A2aLabError, DvrTrackId, FrameDescriptionProvenance, FrameEmbeddingVector, ModelSpaceIdentity,
    SavedFrameRef, TextEmbeddingProvenance, TimeRange, UtcTimestamp, VideoSourceId,
};
const MAX_MANIFEST_PAGE_LIMIT: u32 = 1_000;

/// Retained lifecycle for a manifest revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestRetainedState {
    Retained,
    Expired,
    Superseded,
}

/// Independent checkpoint for description or embedding work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingCheckpoint {
    Pending,
    Ready,
    Failed,
}

/// Result of reading saved media bytes for a manifest revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPayloadStatus {
    Available,
    Missing,
    Corrupt,
}

/// Outcome of registering one saved DVR frame in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterManifestOutcome {
    Registered,
    Idempotent,
    ReplacedPrior,
}

/// Keyset page request for manifest enumeration (`{start_us}|{frame_id}` cursor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPageRequest {
    cursor: Option<String>,
    limit: u32,
}

impl ManifestPageRequest {
    /// Validates bounds and builds a manifest page descriptor.
    pub fn new(cursor: Option<String>, limit: u32) -> Result<Self, A2aLabError> {
        validate_manifest_page(limit, cursor.as_deref())?;
        Ok(Self { cursor, limit })
    }

    /// Rejects empty cursors or out-of-range limits.
    pub fn check(&self) -> Result<(), A2aLabError> {
        validate_manifest_page(self.limit, self.cursor.as_deref())
    }

    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }
}

/// Scoped manifest listing query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListManifestFramesRequest {
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    window: Option<TimeRange>,
    current_search_only: bool,
    page: ManifestPageRequest,
}

/// Ready searchable revisions compatible with one provenance selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSearchCandidateFilter {
    /// Required Luna description generation.
    pub description_generation: u64,
    /// Active embedding model space.
    pub model_space: ModelSpaceIdentity,
}

/// One bounded scan of manifest rows considered before vector ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestSearchCandidateScan {
    /// Loaded revisions in deterministic capture order.
    pub records: Vec<SemanticFrameManifestRecord>,
    /// When true, additional ready rows exist beyond the scan limit.
    pub truncated: bool,
}

impl ListManifestFramesRequest {
    /// Builds a listing request.
    pub fn new(
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        window: Option<TimeRange>,
        current_search_only: bool,
        page: ManifestPageRequest,
    ) -> Result<Self, A2aLabError> {
        page.check()?;
        Ok(Self {
            source_id,
            track_id,
            window,
            current_search_only,
            page,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    #[must_use]
    pub const fn window(&self) -> Option<&TimeRange> {
        self.window.as_ref()
    }

    #[must_use]
    pub const fn current_search_only(&self) -> bool {
        self.current_search_only
    }

    #[must_use]
    pub const fn page(&self) -> &ManifestPageRequest {
        &self.page
    }
}

/// One durable manifest row with resolved media status.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFrameManifestRecord {
    pub(crate) frame: SavedFrameRef,
    pub(crate) end_us: UtcTimestamp,
    pub(crate) retained_state: ManifestRetainedState,
    pub(crate) search_current: bool,
    pub(crate) revision_seq: u32,
    pub(crate) description_checkpoint: IndexingCheckpoint,
    pub(crate) embedding_checkpoint: IndexingCheckpoint,
    pub(crate) description: Option<StoredFrameDescription>,
    pub(crate) embedding: Option<StoredFrameEmbedding>,
    pub(crate) payload_status: MediaPayloadStatus,
}

impl SemanticFrameManifestRecord {
    #[must_use]
    pub const fn frame(&self) -> &SavedFrameRef {
        &self.frame
    }

    #[must_use]
    pub const fn end_us(&self) -> UtcTimestamp {
        self.end_us
    }

    #[must_use]
    pub const fn retained_state(&self) -> ManifestRetainedState {
        self.retained_state
    }

    #[must_use]
    pub const fn search_current(&self) -> bool {
        self.search_current
    }

    #[must_use]
    pub const fn revision_seq(&self) -> u32 {
        self.revision_seq
    }

    #[must_use]
    pub const fn description_checkpoint(&self) -> IndexingCheckpoint {
        self.description_checkpoint
    }

    #[must_use]
    pub const fn embedding_checkpoint(&self) -> IndexingCheckpoint {
        self.embedding_checkpoint
    }

    #[must_use]
    pub const fn description(&self) -> Option<&StoredFrameDescription> {
        self.description.as_ref()
    }

    #[must_use]
    pub const fn embedding(&self) -> Option<&StoredFrameEmbedding> {
        self.embedding.as_ref()
    }

    #[must_use]
    pub const fn payload_status(&self) -> MediaPayloadStatus {
        self.payload_status
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredFrameDescription {
    pub(crate) text: String,
    pub(crate) provenance: FrameDescriptionProvenance,
}

impl StoredFrameDescription {
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn provenance(&self) -> &FrameDescriptionProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredFrameEmbedding {
    pub(crate) provenance: TextEmbeddingProvenance,
    pub(crate) vector: Option<FrameEmbeddingVector>,
}

impl StoredFrameEmbedding {
    #[must_use]
    pub const fn provenance(&self) -> &TextEmbeddingProvenance {
        &self.provenance
    }

    #[must_use]
    pub const fn vector(&self) -> Option<&FrameEmbeddingVector> {
        self.vector.as_ref()
    }
}

fn validate_manifest_page(limit: u32, cursor: Option<&str>) -> Result<(), A2aLabError> {
    if limit == 0 || limit > MAX_MANIFEST_PAGE_LIMIT {
        return Err(A2aLabError::invalid(
            "limit",
            format!("must be 1..={MAX_MANIFEST_PAGE_LIMIT}"),
        ));
    }
    if let Some(cursor) = cursor {
        parse_manifest_cursor(cursor)?;
    }
    Ok(())
}

pub(crate) fn parse_manifest_cursor(cursor: &str) -> Result<(i64, String), A2aLabError> {
    let (start, frame) = cursor
        .split_once('|')
        .ok_or_else(|| A2aLabError::invalid("cursor", "must be start_us|frame_id"))?;
    let start_us = start
        .parse::<i64>()
        .map_err(|_| A2aLabError::invalid("cursor", "start_us is not a valid integer"))?;
    if frame.is_empty() {
        return Err(A2aLabError::invalid("cursor", "frame_id must not be empty"));
    }
    Ok((start_us, frame.to_owned()))
}
