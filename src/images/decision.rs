//! Generic JEV-style image decision contracts.
//!
//! Downstream repositories consume this contract from a published git revision
//! of `a2a-lab-dev-kit`, never a path dependency. The types are transport-neutral:
//! HTTP, MCP, and A2A carry the same JSON. This module does not add
//! settings pages, whole-video indexing or backfill, AgentCore, Strands, an AWS
//! SDK client, or a profile 1.3.0 bump.
//!
//! A generic decision is a question over one image. It is not a typed
//! `LabPredicate` evaluation. [`execute_generic_image_decision`] returns only
//! [`ImageDecisionResult`] and does not construct or append a `FrameDecisionRecord`.
//! The typed three-predicate live evaluator remains on its compatibility adapter.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{A2aLabError, ImageDecisionFailure};
use crate::images::Image;

/// How downstream repositories consume this contract.
pub const IMAGE_DECISION_DOWNSTREAM_DEPENDENCY: &str = "published-git-revision";

/// Lab transports that carry this contract.
pub const IMAGE_DECISION_TRANSPORTS: &[&str] = &["http", "mcp", "a2a"];

/// Work this contract does not include.
pub const IMAGE_DECISION_EXCLUDED_SCOPE: &[&str] = &[
    "settings-pages",
    "whole-video-indexing",
    "whole-video-backfill",
    "agentcore",
    "strands",
    "aws-sdk",
    "profile-1.3.0",
];

/// Maximum visible characters in one question.
pub const MAX_IMAGE_DECISION_QUESTION_CHARS: usize = 4_096;

/// Maximum characters in one model identifier.
pub const MAX_IMAGE_DECISION_MODEL_CHARS: usize = 128;

/// Maximum answers accepted from one model response.
pub const MAX_IMAGE_DECISION_ANSWERS: usize = 32;

/// Maximum visible characters in one answer text.
pub const MAX_IMAGE_DECISION_ANSWER_CHARS: usize = 2_048;

/// Maximum characters in one provider-error message.
pub const MAX_IMAGE_DECISION_ERROR_CHARS: usize = 512;

/// Maximum characters in one provider-error limit name.
pub const MAX_IMAGE_DECISION_LIMIT_CHARS: usize = 64;

/// Generic decisions are not typed lab-predicate evaluations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageDecisionClass {
    /// Question, image, and optional ROI. No typed ledger append.
    Generic,
}

/// Normalized box `[x_min, y_min, x_max, y_max]` on an image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionRoi {
    bounds: [f32; 4],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionRoiRaw {
    bounds: [f32; 4],
}

