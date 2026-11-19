//! Resolve exactly one source image for a generic decision.
//!
//! Callers name an image through [`ImageProvider`]. This module does not append
//! typed frame-decision records, and it does not add settings pages, whole-video
//! indexing or backfill, AgentCore, Strands, an AWS SDK client, or profile 1.3.0.
//! Downstream labs consume the devkit from a published git revision.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId};
use crate::images::decision::{
    ImageDecisionAnswer, ImageDecisionModelInput, ImageDecisionRequest, ImageDecisionResult,
    ImageDecisionRoi, ImageDecisionUnmappedAnswer, check_answer_count, exact_axis_len,
    map_model_bounds_onto_source,
};
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageProvider,
    ImageTransportConfig, SearchImagesRequest,
};
use crate::page::{Page, PageRequest};
use crate::time::TimeRange;

const RESOLUTION_PAGE_LIMIT: u32 = 2;

/// Names exactly one source image without embedding its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "select", rename_all = "snake_case")]
pub enum ImageDecisionImageRef {
    /// One stored image.
    Id {
        /// Image identifier.
        id: ImageId,
    },
    /// The provider's current frame for one source.
    Current {
        /// Source whose current frame should be read.
        source_id: ImageSourceId,
    },
    /// A search that must match one image.
    Query {
        /// Optional source filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_id: Option<ImageSourceId>,
        /// Optional half-open capture window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<TimeRange>,
        /// Optional nonblank caption query.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "select", rename_all = "snake_case", deny_unknown_fields)]
enum ImageDecisionImageRefRaw {
    Id {
        id: ImageId,
    },
    Current {
        source_id: ImageSourceId,
    },
    Query {
        #[serde(default)]
        source_id: Option<ImageSourceId>,
        #[serde(default)]
        range: Option<TimeRange>,
        #[serde(default)]
        text: Option<String>,
    },
}

impl ImageDecisionImageRef {
    /// Selects one stored image.
    #[must_use]
    pub const fn by_id(id: ImageId) -> Self {
        Self::Id { id }
    }

    /// Selects the current frame of one source.
    #[must_use]
    pub const fn current(source_id: ImageSourceId) -> Self {
        Self::Current { source_id }
    }

    /// Selects a search that must resolve to one image.
    pub fn query(
        source_id: Option<ImageSourceId>,
        range: Option<TimeRange>,
        text: Option<String>,
    ) -> Result<Self, A2aLabError> {
        let text = match text {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(A2aLabError::invalid("text", "must be nonblank"));
                }
                Some(trimmed.to_owned())
            }
            None => None,
        };
        if let Some(range) = range {
            range.check()?;
        }
        if range.is_none() && text.is_none() {
            return Err(A2aLabError::invalid(
                "search",
                "requires a time range or nonblank text",
            ));
        }
        Ok(Self::Query {
            source_id,
            range,
            text,
        })
    }
}

impl TryFrom<ImageDecisionImageRefRaw> for ImageDecisionImageRef {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionImageRefRaw) -> Result<Self, Self::Error> {
        match raw {
            ImageDecisionImageRefRaw::Id { id } => Ok(Self::by_id(id)),
            ImageDecisionImageRefRaw::Current { source_id } => Ok(Self::current(source_id)),
            ImageDecisionImageRefRaw::Query {
                source_id,
                range,
                text,
            } => Self::query(source_id, range, text),
        }
    }
}

impl<'de> Deserialize<'de> for ImageDecisionImageRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionImageRefRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Pixel window sent to the model. The stored source image stays uncropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDecisionModelCrop {
    roi: ImageDecisionRoi,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl ImageDecisionModelCrop {
    pub(crate) fn for_image(
        image: &Image,
        roi: Option<ImageDecisionRoi>,
    ) -> Result<Self, A2aLabError> {
        let roi = roi.unwrap_or_else(ImageDecisionRoi::full_image);
        let [x0, y0, x1, y1] = roi.bounds();
        let (x, width) = pixel_span(x0, x1, image.descriptor().width())?;
        let (y, height) = pixel_span(y0, y1, image.descriptor().height())?;
        Ok(Self {
            roi,
            x,
            y,
            width,
            height,
        })
    }

    /// Effective ROI. Omission is the full source image.
    #[must_use]
    pub const fn roi(self) -> ImageDecisionRoi {
        self.roi
    }

    /// Left edge of the model-input window, in source pixels.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Top edge of the model-input window, in source pixels.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Width of the model-input window, in source pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Height of the model-input window, in source pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// One resolved generic decision. Boxes use original-source coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedImageDecision {
    source: Image,
    omitted_roi: bool,
    model_crop: ImageDecisionModelCrop,
    result: ImageDecisionResult,
}

impl ResolvedImageDecision {
    /// Original source image. Its bytes are not replaced by the model crop.
    #[must_use]
    pub const fn source(&self) -> &Image {
        &self.source
    }

    /// Whether the caller omitted the ROI.
    #[must_use]
    pub const fn omits_roi(&self) -> bool {
        self.omitted_roi
    }

    /// Crop used only as model input.
    #[must_use]
    pub const fn model_crop(&self) -> ImageDecisionModelCrop {
        self.model_crop
    }

