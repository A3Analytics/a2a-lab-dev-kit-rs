//! Description and embedding generation selectors for one model space.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::videos::index_provenance::FrameVectorIndexAttachment;
use crate::videos::model_space::{ModelSpaceIdentity, ProvenanceGeneration};

/// Declares the embedding index namespace and description generation for search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticProvenanceSelector {
    model_space: ModelSpaceIdentity,
    description_generation: ProvenanceGeneration,
}

impl SemanticProvenanceSelector {
    /// Binds search to one compatible description/embedding generation set.
    pub fn new(
        model_space: ModelSpaceIdentity,
        description_generation: ProvenanceGeneration,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            model_space,
            description_generation,
        })
    }

    /// Target vector index namespace.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }

    /// Description generation required for hits in this search.
    #[must_use]
    pub const fn description_generation(&self) -> ProvenanceGeneration {
        self.description_generation
    }

    /// Embedding generation implied by `model_space`.
    #[must_use]
    pub fn embedding_generation(&self) -> ProvenanceGeneration {
        self.model_space.generation()
    }

    /// Whether an indexed attachment satisfies this selector.
    #[must_use]
    pub fn accepts_attachment(&self, attachment: &FrameVectorIndexAttachment) -> bool {
        attachment.model_space() == &self.model_space
            && attachment.description().generation() == self.description_generation
            && attachment.validate().is_ok()
    }

    /// Rejects mixed-space requests at validation time.
    pub fn check_attachment(
        &self,
        attachment: &FrameVectorIndexAttachment,
    ) -> Result<(), A2aLabError> {
        if attachment.model_space() != &self.model_space {
            return Err(A2aLabError::invalid(
                "model_space",
                "hit attachment does not match the requested model space",
            ));
        }
        if attachment.description().generation() != self.description_generation {
            return Err(A2aLabError::invalid(
                "description_generation",
                "hit attachment does not match the requested description generation",
            ));
        }
        attachment.validate()?;
        Ok(())
    }
}