impl ImageDecisionRoi {
    /// Full source image (`[0, 0, 1, 1]`).
    #[must_use]
    pub const fn full_image() -> Self {
        Self {
            bounds: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Validates normalized bounds.
    pub fn new(bounds: [f32; 4]) -> Result<Self, A2aLabError> {
        let [x_min, y_min, x_max, y_max] = bounds;
        if !(0.0..=1.0).contains(&x_min)
            || !(0.0..=1.0).contains(&y_min)
            || !(0.0..=1.0).contains(&x_max)
            || !(0.0..=1.0).contains(&y_max)
            || x_min >= x_max
            || y_min >= y_max
        {
            return Err(A2aLabError::invalid(
                "roi",
                "bounds must be normalized with min strictly before max",
            ));
        }
        Ok(Self { bounds })
    }

    /// Normalized bounds.
    #[must_use]
    pub const fn bounds(self) -> [f32; 4] {
        self.bounds
    }
}

impl TryFrom<ImageDecisionRoiRaw> for ImageDecisionRoi {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionRoiRaw) -> Result<Self, Self::Error> {
        Self::new(raw.bounds)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionRoi {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionRoiRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Model identifier and optional crop sent to the decision model.
///
/// An omitted ROI is the full source image.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionModelInput {
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roi: Option<ImageDecisionRoi>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionModelInputRaw {
    model: String,
    #[serde(default)]
    roi: Option<ImageDecisionRoi>,
}

impl ImageDecisionModelInput {
    /// Validates the model identifier. `roi: None` means the full source image.
    pub fn new(model: impl AsRef<str>, roi: Option<ImageDecisionRoi>) -> Result<Self, A2aLabError> {
        Ok(Self {
            model: bounded_token("model", model.as_ref(), MAX_IMAGE_DECISION_MODEL_CHARS)?,
            roi,
        })
    }

    /// Model identifier supplied to the decision provider.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// ROI as supplied. `None` means the caller omitted it.
    #[must_use]
    pub const fn roi(&self) -> Option<ImageDecisionRoi> {
        self.roi
    }

    /// Crop applied before the model. Omission selects the full source image.
    #[must_use]
    pub fn effective_roi(&self) -> ImageDecisionRoi {
        self.roi.unwrap_or_else(ImageDecisionRoi::full_image)
    }

    /// Whether the caller omitted the ROI.
    #[must_use]
    pub const fn omits_roi(&self) -> bool {
        self.roi.is_none()
    }
}

impl TryFrom<ImageDecisionModelInputRaw> for ImageDecisionModelInput {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionModelInputRaw) -> Result<Self, Self::Error> {
        Self::new(raw.model, raw.roi)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionModelInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionModelInputRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// One generic image decision request.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionRequest {
    image: Image,
    question: String,
    model_input: ImageDecisionModelInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionRequestRaw {
    image: Image,
    question: String,
    model_input: ImageDecisionModelInput,
}

impl ImageDecisionRequest {
    /// Validates the required image, question, and model input.
    pub fn new(
        image: Image,
        question: impl AsRef<str>,
        model_input: ImageDecisionModelInput,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            image,
            question: bounded_text(
                "question",
                question.as_ref(),
                MAX_IMAGE_DECISION_QUESTION_CHARS,
            )?,
            model_input,
        })
    }

    /// Generic decisions are not typed lab-predicate evaluations.
    #[must_use]
    pub const fn class(&self) -> ImageDecisionClass {
        ImageDecisionClass::Generic
    }

    /// Image supplied to the decision.
    #[must_use]
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// Question or prompt.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }

    /// Model identifier and optional crop.
    #[must_use]
    pub const fn model_input(&self) -> &ImageDecisionModelInput {
        &self.model_input
    }

    pub(crate) fn into_image(self) -> Image {
        self.image
    }

    /// Crop processed for the model. An omitted ROI is the full source image.
    #[must_use]
    pub fn model_input_roi(&self) -> ImageDecisionRoi {
        self.model_input.effective_roi()
    }
}

impl TryFrom<ImageDecisionRequestRaw> for ImageDecisionRequest {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.image, raw.question, raw.model_input)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// One answer whose box, when present, is on the original source image.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probability: Option<f32>,
    /// Box normalized to the original source image after model-input cropping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_box: Option<ImageDecisionRoi>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionAnswerRaw {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    probability: Option<f32>,
    #[serde(default)]
    source_box: Option<ImageDecisionRoi>,
}

impl ImageDecisionAnswer {
    /// Validates one bounded answer in original-source coordinates.
    pub fn new(
        text: Option<&str>,
        probability: Option<f32>,
        source_box: Option<ImageDecisionRoi>,
    ) -> Result<Self, A2aLabError> {
        let parts = validate_answer_parts(text, probability, source_box)?;
        Ok(Self {
            text: parts.text,
            probability: parts.probability,
            source_box: parts.region,
        })
    }

    /// Bounded answer text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Unit-interval probability when the model supplied one.
    #[must_use]
    pub const fn probability(&self) -> Option<f32> {
        self.probability
    }

    /// Box on the original source image.
    #[must_use]
    pub const fn source_box(&self) -> Option<ImageDecisionRoi> {
        self.source_box
    }
}

impl TryFrom<ImageDecisionAnswerRaw> for ImageDecisionAnswer {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionAnswerRaw) -> Result<Self, Self::Error> {
        Self::new(raw.text.as_deref(), raw.probability, raw.source_box)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionAnswer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionAnswerRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// One model answer before box coordinates are mapped back to the source image.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionModelAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probability: Option<f32>,
    /// Box relative to the cropped model input. `[0, 0, 1, 1]` is the crop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_box: Option<ImageDecisionRoi>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionModelAnswerRaw {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    probability: Option<f32>,
    #[serde(default)]
    model_box: Option<ImageDecisionRoi>,
}

impl ImageDecisionModelAnswer {
    /// Validates one bounded model answer in crop coordinates.
    pub fn new(
        text: Option<&str>,
        probability: Option<f32>,
        model_box: Option<ImageDecisionRoi>,
    ) -> Result<Self, A2aLabError> {
        let parts = validate_answer_parts(text, probability, model_box)?;
        Ok(Self {
            text: parts.text,
            probability: parts.probability,
            model_box: parts.region,
        })
    }

    /// Bounded answer text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Unit-interval probability when the model supplied one.
    #[must_use]
    pub const fn probability(&self) -> Option<f32> {
        self.probability
    }

    /// Box relative to the model-input crop.
    #[must_use]
    pub const fn model_box(&self) -> Option<ImageDecisionRoi> {
        self.model_box
    }
}

impl TryFrom<ImageDecisionModelAnswerRaw> for ImageDecisionModelAnswer {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionModelAnswerRaw) -> Result<Self, Self::Error> {
        Self::new(raw.text.as_deref(), raw.probability, raw.model_box)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionModelAnswer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionModelAnswerRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Structured generic decision result. Boxes use original-source coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDecisionResult {
    answers: Vec<ImageDecisionAnswer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDecisionResultRaw {
    answers: Vec<ImageDecisionAnswer>,
}

impl ImageDecisionResult {
    /// Validates the bounded answer list.
    pub fn new(answers: Vec<ImageDecisionAnswer>) -> Result<Self, A2aLabError> {
        check_answer_count(answers.len())?;
        Ok(Self { answers })
    }

    /// Answers in original-source coordinates.
    #[must_use]
    pub fn answers(&self) -> &[ImageDecisionAnswer] {
        &self.answers
    }

    /// Source-image boxes returned with the answers.
    pub fn source_boxes(&self) -> impl Iterator<Item = ImageDecisionRoi> + '_ {
        self.answers
            .iter()
            .filter_map(ImageDecisionAnswer::source_box)
    }
}

impl TryFrom<ImageDecisionResultRaw> for ImageDecisionResult {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionResultRaw) -> Result<Self, Self::Error> {
        Self::new(raw.answers)
    }
}

impl<'de> Deserialize<'de> for ImageDecisionResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionResultRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Provider failure for a generic image decision.
///
/// Values are devkit errors. They do not carry transport status codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ImageDecisionProviderError {
    /// The decision provider could not serve the request.
    Unavailable {
        /// Provider detail.
        message: String,
    },
    /// The provider answer failed the bounded contract.
    InvalidAnswer {
        /// Provider detail.
        message: String,
    },
    /// A configured bound was exceeded.
    LimitExceeded {
        /// Which limit tripped.
        limit: String,
        /// Provider detail.
        message: String,
    },
    /// The provider deadline elapsed.
    DeadlineExceeded {
        /// Provider detail.
        message: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
enum ImageDecisionProviderErrorRaw {
    Unavailable { message: String },
    InvalidAnswer { message: String },
    LimitExceeded { limit: String, message: String },
    DeadlineExceeded { message: String },
}

impl ImageDecisionProviderError {
    /// Validates an unavailable-provider error.
    pub fn unavailable(message: impl AsRef<str>) -> Result<Self, A2aLabError> {
        Ok(Self::Unavailable {
            message: bounded_text("message", message.as_ref(), MAX_IMAGE_DECISION_ERROR_CHARS)?,
        })
    }

    /// Validates an invalid-answer error.
    pub fn invalid_answer(message: impl AsRef<str>) -> Result<Self, A2aLabError> {
        Ok(Self::InvalidAnswer {
            message: bounded_text("message", message.as_ref(), MAX_IMAGE_DECISION_ERROR_CHARS)?,
        })
    }

    /// Validates a limit error.
    pub fn limit_exceeded(
        limit: impl AsRef<str>,
        message: impl AsRef<str>,
    ) -> Result<Self, A2aLabError> {
        Ok(Self::LimitExceeded {
            limit: bounded_token("limit", limit.as_ref(), MAX_IMAGE_DECISION_LIMIT_CHARS)?,
            message: bounded_text("message", message.as_ref(), MAX_IMAGE_DECISION_ERROR_CHARS)?,
        })
    }

    /// Validates a deadline error.
    pub fn deadline_exceeded(message: impl AsRef<str>) -> Result<Self, A2aLabError> {
        Ok(Self::DeadlineExceeded {
            message: bounded_text("message", message.as_ref(), MAX_IMAGE_DECISION_ERROR_CHARS)?,
        })
    }

    /// Maps a provider failure onto the lab error shared by HTTP, MCP, and A2A.
    #[must_use]
    pub fn into_lab_error(self) -> A2aLabError {
        match self {
            Self::Unavailable { message } => A2aLabError::unavailable(message),
            Self::InvalidAnswer { message } => {
                A2aLabError::image_decision(ImageDecisionFailure::InvalidAnswer, message)
            }
            Self::LimitExceeded { limit, message } => A2aLabError::image_decision(
                ImageDecisionFailure::LimitExceeded,
                format!("{limit}: {message}"),
            ),
            Self::DeadlineExceeded { message } => {
                A2aLabError::image_decision(ImageDecisionFailure::DeadlineExceeded, message)
            }
        }
    }

    /// Stable machine-readable reason.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::InvalidAnswer { .. } => "invalid_answer",
            Self::LimitExceeded { .. } => "limit_exceeded",
            Self::DeadlineExceeded { .. } => "deadline_exceeded",
        }
    }
}

impl TryFrom<ImageDecisionProviderErrorRaw> for ImageDecisionProviderError {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionProviderErrorRaw) -> Result<Self, Self::Error> {
        match raw {
            ImageDecisionProviderErrorRaw::Unavailable { message } => Self::unavailable(message),
            ImageDecisionProviderErrorRaw::InvalidAnswer { message } => {
                Self::invalid_answer(message)
            }
            ImageDecisionProviderErrorRaw::LimitExceeded { limit, message } => {
                Self::limit_exceeded(limit, message)
            }
            ImageDecisionProviderErrorRaw::DeadlineExceeded { message } => {
                Self::deadline_exceeded(message)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ImageDecisionProviderError {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionProviderErrorRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl std::fmt::Display for ImageDecisionProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { message } | Self::InvalidAnswer { message } => {
                write!(formatter, "{}: {message}", self.code())
            }
            Self::DeadlineExceeded { message } => {
                write!(formatter, "deadline_exceeded: {message}")
            }
            Self::LimitExceeded { limit, message } => {
                write!(formatter, "limit_exceeded ({limit}): {message}")
            }
        }
    }
}

impl std::error::Error for ImageDecisionProviderError {}

/// One model answer whose box is relative to the crop and may extend past it.
///
/// Source-image answers still use [`ImageDecisionAnswer`]. This type is the
/// model-side input to resolution. It is not a typed ledger record.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDecisionUnmappedAnswer {
    text: Option<String>,
    probability: Option<f32>,
    model_bounds: Option<[f32; 4]>,
}

impl ImageDecisionUnmappedAnswer {
    /// Validates text, probability, and finite ordered crop-relative bounds.
    ///
    /// Bounds may lie outside `[0, 1]`. Mapping clamps them onto the source image.
    pub fn new(
        text: Option<&str>,
        probability: Option<f32>,
        model_bounds: Option<[f32; 4]>,
    ) -> Result<Self, A2aLabError> {
        let text = text
            .map(|value| bounded_text("text", value, MAX_IMAGE_DECISION_ANSWER_CHARS))
            .transpose()?;
        if let Some(value) = probability {
            validate_probability(value)?;
        }
        if let Some(bounds) = model_bounds {
            validate_model_bounds(bounds)?;
        }
        if text.is_none() && probability.is_none() && model_bounds.is_none() {
            return Err(A2aLabError::invalid(
                "answer",
                "requires text, probability, or a box",
            ));
        }
        Ok(Self {
            text,
            probability,
            model_bounds,
        })
    }

