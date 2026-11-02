//! Rust dev kit for lab logs, metrics, tasks, images, and videos.
//!
//! Provider traits are the source of truth. The A2A adapter (HTTP+JSON, JSON-RPC,
//! and gRPC) and the MCP adapter expose logs, metrics, tasks, images, and videos.
//! The optional `web-ui` feature packages a Rust browser shell. `A2aServer::with_web_ui`
//! serves that shell at `/ui/` on the A2A HTTP listener.

#![allow(clippy::doc_markdown)]

pub mod a2a;
#[cfg(feature = "aas")]
pub mod aas;
pub mod catalog;
pub mod error;
pub mod id;
pub mod images;
pub mod industrial;
pub mod json_object;
pub mod logs;
pub mod mcp;
pub mod memory;
pub mod metrics;
#[cfg(feature = "opcua")]
pub mod opcua;
pub mod page;
pub mod ros2;
pub mod service;
#[cfg(feature = "sila2")]
pub mod sila;
pub mod tasks;
pub mod time;
pub mod videos;

#[cfg(feature = "web-ui")]
pub use a2a::WEB_UI_PATH;
pub use a2a::{
    A2A_PROTOCOL_VERSION, A2aClient, A2aServer, AgentCard, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, AgentMessageResponse, AuthError, Authenticator,
    HttpAuthSecurityScheme, LAB_MEDIA_TYPE, OidcAuthenticator, OpenIdConnectSecurityScheme,
    Principal, SecurityScheme, StreamResponse, Task, TaskPushNotificationConfig, bind_local,
};
pub use catalog::{
    Asset, AssetCatalogProvider, AssetKey, Binding, BindingRole, Endpoint, ListAssetsRequest,
    ListBindingsRequest, OpcUaIdentityKind, ProtocolKind, SecurityMode, SemanticId, SemanticKind,
};
pub use error::{A2aLabError, VideoAvailability};
pub use id::{
    DvrTrackId, ImageId, ImageSourceId, MetricId, RunId, SourceId, TaskId, VideoClipId,
    VideoFollowId, VideoSourceId,
};
pub use images::{
    DEFAULT_MAX_IMAGE_BYTES, GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor,
    ImageProvider, ImageSource, ImageTransportConfig, ListImageSourcesRequest, ListImagesRequest,
    SearchImagesRequest, TckMalformedImageRequests,
};
pub use industrial::{
    IndustrialLabBuilder, IndustrialLogs, IndustrialMetrics, IndustrialTasks, ScriptedLive,
};
pub use json_object::JsonObject;
pub use logs::{
    ListLogSourcesRequest, LogLevel, LogProvider, LogRecord, LogSource, QueryLogsRequest,
};
pub use mcp::{DEFAULT_MCP_URL, McpLab, McpServer};
pub use memory::{
    MemoryCatalog, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos,
};
pub use metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
pub use page::{MAX_PAGE_LIMIT, Page, PageRequest};
pub use ros2::{MemoryRos2, Ros2Action, Ros2Goal, Ros2GoalStatus, Ros2Graph, Ros2Tasks};
pub use service::{
    A2aLabApi, A2aLabCommand, A2aLabFuture, A2aLabOutcome, A2aLabResult, A2aLabService,
    TaskSnapshot,
};
pub use tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState, start_run,
};
pub use time::{TimeRange, UtcTimestamp};
pub use videos::{
    ClipAssetRequest, DEFAULT_MJPEG_BOUNDARY, DEFAULT_PLAYBACK_WINDOW_US, DvrDiscontinuity,
    DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPackager, DvrMediaPayload,
    DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrPackagerConfig, DvrRetentionBounds,
    FinitePlaybackSession, FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, ListVideoSourcesRequest,
    LiveAssetRequest, LivePlaybackSession, LocalMediaAssetProvider, MAX_VIDEO_CLIP_DURATION_US,
    MJPEG_MEDIA_TYPE, MediaAssetProvider, PackagedHlsSegment, PackagedHlsSession,
    PackagedHlsTrick8xRendition, PackagedMediaAsset, ReadDvrMediaWindowRequest, ResolvedMediaAsset,
    SharedMediaAssetProvider, TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES,
    TRICK_8X_UNAVAILABLE_NO_REDUCTION, TckMalformedVideoRequests, VIDEO_CLIP_PATH_PREFIX,
    VIDEO_LIVE_PATH_PREFIX, VideoCatalog, VideoClip, VideoClipStream, VideoCodec, VideoFollow,
    VideoFrame, VideoLiveStream, VideoPlayback8xMetadata, VideoProvider, VideoSource,
    VideoTimeline, decode_mjpeg_multipart, encode_mjpeg_end, encode_mjpeg_multipart,
    encode_mjpeg_part, mjpeg_content_type, router, router_for_lab, video_clip_path,
    video_live_path,
};

/// Returns the crate version.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn reports_package_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }
}
