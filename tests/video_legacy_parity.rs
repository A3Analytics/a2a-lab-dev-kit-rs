#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    LAB_MEDIA_TYPE, ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE, McpLab,
    McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, Page, PageRequest,
    TckMalformedVideoRequests, TimeRange, UtcTimestamp, VideoClip, VideoClipStream, VideoFollow,
    VideoProvider, VideoSource, VideoSourceId, VideoTimeline, bind_local, video_clip_path,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

include!("support/video_wire.rs");
include!("support/legacy_harness.rs");
include!("support/legacy_tests.rs");
