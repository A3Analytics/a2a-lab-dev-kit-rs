//! Video sources, frame-at-time lookup, timelines, live follow, and bounded clips.
//!
//! Saved images stay in the images domain. [`CapturedColorFrame`] is an owned
//! capture snapshot and is distinct from delivery [`VideoCodec`]. This crate
//! does not open cameras and does not persist recordings.

mod capture;
mod capture_profile;
mod capture_session;
mod catalog;
mod cmaf;
mod codec;
#[cfg(feature = "yuv")]
mod color_convert;
#[cfg(all(feature = "yuv", feature = "openh264"))]
mod color_pipeline;
mod dvr_store;
mod epoch_hls;
mod frame_select;
#[cfg(feature = "openh264")]
mod h264_encode;
mod h264_sample;
pub(crate) mod http;
#[cfg(feature = "yuv")]
mod jpeg_derivative;
mod local_media;
mod media_asset;
#[cfg(feature = "yuv")]
mod mjpeg_decode;
mod model;
#[cfg(feature = "nokhwa")]
mod nokhwa_device;
mod packager;
mod provider;
mod timeline;

pub use capture::{
    CaptureCapabilities, CaptureClock, CaptureFormat, CaptureIdentity, CaptureMode, CaptureTiming,
    CapturedColorFrame, ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange,
    MemoryColorCapture,
};
pub use capture_profile::{
    AdvertisedCaptureProfile, CaptureFormatPreference, CaptureProfileError, CaptureProfileRequest,
    CaptureRate, DEFAULT_CAPTURE_OUTPUT_FPS, SelectedCaptureProfile, confirm_selected_profile,
    select_capture_profile,
};
pub use capture_session::{CaptureSession, ColorCamera, RawCameraSample, captured_frame};
pub use catalog::{FinitePlaybackSession, LivePlaybackSession, VideoCatalog};
pub use codec::VideoCodec;
#[cfg(feature = "yuv")]
pub use color_convert::{
    InterleavedColor, InterleavedRgb, MAX_ENCODER_EDGE, OwnedI420, PackedYuy2, rgb_to_i420,
    rgb_to_yuy2, yuy2_to_i420,
};
#[cfg(all(feature = "yuv", feature = "openh264"))]
pub use color_pipeline::{ColorEncodeCounters, ColorEncodePipeline, ColorEncodedFrame};
pub use dvr_store::{
    DvrDiscontinuity, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPayload,
    DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrPictureProperties, DvrRetentionBounds,
    H264TrackRelation, ReadDvrMediaWindowRequest, h264_track_relation,
};
pub use epoch_hls::{
    EpochHlsInit, EpochHlsSegment, EpochHlsSession, EpochPlaylistKind, FiniteH264Clip,
    export_finite_h264_epochs, package_h264_epochs,
};
pub use frame_select::{FrameRejection, FreshFrameGate, OUTPUT_5FPS_PERIOD_NS, RejectedFrame};
#[cfg(feature = "openh264")]
pub use h264_encode::{
    AllIdrEncoder, EncodedAccessUnit, I420Picture, OPENH264_NOTICE, decode_luma,
};
pub use h264_sample::{H264PictureSample, frame_h264_picture};
pub use http::{apply_media_security_headers, is_gated_media_path, router, router_for_lab};
#[cfg(feature = "yuv")]
pub use jpeg_derivative::{
    JpegDerivative, JpegDerivativeCache, JpegDerivativeLimits, MAX_JPEG_QUALITY, MIN_JPEG_QUALITY,
    jpeg_derivative,
};
pub use local_media::LocalMediaAssetProvider;
pub use media_asset::{
    ClipAssetRequest, LiveAssetRequest, MediaAssetProvider, ResolvedMediaAsset,
    SharedMediaAssetProvider, media_query_carries_credential, normalize_media_asset_path,
    validate_media_asset_path,
};
#[cfg(feature = "yuv")]
pub use mjpeg_decode::{
    DecodedMjpeg, DecodedPicture, MAX_MJPEG_BYTES, MjpegDecodeLimits, MjpegDecodeTarget,
    decode_mjpeg,
};
pub use model::{
    DEFAULT_MJPEG_BOUNDARY, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE,
    TckMalformedVideoRequests, VIDEO_CLIP_PATH_PREFIX, VideoClip, VideoClipStream, VideoFrame,
    VideoPlayback8xMetadata, VideoSource, decode_mjpeg_multipart, encode_mjpeg_end,
    encode_mjpeg_multipart, encode_mjpeg_part, mjpeg_content_type, video_clip_path,
};
#[cfg(feature = "nokhwa")]
pub use nokhwa_device::{NokhwaCamera, profiles_from_nokhwa};
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
