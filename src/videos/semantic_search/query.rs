//! Text-to-frame and frame-to-frame search query shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::TimeRange;
use crate::videos::saved_frame::SavedFrameRef;

/// Finite retained source scope with an optional half-open UTC candidate filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SemanticSearchScope {
    source_id: VideoSourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    range: Option<TimeRange>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticSearchScopeRaw {
    source_id: VideoSourceId,
    #[serde(default)]
    range: Option<TimeRange>,
}

impl SemanticSearchScope {
    /// Creates scope when an optional `range` is valid.
    pub fn new(source_id: VideoSourceId, range: Option<TimeRange>) -> Result<Self, A2aLabError> {
        let scope = Self { source_id, range };
        scope.check()?;
        Ok(scope)
    }

    /// Rejects a reversed or empty optional interval.
    pub fn check(&self) -> Result<(), A2aLabError> {
        if let Some(range) = self.range {
            range.check()?;
        }
        Ok(())
    }

    /// Saved source filter.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Optional half-open UTC interval `[start, end)` restricting search candidates.
    #[must_use]
    pub const fn range(&self) -> Option<TimeRange> {
        self.range
    }

    /// Whether `timestamp` would be considered inside the optional candidate filter.
    #[must_use]
    pub fn contains_capture(&self, timestamp: crate::time::UtcTimestamp) -> bool {
        self.range.is_none_or(|range| range.contains(timestamp))
    }
}

impl<'de> Deserialize<'de> for SemanticSearchScope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SemanticSearchScopeRaw::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl TryFrom<SemanticSearchScopeRaw> for SemanticSearchScope {
    type Error = A2aLabError;

    fn try_from(raw: SemanticSearchScopeRaw) -> Result<Self, Self::Error> {
        Self::new(raw.source_id, raw.range)
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
