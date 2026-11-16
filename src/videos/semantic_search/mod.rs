//! Bounded semantic frame search and indexing coverage contracts.

mod api;
mod assessments;
mod chunk_provider;
mod decision_filter;
mod fakes;
mod limits;
mod provenance;
mod provider;
mod query;
mod ranking;
mod request;
mod result;

pub use api::{
    GetSemanticSearchCapabilitiesRequest, SemanticSearchCapabilities, SemanticSearchOperation,
    SemanticSearchSchemaRefs,
};

pub use assessments::{SemanticFrameHitAssessments, authorized_assessments_for_frame};
pub use chunk_provider::ChunkDvrSemanticSearchProvider;
pub use decision_filter::{DecisionHistoryPolicy, DecisionSearchFilter};
pub use fakes::{FakeSemanticHit, FakeSemanticVideoSearchProvider, fake_semantic_hit};
pub use limits::{
    DEFAULT_SEMANTIC_CANDIDATES, DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS, DEFAULT_SEMANTIC_TOP_K,
    MAX_SEMANTIC_CANDIDATES, MAX_SEMANTIC_HIT_ASSESSMENT_ROWS, MAX_SEMANTIC_NEIGHBOR_FRAMES,
    MAX_SEMANTIC_TOP_K, SemanticSearchLimits,
};
pub use provenance::SemanticProvenanceSelector;
pub use provider::{SemanticVideoSearchProvider, semantic_search_unavailable};
pub use query::{SemanticSearchQuery, SemanticSearchScope};
pub use request::{
    GetSemanticIndexingCoverageRequest, SearchSemanticFramesRequest,
    TckMalformedSemanticSearchRequests,
};
pub use result::{
    SemanticFrameHit, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticFrameSearchResult, SemanticIndexingCoverage,
    SemanticIndexingFreshness, SemanticIndexingStateCounts, SemanticRetrievalScore,
};
