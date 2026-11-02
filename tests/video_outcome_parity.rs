#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, HttpAuthSecurityScheme, LAB_MEDIA_TYPE, ListVideoSourcesRequest,
    MJPEG_MEDIA_TYPE, McpLab, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos,
    Page, PageRequest, SecurityScheme, TimeRange, UtcTimestamp, VideoClip, VideoClipStream,
    VideoFollow, VideoFollowId, VideoLiveStream, VideoProvider, VideoSource, VideoSourceId,
    VideoTimeline, bind_local,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

include!("support/video_wire.rs");
include!("support/outcome_tests.rs");
