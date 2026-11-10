//! Bounded scopes for frame decision history reads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::{SavedFrameId, VideoSourceId};
use crate::time::TimeRange;
use crate::videos::saved_frame::SavedFrameRef;

/// Half-open source/time scope with an optional exact frame revision pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionReadScope {
    source_id: VideoSourceId,
    range: TimeRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame: Option<SavedFrameRef>,
}

impl FrameDecisionReadScope {
    /// Creates a scope when the range is valid.
    pub fn new(
        source_id: VideoSourceId,
        range: TimeRange,
        frame: Option<SavedFrameRef>,
    ) -> Result<Self, A2aLabError> {
        range.check()?;
        if let Some(frame) = &frame
            && frame.source_id() != &source_id
        {
            return Err(A2aLabError::invalid(
                "frame",
                "frame revision must match scope source_id",
            ));
        }
        Ok(Self {
            source_id,
            range,
            frame,
        })
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

    /// Optional exact frame revision pin.
    #[must_use]
    pub const fn frame(&self) -> Option<&SavedFrameRef> {
        self.frame.as_ref()
    }

    /// Whether `record` lies inside this scope.
    #[must_use]
    pub fn contains(
        &self,
        record: &crate::videos::frame_decision::record::FrameDecisionRecord,
    ) -> bool {
        if record.frame().source_id() != &self.source_id {
            return false;
        }
        let captured = record.frame().captured_at();
        if captured < self.range.start() || captured >= self.range.end() {
            return false;
        }
        if let Some(frame) = &self.frame
            && record.frame().frame_id() != frame.frame_id()
        {
            return false;
        }
        true
    }

    /// Whether `frame_id` matches the optional pin.
    #[must_use]
    pub fn matches_frame_id(&self, frame_id: &SavedFrameId) -> bool {
        self.frame
            .as_ref()
            .is_none_or(|frame| frame.frame_id() == frame_id)
    }

    /// Builds a scope with a reversed range for malformed-request fixtures.
    #[doc(hidden)]
    #[must_use]
    pub fn tck_reversed_range(source_id: VideoSourceId, range: TimeRange) -> Self {
        Self {
            source_id,
            range: TimeRange::new(range.end(), range.start()).unwrap_or(range),
            frame: None,
        }
    }
}
