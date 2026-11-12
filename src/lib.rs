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
pub mod config;
pub mod error;
pub mod id;
pub mod images;
pub mod industrial;
pub mod json_object;
pub mod logs;
pub mod mcp;
pub mod memory;
pub mod metrics;
pub mod observability;
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
pub use config::{
    A2A_LAB_CONFIG_SCHEMA_VERSION, A2A_LAB_CONFIG_SCHEMA_VERSION_V2, A2aLabConfig, ChatConfig,
    ChatProvider, ConfigDuration, CredentialReference, DEFAULT_LOG_OUTPUT_FORMAT,
    DEFAULT_LOGGING_LEVEL, DEFAULT_SEMANTIC_DESCRIPTION_BACKEND, DecisionCoveragePolicy,
    EmbeddingSpaceConfig, LoadedLabConfig, LogOutputFormat, LoggingConfig,
    OpenTelemetryLoggingConfig, SemanticDecisionLimits, SemanticDecisionsConfig,
    SemanticDescriptionConfig, SemanticEmbeddingsConfig, SemanticIndexingConfig,
    SemanticIndexingLimits, SemanticIndexingSearchLimits, SemanticRoiLimits, SemanticSchedule,
    SemanticStorageLimits, SemanticStreamSchedule, SemanticWorkerLimits, VideoConfig,
    VideoDvrConfig, VideoInputFormat, VideoSourceConfig,
};
pub use error::{A2aLabError, VideoAvailability};
pub use id::{
    CaptureGroupId, DecisionIdempotencyKey, DvrStoreNamespace, DvrTrackId, FrameDecisionId,
    ImageId, ImageSourceId, JevEvaluationId, MetricId, RunId, SavedFrameId, SourceId, TaskId,
    VideoClipId, VideoFollowId, VideoSourceId,
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
#[doc(hidden)]
pub use observability::reset_logging_installed_for_tests;
pub use observability::{
    LoggingGuard, init as init_logging, init_from_loaded as init_logging_from_loaded,
    is_sensitive_field, log_effective_config, log_server_ready, logging_installed, operation_name,
    redact_field, redact_free_text, trace_operation,
};
pub use page::{MAX_PAGE_LIMIT, Page, PageRequest};
pub use ros2::{MemoryRos2, Ros2Action, Ros2Goal, Ros2GoalStatus, Ros2Graph, Ros2Tasks};
pub use service::{
    A2aLabApi, A2aLabCommand, A2aLabFuture, A2aLabOutcome, A2aLabResult, A2aLabService,
    EmptyImages, EmptyVideos, TaskSnapshot,
};
pub use tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState, start_run,
};
pub use time::{TimeRange, UtcTimestamp};
pub use videos::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, ClipAssetRequest,
    DEFAULT_BEDROCK_DESCRIPTION_MODEL, DEFAULT_JEV_MODEL_REVISION, DEFAULT_MJPEG_BOUNDARY,
    DEFAULT_OPENAI_DESCRIPTION_MODEL, DEFAULT_PLAYBACK_WINDOW_US, DEFAULT_SEMANTIC_CANDIDATES,
    DEFAULT_SEMANTIC_TOP_K, DecisionEvaluationCoverage, DecisionEvidenceAvailability,
    DecisionEvidenceItem, DecisionEvidenceKind, DecisionHistoryPolicy, DecisionOutcome,
    DecisionSearchFilter, DecisionSubject, DecisionThresholdPolicy, DecisionUnknownReason,
    DvrDiscontinuity, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPackager,
    DvrMediaPayload, DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrPackagerConfig,
    DvrPictureProperties, DvrRetentionBounds, EmbeddingMetric, EpochHlsInit, EpochHlsSegment,
    EpochHlsSession, EpochPlaylistKind, FakeDescriptionBehavior, FakeEmbeddingBehavior,
    FakeFrameDecisionEvaluator, FakeFrameDecisionLedgerProvider, FakeFrameDescriptionProvider,
    FakeJevBehavior, FakeSemanticHit, FakeSemanticVideoSearchProvider, FakeTextEmbeddingProvider,
    FiniteH264Clip, FinitePlaybackSession, FollowVideoRequest, ForbiddenDecisionInput,
    ForeignObjectSubject, FrameDecisionAppendError, FrameDecisionCapabilities,
    FrameDecisionClientRole, FrameDecisionEvaluationFailure, FrameDecisionEvaluationOwnership,
    FrameDecisionEvaluationRequest, FrameDecisionEvaluationRouter, FrameDecisionEvaluator,
    FrameDecisionEvaluatorCapabilities, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionHistoryEntry, FrameDecisionHistoryResult,
    FrameDecisionLedger, FrameDecisionLedgerProvider, FrameDecisionOperation,
    FrameDecisionReadLimits, FrameDecisionReadScope, FrameDecisionRecord,
    FrameDecisionRoutingAudit, FrameDecisionSchemaRefs, FrameDescriptionBackend,
    FrameDescriptionKey, FrameDescriptionLimits, FrameDescriptionProvenance,
    FrameDescriptionProvider, FrameDescriptionProviderConfig, FrameDescriptionRequest,
    FrameEmbeddingVector, FrameRoiTransform, FrameVectorIndexAttachment,
    GetFrameDecisionCapabilitiesRequest, GetSemanticIndexingCoverageRequest,
    GetSemanticSearchCapabilitiesRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, H264PictureSample, H264TrackRelation,
    HLS_INIT_RELATIVE_PATH, HLS_PLAYLIST_RELATIVE_PATH, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH,
    JEV_EVALUATOR_PROVIDER_ID, JEV_MAX_EDGE_PIXELS, JEV_MAX_IMAGE_BYTES, JEV_MAX_REQUEST_BYTES,
    JEV_MIN_EDGE_PIXELS, JEV_PRODUCER_ID, JevEvaluationProvenance, JevProducerIdentity,
    JevStructuredOutput, KeyedFrameDecisionEvaluation, KeyedFrameDescription, KeyedTextEmbedding,
    LabPredicate, LabProbabilityVector, ListVideoSourcesRequest, LiveAssetRequest,
    LivePlaybackSession, LocalMediaAssetProvider, MAX_SEMANTIC_CANDIDATES,
    MAX_SEMANTIC_NEIGHBOR_FRAMES, MAX_SEMANTIC_TOP_K, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE,
    MediaAssetProvider, ModelSpaceIdentity, OperatorDecisionCriteria, OperatorReferenceImage,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    PackagedHlsSegment, PackagedHlsSession, PackagedHlsTrick8xRendition, PackagedMediaAsset,
    PipetteChannelSubject, PlateAlignmentReference, PlateSlotSubject, ProvenanceGeneration,
    ReadDvrMediaWindowRequest, ReadFrameDecisionHistoryRequest, RegisteredFrameDecisionEvaluator,
    RejectedDecisionEvaluatorBackend, RejectedDecisionProvenance, ResolvedMediaAsset,
    SavedFrameRef, SearchSemanticFramesRequest, SemanticFrameHit, SemanticFrameHitAssessments,
    SemanticFrameIndexState, SemanticFrameMediaAvailability, SemanticFramePlaybackNeighbors,
    SemanticFrameSearchResult, SemanticIndexingCoverage, SemanticIndexingFailure,
    SemanticIndexingFreshness, SemanticIndexingStateCounts, SemanticProvenanceSelector,
    SemanticRetrievalScore, SemanticSearchCapabilities, SemanticSearchLimits,
    SemanticSearchOperation, SemanticSearchQuery, SemanticSearchSchemaRefs, SemanticSearchScope,
    SemanticVideoSearchProvider, Sha256Digest, SharedMediaAssetProvider,
    TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES, TRICK_8X_UNAVAILABLE_NO_REDUCTION,
    TckMalformedFrameDecisionRequests, TckMalformedSemanticSearchRequests,
    TckMalformedVideoRequests, TextEmbeddingKey, TextEmbeddingLimits, TextEmbeddingProvenance,
    TextEmbeddingProvider, TextEmbeddingProviderConfig, TextEmbeddingRequest,
    VIDEO_CLIP_PATH_PREFIX, VIDEO_LIVE_PATH_PREFIX, VectorNormalization, VideoCatalog, VideoClip,
    VideoClipStream, VideoCodec, VideoFollow, VideoFrame, VideoLiveStream, VideoPlayback8xMetadata,
    VideoProvider, VideoSource, VideoTimeline, decision_history_fingerprint,
    decode_mjpeg_multipart, default_pinned_jev_revision, derive_decision_id, derive_frame_id,
    encode_mjpeg_end, encode_mjpeg_multipart, encode_mjpeg_part, export_finite_h264_epochs,
    fake_semantic_hit, frame_decision_unavailable, frame_h264_picture, group_history_by_predicate,
    guard_against_description_evidence, h264_track_relation, interpret_structured_outcome,
    map_append_error, mjpeg_content_type, package_h264_epochs, parse_decision_evaluator_provider,
    reject_decision_evaluator_backend, reject_decision_provenance, reject_forbidden_decision_input,
    router, router_for_lab, semantic_search_unavailable, unsupported_predicate_result,
    validate_description_batch, validate_embedding_batch, validate_evaluation_batch,
    video_clip_path, video_live_path,
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