    /// Answers in original-source coordinates.
    #[must_use]
    pub const fn result(&self) -> &ImageDecisionResult {
        &self.result
    }
}

pub(crate) async fn resolve_one_source_image(
    provider: &impl ImageProvider,
    transport: ImageTransportConfig,
    image_ref: &ImageDecisionImageRef,
) -> Result<Image, A2aLabError> {
    match image_ref {
        ImageDecisionImageRef::Id { id } => resolve_id(provider, transport, id).await,
        ImageDecisionImageRef::Current { source_id } => {
            resolve_current(provider, transport, source_id).await
        }
        ImageDecisionImageRef::Query {
            source_id,
            range,
            text,
        } => {
            resolve_query(
                provider,
                transport,
                source_id.as_ref(),
                *range,
                text.as_deref(),
            )
            .await
        }
    }
}

pub(crate) fn complete_resolved_image_decision(
    source: Image,
    question: &str,
    model_input: ImageDecisionModelInput,
    model_answers: &[ImageDecisionUnmappedAnswer],
) -> Result<ResolvedImageDecision, A2aLabError> {
    let request = ImageDecisionRequest::new(source, question, model_input)?;
    let omitted_roi = request.model_input().omits_roi();
    let model_crop =
        ImageDecisionModelCrop::for_image(request.image(), request.model_input().roi())?;
    let result = map_answers(request.image(), model_crop.roi(), model_answers)?;
    Ok(ResolvedImageDecision {
        source: request.into_image(),
        omitted_roi,
        model_crop,
        result,
    })
}

async fn resolve_id(
    provider: &impl ImageProvider,
    transport: ImageTransportConfig,
    id: &ImageId,
) -> Result<Image, A2aLabError> {
    let image = provider.get_image(GetImageRequest::new(id.clone())).await?;
    accept_image(transport, &image, Some(id), None)
}

async fn resolve_current(
    provider: &impl ImageProvider,
    transport: ImageTransportConfig,
    source_id: &ImageSourceId,
) -> Result<Image, A2aLabError> {
    let image = provider
        .get_current_image(GetCurrentImageRequest::new(source_id.clone()))
        .await?;
    accept_image(transport, &image, None, Some(source_id))
}

async fn resolve_query(
    provider: &impl ImageProvider,
    transport: ImageTransportConfig,
    source_id: Option<&ImageSourceId>,
    range: Option<TimeRange>,
    text: Option<&str>,
) -> Result<Image, A2aLabError> {
    let page = PageRequest::new(None, RESOLUTION_PAGE_LIMIT)?;
    let request =
        SearchImagesRequest::new(source_id.cloned(), range, text.map(str::to_owned), page)?;
    let found = exactly_one(&provider.search_images(request).await?)?;
    let image = provider
        .get_image(GetImageRequest::new(found.id().clone()))
        .await?;
    accept_image(transport, &image, Some(found.id()), Some(found.source_id()))
}

fn exactly_one(page: &Page<ImageDescriptor>) -> Result<ImageDescriptor, A2aLabError> {
    let items = page.items();
    if items.len() == 1 && page.next_cursor().is_none() {
        return Ok(items[0].clone());
    }
    if items.is_empty() && page.next_cursor().is_none() {
        return Err(A2aLabError::not_found("image", "query"));
    }
    Err(A2aLabError::invalid("image", "ambiguous image reference"))
}

fn accept_image(
    transport: ImageTransportConfig,
    image: &Image,
    id: Option<&ImageId>,
    source_id: Option<&ImageSourceId>,
) -> Result<Image, A2aLabError> {
    let descriptor = image.descriptor();
    if id.is_some_and(|expected| descriptor.id() != expected)
        || source_id.is_some_and(|expected| descriptor.source_id() != expected)
    {
        return Err(A2aLabError::invalid(
            "image",
            "resolved image does not match the requested reference",
        ));
    }
    Image::with_transport(descriptor.clone(), image.data().to_vec(), transport)
}

fn map_answers(
    source: &Image,
    crop: ImageDecisionRoi,
    model_answers: &[ImageDecisionUnmappedAnswer],
) -> Result<ImageDecisionResult, A2aLabError> {
    check_answer_count(model_answers.len())?;
    let width = source.descriptor().width();
    let height = source.descriptor().height();
    let mut answers = Vec::with_capacity(model_answers.len());
    for model_answer in model_answers {
        let source_box = model_answer
            .model_bounds()
            .map(|bounds| map_model_bounds_onto_source(width, height, crop, bounds))
            .transpose()?;
        answers.push(ImageDecisionAnswer::new(
            model_answer.text(),
            model_answer.probability(),
            source_box,
        )?);
    }
    ImageDecisionResult::new(answers)
}

fn pixel_span(start: f32, end: f32, size: u32) -> Result<(u32, u32), A2aLabError> {
    let size_f = exact_axis_len(size)?;
    let start_px = (start * size_f).floor().clamp(0.0, size_f);
    let end_px = (end * size_f).ceil().clamp(start_px, size_f);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let origin = start_px as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let limit = end_px as u32;
    let span = limit.checked_sub(origin).filter(|span| *span > 0);
    span.map(|span| (origin, span))
        .ok_or_else(|| A2aLabError::invalid("roi", "crop is empty in source pixels"))
}
