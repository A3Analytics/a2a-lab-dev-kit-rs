//! Format-neutral video catalog and opaque playback-session contracts.
//!
//! Catalog operations stay separate from DVR storage, packaging, and HTTP transport.
//! [`VideoCatalog`] marks the control-plane surface on [`VideoProvider`]: listing,
//! timelines, frame-at-time lookup, and opaque session preparation.

use crate::videos::model::VideoClip;
use crate::videos::timeline::VideoFollow;

/// Opaque finite playback session returned from the catalog.
pub type FinitePlaybackSession = VideoClip;

/// Opaque live playback session returned from the catalog.
pub type LivePlaybackSession = VideoFollow;

/// Control-plane catalog contract: sources, timelines, frames, and session handles.
///
/// Every [`VideoProvider`] implements this marker. Catalog methods do not require
/// HLS playlists, CMAF segments, MP4 exports, HTTP responses, or authentication.
pub trait VideoCatalog: crate::videos::VideoProvider {}

impl<T: crate::videos::VideoProvider + ?Sized> VideoCatalog for T {}
