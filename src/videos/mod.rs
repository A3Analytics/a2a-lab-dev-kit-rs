//! Video sources, frame-at-time lookup, timelines, live follow, and bounded clips.
//!
//! Saved images stay in the images domain. [`CapturedColorFrame`] is an owned
//! capture snapshot and is distinct from delivery [`VideoCodec`]. This crate
//! does not open cameras and does not persist recordings.

mod capture;
mod catalog;
mod cmaf;
mod codec;
#[cfg(feature = "yuv")]
mod color_convert;
mod dvr_store;
#[cfg(feature = "openh264")]
mod h264_encode;
mod h264_sample;
pub(crate) mod http;
mod local_media;
mod media_asset;
mod model;
mod packager;
mod provider;
mod timeline;

pub use capture::{
    CaptureCapabilities, CaptureClock, CaptureFormat, CaptureIdentity, CaptureMode, CaptureTiming,
    CapturedColorFrame, ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange,
    MemoryColorCapture,
};
pub use catalog::{FinitePlaybackSession, LivePlaybackSession, VideoCatalog};
pub use codec::VideoCodec;
#[cfg(feature = "yuv")]
pub use color_convert::{
    InterleavedColor, InterleavedRgb, MAX_ENCODER_EDGE, OwnedI420, PackedYuy2, rgb_to_yuy2,
    yuy2_to_i420,
};
pub use dvr_store::{
    DvrDiscontinuity, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPayload,
    DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrRetentionBounds, ReadDvrMediaWindowRequest,
};
#[cfg(feature = "openh264")]
pub use h264_encode::{
    AllIdrEncoder, EncodedAccessUnit, I420Picture, OPENH264_NOTICE, decode_luma,
};
pub use h264_sample::{H264PictureSample, frame_h264_picture};
pub use http::{apply_media_security_headers, is_gated_media_path, router, router_for_lab};
pub use local_media::LocalMediaAssetProvider;
pub use media_asset::{
    ClipAssetRequest, LiveAssetRequest, MediaAssetProvider, ResolvedMediaAsset,
    SharedMediaAssetProvider, media_query_carries_credential, normalize_media_asset_path,
    validate_media_asset_path,
};
pub use model::{
    DEFAULT_MJPEG_BOUNDARY, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE,
    TckMalformedVideoRequests, VIDEO_CLIP_PATH_PREFIX, VideoClip, VideoClipStream, VideoFrame,
    VideoPlayback8xMetadata, VideoSource, decode_mjpeg_multipart, encode_mjpeg_end,
    encode_mjpeg_multipart, encode_mjpeg_part, mjpeg_content_type, video_clip_path,
};
pub use packager::{
    DEFAULT_PLAYBACK_WINDOW_US, DvrMediaPackager, DvrPackagerConfig, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, PackagedHlsSegment,
    PackagedHlsSession, PackagedHlsTrick8xRendition, PackagedMediaAsset,
    TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES, TRICK_8X_UNAVAILABLE_NO_REDUCTION,
};
pub use provider::VideoProvider;
pub use timeline::{
    FollowVideoRequest, GetVideoTimelineRequest, VIDEO_LIVE_PATH_PREFIX, VideoFollow,
    VideoLiveStream, VideoTimeline, video_live_path,
};
