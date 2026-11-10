//! Capability discovery for semantic search transport APIs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Uses an empty object on the wire so MCP `inputSchema` roots as `object`, not `null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct GetSemanticSearchCapabilitiesRequest {}

impl GetSemanticSearchCapabilitiesRequest {
    /// Default empty request accepted by A2A and MCP adapters.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

/// Search and coverage operations exposed when a semantic provider is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchOperation {
    /// Scoped text or reference-frame retrieval.
    SearchSemanticFrames,
    /// Per-state indexing counts for one scope and generation.
    GetSemanticIndexingCoverage,
}

/// JSON Schema type names advertised during capability discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticSearchSchemaRefs {
    search_request: String,
    search_result: String,
    coverage_request: String,
    coverage_result: String,
}

impl SemanticSearchSchemaRefs {
    /// Stable schema names for registered search and coverage APIs.
    #[must_use]
    pub fn registered() -> Self {
        Self {
            search_request: "SearchSemanticFramesRequest".to_owned(),
            search_result: "SemanticFrameSearchResult".to_owned(),
            coverage_request: "GetSemanticIndexingCoverageRequest".to_owned(),
            coverage_result: "SemanticIndexingCoverage".to_owned(),
        }
    }

    /// Search request schema name.
    #[must_use]
    pub fn search_request(&self) -> &str {
        &self.search_request
    }

    /// Search result schema name.
    #[must_use]
    pub fn search_result(&self) -> &str {
        &self.search_result
    }

    /// Coverage request schema name.
    #[must_use]
    pub fn coverage_request(&self) -> &str {
        &self.coverage_request
    }

    /// Coverage result schema name.
    #[must_use]
    pub fn coverage_result(&self) -> &str {
        &self.coverage_result
    }
}

/// Capability discovery for semantic search transport APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticSearchCapabilities {
    registered: bool,
    operations: Vec<SemanticSearchOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schemas: Option<SemanticSearchSchemaRefs>,
}

impl SemanticSearchCapabilities {
    /// No provider: no schemas or operations.
    #[must_use]
    pub fn unregistered() -> Self {
        Self {
            registered: false,
            operations: Vec::new(),
            schemas: None,
        }
    }

    /// Registered provider exposing search and coverage schemas.
    #[must_use]
    pub fn registered() -> Self {
        Self {
            registered: true,
            operations: vec![
                SemanticSearchOperation::SearchSemanticFrames,
                SemanticSearchOperation::GetSemanticIndexingCoverage,
            ],
            schemas: Some(SemanticSearchSchemaRefs::registered()),
        }
    }

    /// Whether a semantic search provider is configured.
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        self.registered
    }

    /// Advertised search and coverage operations when registered.
    #[must_use]
    pub fn operations(&self) -> &[SemanticSearchOperation] {
        &self.operations
    }

    /// Request/result JSON Schema type names when registered.
    #[must_use]
    pub const fn schemas(&self) -> Option<&SemanticSearchSchemaRefs> {
        self.schemas.as_ref()
    }
}
