//! Image sources, searchable descriptors, inline image payloads, generic image
//! decision contracts, one-image resolution, a synchronous JEV-style decision
//! HTTP client, and a TypeSafe System One HTTP client.
//!
//! This crate does not capture frames and does not include a built-in image adapter.

mod decision;
mod decision_http;
mod model;
mod preamble;
mod provider;
mod resolve;
mod source_decision;
mod typesafe_system_one;

pub use decision::{
    IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, IMAGE_DECISION_EXCLUDED_SCOPE, IMAGE_DECISION_TRANSPORTS,
    ImageDecisionAnswer, ImageDecisionClass, ImageDecisionExecutor, ImageDecisionModelAnswer,
    ImageDecisionModelInput, ImageDecisionProviderError, ImageDecisionRequest, ImageDecisionResult,
    ImageDecisionRoi, ImageDecisionUnmappedAnswer, MAX_IMAGE_DECISION_ANSWER_CHARS,
    MAX_IMAGE_DECISION_ANSWERS, MAX_IMAGE_DECISION_ERROR_CHARS, MAX_IMAGE_DECISION_LIMIT_CHARS,
    MAX_IMAGE_DECISION_MODEL_CHARS, MAX_IMAGE_DECISION_QUESTION_CHARS,
    execute_generic_image_decision, map_model_bounds_onto_source, normalize_model_box_to_source,
};
pub use decision_http::{
    IMAGE_DECISION_HTTP_DEADLINE, IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES,
    IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES, IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES,
    ImageDecisionHttpClient, ImageDecisionHttpLimits, ModelPinnedImageDecision,
};
pub use model::{
    DEFAULT_MAX_IMAGE_BYTES, GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor,
    ImageSource, ImageTransportConfig, ListImageSourcesRequest, ListImagesRequest,
    SearchImagesRequest, TckMalformedImageRequests,
};
pub use provider::ImageProvider;
pub use resolve::{ImageDecisionImageRef, ImageDecisionModelCrop, ResolvedImageDecision};
pub(crate) use resolve::{
    complete_resolved_image_decision, resolve_captured_image, resolve_one_source_image,
};
pub use source_decision::{
    ImageDecisionPrimitiveAnswer, ImageDecisionQuestion, ImageDecisionQuestions, NoulCriteria,
    SourceImageDecisionRequest, SourceImageDecisionResult,
};
pub(crate) use typesafe_system_one::PinnedTypeSafeDecision;
pub use typesafe_system_one::TypeSafeSystemOneClient;
