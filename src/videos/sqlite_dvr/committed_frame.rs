//! Metadata passed to [`super::DvrPersistHook`] after commit.

use crate::id::{DvrTrackId, VideoSourceId};
use crate::time::UtcTimestamp;

/// One chunk-indexed frame that survived metadata commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrCommittedFrame {
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    start: UtcTimestamp,
    end: UtcTimestamp,
    keyframe: bool,
    chunk_relpath: String,
    payload_crc32: u32,
    h264_epoch_id: Option<i64>,
}

impl DvrCommittedFrame {
    pub(crate) fn new(
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        start: UtcTimestamp,
        end: UtcTimestamp,
        keyframe: bool,
        chunk_relpath: String,
        payload_crc32: u32,
        h264_epoch_id: Option<i64>,
    ) -> Self {
        Self {
            source_id,
            track_id,
            start,
            end,
            keyframe,
            chunk_relpath,
            payload_crc32,
            h264_epoch_id,
        }
    }

    /// Indexed source.
    #[must_use]
    pub fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Track that owns the payload.
    #[must_use]
    pub fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Capture start instant.
    #[must_use]
    pub const fn start(&self) -> UtcTimestamp {
        self.start
    }

    /// Half-open segment end instant.
    #[must_use]
    pub const fn end(&self) -> UtcTimestamp {
        self.end
    }

    /// Whether this sample is a keyframe.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }

    /// Minute chunk relative path under the store root.
    #[must_use]
    pub fn chunk_relpath(&self) -> &str {
        &self.chunk_relpath
    }

    /// CRC32 of the stored payload bytes.
    #[must_use]
    pub const fn payload_crc32(&self) -> u32 {
        self.payload_crc32
    }

    /// H.264 epoch when `track_id` is `h264`.
    #[must_use]
    pub const fn h264_epoch_id(&self) -> Option<i64> {
        self.h264_epoch_id
    }
}
