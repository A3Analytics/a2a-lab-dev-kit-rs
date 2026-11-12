//! Optional semantic retrieval provider boundary.

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::videos::semantic_search::provenance::SemanticProvenanceSelector;
use crate::videos::semantic_search::request::{
    GetSemanticIndexingCoverageRequest, SearchSemanticFramesRequest,
};
use crate::videos::semantic_search::result::{SemanticFrameSearchResult, SemanticIndexingCoverage};

/// Text-to-frame and frame-to-frame retrieval plus indexing coverage.
///
/// Implementations are optional. [`crate::videos::VideoProvider`] remains the
/// control plane for clips, timelines, and live follow without semantic search.
#[async_trait]
pub trait SemanticVideoSearchProvider: Send + Sync {
    /// Returns ranked semantic hits inside the request scope.
    async fn search_semantic_frames(
        &self,
        request: SearchSemanticFramesRequest,
    ) -> Result<SemanticFrameSearchResult, A2aLabError>;

    /// Returns per-state counts and freshness for the scoped generation.
    async fn get_semantic_indexing_coverage(
        &self,
        request: GetSemanticIndexingCoverageRequest,
    ) -> Result<SemanticIndexingCoverage, A2aLabError>;

    /// Active model space and description generation advertised to clients.
    fn active_provenance(&self) -> Result<SemanticProvenanceSelector, A2aLabError>;
}

/// Stable unavailable error for absent semantic capabilities.
#[must_use]
pub fn semantic_search_unavailable() -> A2aLabError {
    A2aLabError::unavailable("semantic frame search is not available")
}
