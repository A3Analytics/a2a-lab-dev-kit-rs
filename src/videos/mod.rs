//! Video sources, frame-at-time lookup, timelines, live follow, and bounded MJPEG clips.
//!
//! Saved images stay in the images domain. This crate does not capture cameras
//! and does not persist recordings.

pub(crate) mod http;
mod model;
mod provider;
mod timeline;

pub use model::{
    DEFAULT_MJPEG_BOUNDARY, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE,
    TckMalformedVideoRequests, VIDEO_CLIP_PATH_PREFIX, VideoClip, VideoClipStream, VideoFrame,
    VideoSource, decode_mjpeg_multipart, encode_mjpeg_end, encode_mjpeg_multipart,
    encode_mjpeg_part, mjpeg_content_type, video_clip_path,
};
pub use provider::VideoProvider;
pub use timeline::{
    FollowVideoRequest, GetVideoTimelineRequest, VIDEO_LIVE_PATH_PREFIX, VideoFollow,
    VideoLiveStream, VideoTimeline, video_live_path,
};
