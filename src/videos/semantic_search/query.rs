//! Text-to-frame and frame-to-frame search query shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::TimeRange;
use crate::videos::saved_frame::SavedFrameRef;

/// Finite retained source and half-open UTC scope for one search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticSearchScope {
    source_id: VideoSourceId,
    range: TimeRange,
}

impl SemanticSearchScope {
    /// Creates scope when `range` is valid and non-empty.
    pub fn new(source_id: VideoSourceId, range: TimeRange) -> Result<Self, A2aLabError> {
        range.check()?;
        Ok(Self { source_id, range })
    }

    /// Saved source filter.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Half-open UTC interval `[start, end)`.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }
}

/// Exactly one retrieval query: nonblank text or a saved reference revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticSearchQuery {
    /// Encodes query text in the configured text-embedding space.
    Text {
        /// Nonblank natural-language query.
        text: String,
    },
    /// Reuses the reference frame's indexed description and vector.
    ReferenceFrame {
        /// Indexed saved revision. No secondary image encoder is implied.
        frame: SavedFrameRef,
    },
}

impl SemanticSearchQuery {
    /// Validates mutual exclusivity and nonblank text.
    pub fn check(&self) -> Result<(), A2aLabError> {
        match self {
            Self::Text { text } => {
                if text.trim().is_empty() {
                    return Err(A2aLabError::invalid("text", "must be nonblank"));
                }
            }
            Self::ReferenceFrame { frame: _ } => {}
        }
        Ok(())
    }

    /// Nonblank text when this is a text query.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text.as_str()),
            Self::ReferenceFrame { .. } => None,
        }
    }

    /// Reference revision when this is a frame query.
    #[must_use]
    pub fn reference_frame(&self) -> Option<&SavedFrameRef> {
        match self {
            Self::Text { .. } => None,
            Self::ReferenceFrame { frame } => Some(frame),
        }
    }

    /// Whether the query reuses an indexed saved frame vector.
    #[must_use]
    pub fn is_reference_frame(&self) -> bool {
        matches!(self, Self::ReferenceFrame { .. })
    }
}
