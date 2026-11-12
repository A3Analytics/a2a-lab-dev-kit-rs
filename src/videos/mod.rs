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
mod digest;
mod dvr_store;
mod epoch_hls;
mod frame_decision;
mod frame_select;
#[cfg(feature = "openh264")]
mod h264_encode;
mod h264_sample;
pub(crate) mod http;
mod index_provenance;
#[cfg(feature = "yuv")]
mod jpeg_derivative;
mod local_media;
mod media_asset;
#[cfg(feature = "yuv")]
mod mjpeg_decode;
mod model;
mod model_space;
#[cfg(feature = "nokhwa")]
mod nokhwa_device;
mod packager;
mod provider;
mod saved_frame;
mod semantic_indexing;
mod semantic_search;
mod timeline;

pub use capture::{
    CaptureCapabilities, CaptureClock, CaptureFormat, CaptureIdentity, CaptureMode, CaptureTiming,
    CapturedColorFrame, ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange,
    MemoryColorCapture,
};
pub use capture_profile::{
    AdvertisedCaptureProfile, CaptureFormatPreference, CaptureProfileError, CaptureProfileRequest,
    CaptureRate, DEFAULT_CAPTURE_OUTPUT_FPS, SelectedCaptureProfile, confirm_selected_profile,
    describe_capture_profile_error, format_advertised_capture_profiles,
    format_capture_profile_request, select_capture_profile, select_capture_profile_flexible,
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
pub use digest::Sha256Digest;
pub use dvr_store::{
    DvrDiscontinuity, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPayload,
    DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrPictureProperties, DvrRetentionBounds,
    H264TrackRelation, ReadDvrMediaWindowRequest, h264_track_relation,
};
pub use epoch_hls::{
    EpochHlsInit, EpochHlsSegment, EpochHlsSession, EpochPlaylistKind, FiniteH264Clip,
    export_finite_h264_epochs, package_h264_epochs,
};
pub use frame_decision::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, DEFAULT_FRAME_DECISION_HISTORY_ROWS,
    DEFAULT_JEV_MODEL_REVISION, DecisionEvaluationCoverage, DecisionEvidenceAvailability,
    DecisionEvidenceItem, DecisionEvidenceKind, DecisionOutcome, DecisionSubject,
    DecisionThresholdPolicy, DecisionUnknownReason, FakeFrameDecisionEvaluator,
    FakeFrameDecisionLedgerProvider, FakeJevBehavior, ForbiddenDecisionInput, ForeignObjectSubject,
    FrameDecisionAppendError, FrameDecisionCapabilities, FrameDecisionClientRole,
    FrameDecisionEvaluationFailure, FrameDecisionEvaluationOwnership,
    FrameDecisionEvaluationRequest, FrameDecisionEvaluationRouter, FrameDecisionEvaluator,
    FrameDecisionEvaluatorCapabilities, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionHistoryEntry, FrameDecisionHistoryResult,
    FrameDecisionLedger, FrameDecisionLedgerProvider, FrameDecisionOperation,
    FrameDecisionReadLimits, FrameDecisionReadScope, FrameDecisionRecord,
    FrameDecisionRoutingAudit, FrameDecisionSchemaRefs, GetFrameDecisionCapabilitiesRequest,
    JEV_EVALUATOR_PROVIDER_ID, JEV_MAX_EDGE_PIXELS, JEV_MAX_IMAGE_BYTES, JEV_MAX_REQUEST_BYTES,
    JEV_MIN_EDGE_PIXELS, JEV_PRODUCER_ID, JevEvaluationProvenance, JevProducerIdentity,
    JevStructuredOutput, KeyedFrameDecisionEvaluation, LabPredicate, LabProbabilityVector,
    MAX_FRAME_DECISION_HISTORY_ROWS, OperatorDecisionCriteria, OperatorReferenceImage,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    PipetteChannelSubject, PlateAlignmentReference, PlateSlotSubject,
    ReadFrameDecisionHistoryRequest, RegisteredFrameDecisionEvaluator,
    RejectedDecisionEvaluatorBackend, RejectedDecisionProvenance,
    TckMalformedFrameDecisionRequests, decision_history_fingerprint, default_pinned_jev_revision,
    derive_decision_id, frame_decision_unavailable, group_history_by_predicate,
    guard_against_description_evidence, interpret_structured_outcome, map_append_error,
    parse_decision_evaluator_provider, reject_decision_evaluator_backend,
    reject_decision_provenance, reject_forbidden_decision_input, unsupported_predicate_result,
    validate_evaluation_batch,
};
pub use frame_select::{FrameRejection, FreshFrameGate, OUTPUT_5FPS_PERIOD_NS, RejectedFrame};
#[cfg(feature = "openh264")]
pub use h264_encode::{
    AllIdrEncoder, EncodedAccessUnit, I420Picture, OPENH264_NOTICE, decode_luma,
};
pub use h264_sample::{H264PictureSample, frame_h264_picture};
pub use http::{apply_media_security_headers, is_gated_media_path, router, router_for_lab};
pub use index_provenance::{
    FrameDescriptionProvenance, FrameEmbeddingVector, FrameRoiTransform,
    FrameVectorIndexAttachment, TextEmbeddingProvenance,
};
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
pub use model_space::{
    EmbeddingMetric, ModelSpaceIdentity, ProvenanceGeneration, VectorNormalization,
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
pub use saved_frame::{SavedFrameRef, derive_frame_id};
pub use semantic_indexing::{
    DEFAULT_BEDROCK_DESCRIPTION_MODEL, DEFAULT_OPENAI_DESCRIPTION_MODEL, FakeDescriptionBehavior,
    FakeEmbeddingBehavior, FakeFrameDescriptionProvider, FakeTextEmbeddingProvider,
    FrameDescriptionBackend, FrameDescriptionKey, FrameDescriptionLimits, FrameDescriptionProvider,
    FrameDescriptionProviderConfig, FrameDescriptionRequest, KeyedFrameDescription,
    KeyedTextEmbedding, SemanticIndexingFailure, TextEmbeddingKey, TextEmbeddingLimits,
    TextEmbeddingProvider, TextEmbeddingProviderConfig, TextEmbeddingRequest,
    validate_description_batch, validate_embedding_batch,
};
pub use semantic_search::{
    DEFAULT_SEMANTIC_CANDIDATES, DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS, DEFAULT_SEMANTIC_TOP_K,
    DecisionHistoryPolicy, DecisionSearchFilter, FakeSemanticHit, FakeSemanticVideoSearchProvider,
    GetSemanticIndexingCoverageRequest, GetSemanticSearchCapabilitiesRequest,
    MAX_SEMANTIC_CANDIDATES, MAX_SEMANTIC_HIT_ASSESSMENT_ROWS, MAX_SEMANTIC_NEIGHBOR_FRAMES,
    MAX_SEMANTIC_TOP_K, SearchSemanticFramesRequest, SemanticFrameHit, SemanticFrameHitAssessments,
    SemanticFrameIndexState, SemanticFrameMediaAvailability, SemanticFramePlaybackNeighbors,
    SemanticFrameSearchResult, SemanticIndexingCoverage, SemanticIndexingFreshness,
    SemanticIndexingStateCounts, SemanticProvenanceSelector, SemanticRetrievalScore,
    SemanticSearchCapabilities, SemanticSearchLimits, SemanticSearchOperation, SemanticSearchQuery,
    SemanticSearchSchemaRefs, SemanticSearchScope, SemanticVideoSearchProvider,
    TckMalformedSemanticSearchRequests, authorized_assessments_for_frame, fake_semantic_hit,
    semantic_search_unavailable,
};
pub use timeline::{
    FollowVideoRequest, GetVideoTimelineRequest, VIDEO_LIVE_PATH_PREFIX, VideoFollow,
    VideoLiveStream, VideoTimeline, video_live_path,
};