    /// Bounded answer text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Unit-interval probability when the model supplied one.
    #[must_use]
    pub const fn probability(&self) -> Option<f32> {
        self.probability
    }

    /// Crop-relative bounds. Values may lie outside the crop.
    #[must_use]
    pub const fn model_bounds(&self) -> Option<[f32; 4]> {
        self.model_bounds
    }
}

/// Maps a crop-relative model box onto the original source image.
///
/// `source_roi` is the crop sent to the model. An omitted request ROI uses
/// [`ImageDecisionRoi::full_image`]. `model_box` is normalized inside that crop.
pub fn normalize_model_box_to_source(
    source_roi: ImageDecisionRoi,
    model_box: ImageDecisionRoi,
) -> Result<ImageDecisionRoi, A2aLabError> {
    map_model_bounds_onto_source(1, 1, source_roi, model_box.bounds())
}

/// Maps crop-relative bounds onto the original image using its pixel dimensions.
///
/// Bounds may extend past the crop. The result is clamped into the normalized
/// source-image space. A box that falls entirely outside that space is rejected.
pub fn map_model_bounds_onto_source(
    source_width: u32,
    source_height: u32,
    source_roi: ImageDecisionRoi,
    model_bounds: [f32; 4],
) -> Result<ImageDecisionRoi, A2aLabError> {
    validate_model_bounds(model_bounds)?;
    let width = exact_axis_len(source_width)?;
    let height = exact_axis_len(source_height)?;
    let [x0, y0, x1, y1] = source_roi.bounds();
    let [mx0, my0, mx1, my1] = model_bounds;
    let mapped = [
        map_edge(x0, x1, mx0, width)?,
        map_edge(y0, y1, my0, height)?,
        map_edge(x0, x1, mx1, width)?,
        map_edge(y0, y1, my1, height)?,
    ];
    ImageDecisionRoi::new(mapped)
        .map_err(|_| A2aLabError::invalid("model_box", "mapped box is outside the source image"))
}

pub(crate) fn exact_axis_len(size: u32) -> Result<f32, A2aLabError> {
    if size == 0 {
        return Err(A2aLabError::invalid("image", "dimensions must be positive"));
    }
    // Every integer up to 2^24 is exactly representable as f32.
    if size > 1 << 24 {
        return Err(A2aLabError::invalid(
            "image",
            "dimensions are too large to map coordinates exactly",
        ));
    }
    #[allow(clippy::cast_precision_loss)]
    Ok(size as f32)
}

/// Executes one synchronous generic image decision.
///
/// HTTP, MCP, and A2A call this through the service. Implementations return
/// [`ImageDecisionResult`] and do not append a typed frame-decision record.
#[async_trait]
pub trait ImageDecisionExecutor: Send + Sync {
    /// Completes one validated generic request.
    async fn decide(
        &self,
        request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError>;
}

/// Completes a generic image decision from model answers.
///
/// Returned boxes are normalized to the original source image. This function
/// does not construct a `FrameDecisionRecord` and does not append to a typed
/// frame-decision ledger.
pub fn execute_generic_image_decision(
    request: &ImageDecisionRequest,
    model_answers: &[ImageDecisionModelAnswer],
) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
    if model_answers.len() > MAX_IMAGE_DECISION_ANSWERS {
        return Err(answer_limit_exceeded());
    }
    let crop = request.model_input_roi();
    let width = request.image().descriptor().width();
    let height = request.image().descriptor().height();
    let mut answers = Vec::with_capacity(model_answers.len());
    for model_answer in model_answers {
        let source_box = model_answer
            .model_box()
            .map(|model_box| {
                map_model_bounds_onto_source(width, height, crop, model_box.bounds()).map_err(
                    |error| ImageDecisionProviderError::InvalidAnswer {
                        message: error.to_string(),
                    },
                )
            })
            .transpose()?;
        answers.push(ImageDecisionAnswer {
            text: model_answer.text().map(str::to_owned),
            probability: model_answer.probability(),
            source_box,
        });
    }
    Ok(ImageDecisionResult { answers })
}

struct AnswerParts {
    text: Option<String>,
    probability: Option<f32>,
    region: Option<ImageDecisionRoi>,
}

fn validate_answer_parts(
    text: Option<&str>,
    probability: Option<f32>,
    region: Option<ImageDecisionRoi>,
) -> Result<AnswerParts, A2aLabError> {
    let text = text
        .map(|value| bounded_text("text", value, MAX_IMAGE_DECISION_ANSWER_CHARS))
        .transpose()?;
    if let Some(value) = probability {
        validate_probability(value)?;
    }
    if text.is_none() && probability.is_none() && region.is_none() {
        return Err(A2aLabError::invalid(
            "answer",
            "requires text, probability, or a box",
        ));
    }
    Ok(AnswerParts {
        text,
        probability,
        region,
    })
}

fn validate_probability(value: f32) -> Result<(), A2aLabError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(A2aLabError::invalid(
            "probability",
            "must be a finite value in 0.0..=1.0",
        ));
    }
    Ok(())
}

