//! Bounded semantic search and coverage requests.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::page::PageRequest;
use crate::videos::semantic_search::decision_filter::DecisionSearchFilter;
use crate::videos::semantic_search::limits::SemanticSearchLimits;
use crate::videos::semantic_search::provenance::SemanticProvenanceSelector;
use crate::videos::semantic_search::query::{SemanticSearchQuery, SemanticSearchScope};

/// Search saved frames by text or reference frame within a finite scope.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchSemanticFramesRequest {
    scope: SemanticSearchScope,
    query: SemanticSearchQuery,
    provenance: SemanticProvenanceSelector,
    limits: SemanticSearchLimits,
    page: PageRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_filter: Option<DecisionSearchFilter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchSemanticFramesRequestRaw {
    scope: SemanticSearchScope,
    query: SemanticSearchQuery,
    provenance: SemanticProvenanceSelector,
    limits: SemanticSearchLimits,
    page: PageRequest,
    #[serde(default)]
    decision_filter: Option<DecisionSearchFilter>,
}

impl SearchSemanticFramesRequest {
    /// Validates scope, query, provenance, paging, and optional decision filter.
    pub fn new(
        scope: SemanticSearchScope,
        query: SemanticSearchQuery,
        provenance: SemanticProvenanceSelector,
        limits: SemanticSearchLimits,
        page: PageRequest,
        decision_filter: Option<DecisionSearchFilter>,
    ) -> Result<Self, A2aLabError> {
        let request = Self {
            scope,
            query,
            provenance,
            limits,
            page,
            decision_filter,
        };
        request.check()?;
        Ok(request)
    }

    /// Rejects malformed scope, query, limits, and page combinations.
    pub fn check(&self) -> Result<(), A2aLabError> {
        self.scope.check()?;
        self.query.check()?;
        self.limits.check()?;
        self.page.check()?;
        if self.page.limit() > self.limits.top_k() {
            return Err(A2aLabError::invalid("page.limit", "must not exceed top_k"));
        }
        Ok(())
    }

    /// Source and UTC scope.
    #[must_use]
    pub const fn scope(&self) -> &SemanticSearchScope {
        &self.scope
    }

    /// Text or saved reference frame query.
    #[must_use]
    pub const fn query(&self) -> &SemanticSearchQuery {
        &self.query
    }

    /// Compatible description and embedding generations.
    #[must_use]
    pub const fn provenance(&self) -> &SemanticProvenanceSelector {
        &self.provenance
    }

    /// Top-k, candidate, and neighbor limits.
    #[must_use]
    pub const fn limits(&self) -> SemanticSearchLimits {
        self.limits
    }

    /// Result page bounds.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }

    /// Optional Jev decision filter applied before final ranking.
    #[must_use]
    pub const fn decision_filter(&self) -> Option<&DecisionSearchFilter> {
        self.decision_filter.as_ref()
    }
}

impl TryFrom<SearchSemanticFramesRequestRaw> for SearchSemanticFramesRequest {
    type Error = A2aLabError;

    fn try_from(raw: SearchSemanticFramesRequestRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.scope,
            raw.query,
            raw.provenance,
            raw.limits,
            raw.page,
            raw.decision_filter,
        )
    }
}

impl<'de> Deserialize<'de> for SearchSemanticFramesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SearchSemanticFramesRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Coverage accounting for semantic indexing inside one scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetSemanticIndexingCoverageRequest {
    scope: SemanticSearchScope,
    provenance: SemanticProvenanceSelector,
}

impl GetSemanticIndexingCoverageRequest {
    /// Creates a coverage request.
    pub fn new(
        scope: SemanticSearchScope,
        provenance: SemanticProvenanceSelector,
    ) -> Result<Self, A2aLabError> {
        scope.check()?;
        Ok(Self { scope, provenance })
    }

    /// Source and UTC scope.
    #[must_use]
    pub const fn scope(&self) -> &SemanticSearchScope {
        &self.scope
    }

    /// Generation set under inspection.
    #[must_use]
    pub const fn provenance(&self) -> &SemanticProvenanceSelector {
        &self.provenance
    }
}

/// Constructors reserved for malformed semantic search conformance fixtures.
pub struct TckMalformedSemanticSearchRequests;

impl TckMalformedSemanticSearchRequests {
    /// Search with blank text.
    #[must_use]
    pub fn search_blank_text(
        scope: SemanticSearchScope,
        provenance: SemanticProvenanceSelector,
    ) -> SearchSemanticFramesRequest {
        SearchSemanticFramesRequest {
            scope,
            query: SemanticSearchQuery::Text {
                text: "   ".to_owned(),
            },
            provenance,
            limits: SemanticSearchLimits::pilot(),
            page: PageRequest::default(),
            decision_filter: None,
        }
    }

    /// Search with zero page limit.
    #[must_use]
    pub fn search_zero_page_limit(
        scope: SemanticSearchScope,
        query: SemanticSearchQuery,
        provenance: SemanticProvenanceSelector,
    ) -> SearchSemanticFramesRequest {
        SearchSemanticFramesRequest {
            scope,
            query,
            provenance,
            limits: SemanticSearchLimits::pilot(),
            page: PageRequest::tck_zero_limit(),
            decision_filter: None,
        }
    }

    /// Search whose page size exceeds `top_k`.
    #[must_use]
    pub fn search_page_exceeds_top_k(
        scope: SemanticSearchScope,
        query: SemanticSearchQuery,
        provenance: SemanticProvenanceSelector,
    ) -> SearchSemanticFramesRequest {
        SearchSemanticFramesRequest {
            scope,
            query,
            provenance,
            limits: SemanticSearchLimits::new(5, 100, 3).unwrap(),
            page: PageRequest::new(None, 10).unwrap(),
            decision_filter: None,
        }
    }
}
