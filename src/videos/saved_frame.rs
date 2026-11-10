//! Immutable saved-frame revision identity for semantic indexing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::A2aLabError;
use crate::id::{CaptureGroupId, DvrStoreNamespace, DvrTrackId, SavedFrameId, VideoSourceId};
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;

/// Reference to one committed saved media revision.
///
/// The [`SavedFrameRef::frame_id`] binds store identity, source, track epoch,
/// original capture instant, and payload bytes. Payload replacement at the same
/// DVR key produces a different revision and cannot inherit prior decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedFrameRef {
    frame_id: SavedFrameId,
    store: DvrStoreNamespace,
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    captured_at: UtcTimestamp,
    payload_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    capture_group: Option<CaptureGroupId>,
}

impl SavedFrameRef {
    /// Builds a revision reference and derives its stable [`SavedFrameId`].
    pub fn new(
        store: DvrStoreNamespace,
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        captured_at: UtcTimestamp,
        payload_digest: Sha256Digest,
        capture_group: Option<CaptureGroupId>,
    ) -> Result<Self, A2aLabError> {
        let frame_id =
            derive_frame_id(&store, &source_id, &track_id, captured_at, &payload_digest)?;
        Ok(Self {
            frame_id,
            store,
            source_id,
            track_id,
            captured_at,
            payload_digest,
            capture_group,
        })
    }

    /// Constructs a reference from a payload byte slice.
    pub fn from_payload_bytes(
        store: DvrStoreNamespace,
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        captured_at: UtcTimestamp,
        payload: &[u8],
        capture_group: Option<CaptureGroupId>,
    ) -> Result<Self, A2aLabError> {
        Self::new(
            store,
            source_id,
            track_id,
            captured_at,
            Sha256Digest::hash_bytes(payload),
            capture_group,
        )
    }

    /// Stable immutable revision identity.
    #[must_use]
    pub const fn frame_id(&self) -> &SavedFrameId {
        &self.frame_id
    }

    /// Persisted DVR store namespace.
    #[must_use]
    pub const fn store(&self) -> &DvrStoreNamespace {
        &self.store
    }

    /// Saved video source.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Track or codec epoch.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Original capture instant in UTC.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Immutable payload revision digest.
    #[must_use]
    pub const fn payload_digest(&self) -> &Sha256Digest {
        &self.payload_digest
    }

    /// Optional verified original-capture grouping.
    #[must_use]
    pub const fn capture_group(&self) -> Option<&CaptureGroupId> {
        self.capture_group.as_ref()
    }

    /// Reports whether `other` overwrites the same DVR lookup key with different bytes.
    #[must_use]
    pub fn is_payload_replacement(&self, other: &Self) -> bool {
        self.store == other.store
            && self.source_id == other.source_id
            && self.track_id == other.track_id
            && self.captured_at == other.captured_at
            && self.payload_digest != other.payload_digest
    }

    /// Prior model decisions must not carry to a replacement revision.
    #[must_use]
    pub fn decisions_may_inherit_from(&self, prior: &Self) -> bool {
        self.frame_id() == prior.frame_id()
    }
}

/// Derives a stable saved-frame revision identifier.
pub fn derive_frame_id(
    store: &DvrStoreNamespace,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    captured_at: UtcTimestamp,
    payload_digest: &Sha256Digest,
) -> Result<SavedFrameId, A2aLabError> {
    let canonical = format!(
        "saved-frame/v1\0{}\0{}\0{}\0{}\0{}",
        store.as_str(),
        source_id.as_str(),
        track_id.as_str(),
        captured_at.as_micros(),
        payload_digest.hex(),
    );
    let digest = Sha256::digest(canonical.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    SavedFrameId::new(format!("sf:{hex}"))
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::id::{DvrStoreNamespace, DvrTrackId, VideoSourceId};

    fn store() -> DvrStoreNamespace {
        DvrStoreNamespace::new("ot2.default").unwrap()
    }

    fn source(name: &str) -> VideoSourceId {
        VideoSourceId::new(name).unwrap()
    }

    fn track(name: &str) -> DvrTrackId {
        DvrTrackId::new(name).unwrap()
    }

    fn at(value: &str) -> UtcTimestamp {
        UtcTimestamp::parse(value).unwrap()
    }

    #[test]
    fn frame_id_is_stable_for_same_revision() {
        let first = SavedFrameRef::from_payload_bytes(
            store(),
            source("cam-a"),
            track("mjpeg-0"),
            at("2024-01-01T00:00:01Z"),
            b"pixels",
            None,
        )
        .unwrap();
        let second = SavedFrameRef::from_payload_bytes(
            store(),
            source("cam-a"),
            track("mjpeg-0"),
            at("2024-01-01T00:00:01Z"),
            b"pixels",
            None,
        )
        .unwrap();
        assert_eq!(first.frame_id(), second.frame_id());
    }

    #[test]
    fn payload_replacement_changes_revision() {
        let first = SavedFrameRef::from_payload_bytes(
            store(),
            source("cam-a"),
            track("h264-1"),
            at("2024-01-01T00:00:01Z"),
            b"v1",
            None,
        )
        .unwrap();
        let second = SavedFrameRef::from_payload_bytes(
            store(),
            source("cam-a"),
            track("h264-1"),
            at("2024-01-01T00:00:01Z"),
            b"v2",
            None,
        )
        .unwrap();
        assert!(first.is_payload_replacement(&second));
        assert!(!first.decisions_may_inherit_from(&second));
    }
}