fn answer_limit_exceeded() -> ImageDecisionProviderError {
    ImageDecisionProviderError::LimitExceeded {
        limit: "answers".to_owned(),
        message: format!("must contain at most {MAX_IMAGE_DECISION_ANSWERS} answers"),
    }
}

pub(crate) fn check_answer_count(count: usize) -> Result<(), A2aLabError> {
    if count > MAX_IMAGE_DECISION_ANSWERS {
        return Err(A2aLabError::invalid(
            "answers",
            format!("must contain at most {MAX_IMAGE_DECISION_ANSWERS} answers"),
        ));
    }
    Ok(())
}

fn validate_model_bounds(bounds: [f32; 4]) -> Result<(), A2aLabError> {
    let [x_min, y_min, x_max, y_max] = bounds;
    if !x_min.is_finite()
        || !y_min.is_finite()
        || !x_max.is_finite()
        || !y_max.is_finite()
        || x_min >= x_max
        || y_min >= y_max
    {
        return Err(A2aLabError::invalid(
            "model_box",
            "bounds must be finite with min strictly before max",
        ));
    }
    Ok(())
}

fn map_edge(start: f32, end: f32, model: f32, size: f32) -> Result<f32, A2aLabError> {
    let px = (start * size) + (model * ((end - start) * size));
    if !px.is_finite() {
        return Err(A2aLabError::invalid(
            "model_box",
            "bounds must be finite with min strictly before max",
        ));
    }
    let normalized = px.clamp(0.0, size) / size;
    if !normalized.is_finite() || !(0.0..=1.0).contains(&normalized) {
        return Err(A2aLabError::invalid(
            "model_box",
            "mapped box is outside the source image",
        ));
    }
    Ok(normalized)
}

fn bounded_text(field: &'static str, value: &str, max_chars: usize) -> Result<String, A2aLabError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max_chars {
        return Err(A2aLabError::invalid(
            field,
            format!("must be 1..={max_chars} visible characters"),
        ));
    }
    Ok(trimmed.to_owned())
}

fn bounded_token(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<String, A2aLabError> {
    let token = bounded_text(field, value, max_chars)?;
    if token.chars().any(char::is_whitespace) {
        return Err(A2aLabError::invalid(field, "must not contain whitespace"));
    }
    Ok(token)
}
