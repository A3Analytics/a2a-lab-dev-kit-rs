//! Image catalog, payload, and request types.

use base64::Engine;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId};
use crate::json_object::JsonObject;
use crate::page::PageRequest;
use crate::time::{TimeRange, UtcTimestamp};

/// Default decoded-byte ceiling for one inline image (64 MiB).
pub const DEFAULT_MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// A source that can supply images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImageSource {
    /// Stable source identifier.
    pub id: ImageSourceId,
    /// Human-readable name.
    pub name: String,
    /// What this source captures.
    pub description: String,
    /// Asset identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Semantic identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
}

/// Metadata for one captured frame. Pixel bytes are not included.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageDescriptor {
    id: ImageId,
    source_id: ImageSourceId,
    captured_at: UtcTimestamp,
    media_type: String,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    caption: Option<String>,
    #[serde(default = "empty_attributes")]
    attributes: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageDescriptorRaw {
    id: ImageId,
    source_id: ImageSourceId,
    captured_at: UtcTimestamp,
    media_type: String,
    width: u32,
    height: u32,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default = "empty_attributes")]
    attributes: JsonObject,
}

impl TryFrom<ImageDescriptorRaw> for ImageDescriptor {
    type Error = A2aLabError;

    fn try_from(raw: ImageDescriptorRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.id,
            raw.source_id,
            raw.captured_at,
            raw.media_type,
            raw.width,
            raw.height,
            raw.caption,
            raw.attributes,
        )
    }
}

impl<'de> Deserialize<'de> for ImageDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDescriptorRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl ImageDescriptor {
    /// Creates metadata when the media type and dimensions describe an image.
    pub fn new(
        id: ImageId,
        source_id: ImageSourceId,
        captured_at: UtcTimestamp,
        media_type: impl Into<String>,
        width: u32,
        height: u32,
        caption: Option<String>,
        attributes: JsonObject,
    ) -> Result<Self, A2aLabError> {
        let media_type = media_type.into();
        check_media_type(&media_type)?;
        check_dimension("width", width)?;
        check_dimension("height", height)?;
        Ok(Self {
            id,
            source_id,
            captured_at,
            media_type,
            width,
            height,
            caption,
            attributes,
        })
    }

    /// Stable image identifier.
    #[must_use]
    pub const fn id(&self) -> &ImageId {
        &self.id
    }

    /// Source that captured the frame.
    #[must_use]
    pub const fn source_id(&self) -> &ImageSourceId {
        &self.source_id
    }

    /// When the frame was captured.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// `image/*` media type of the payload.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Optional caption used by text search.
    #[must_use]
    pub fn caption(&self) -> Option<&str> {
        self.caption.as_deref()
    }

    /// Structured attributes. These do not carry pixel bytes.
    #[must_use]
    pub const fn attributes(&self) -> &JsonObject {
        &self.attributes
    }
}

/// Decoded-byte limit applied to inline image payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageTransportConfig {
    max_image_bytes: u64,
}

impl ImageTransportConfig {
    /// Rejects zero and limits whose allocation or base64 expansion would overflow.
    pub fn new(max_image_bytes: u64) -> Result<Self, A2aLabError> {
        if max_image_bytes == 0 {
            return Err(A2aLabError::invalid(
                "max_image_bytes",
                "must be greater than zero",
            ));
        }
        if !limit_fits(max_image_bytes) {
            return Err(A2aLabError::invalid("max_image_bytes", "is overflow-prone"));
        }
        Ok(Self { max_image_bytes })
    }

    /// Returns the decoded-byte maximum.
    #[must_use]
    pub const fn max_image_bytes(self) -> u64 {
        self.max_image_bytes
    }

    /// Accepts a decoded payload at or below the configured maximum.
    pub fn check_payload(self, payload: &[u8]) -> Result<(), A2aLabError> {
        let len = u64::try_from(payload.len()).map_err(|_| {
            A2aLabError::invalid("data", "decoded image exceeds the configured maximum")
        })?;
        if len > self.max_image_bytes {
            let maximum = self.max_image_bytes;
            return Err(A2aLabError::invalid(
                "data",
                format!("decoded image is {len} bytes; maximum is {maximum}"),
            ));
        }
        Ok(())
    }

    pub(crate) const fn from_default() -> Self {
        Self {
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
        }
    }
}

impl Default for ImageTransportConfig {
    fn default() -> Self {
        Self::from_default()
    }
}

fn limit_fits(max_image_bytes: u64) -> bool {
    if isize::try_from(max_image_bytes).is_err() {
        return false;
    }
    base64_encoded_len(max_image_bytes).is_some()
}

fn base64_encoded_len(decoded: u64) -> Option<u64> {
    decoded.div_ceil(3).checked_mul(4)
}

/// One image: metadata plus inline bytes.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Image {
    descriptor: ImageDescriptor,
    data: ImageBytes,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageRaw {
    descriptor: ImageDescriptor,
    data: String,
}

impl TryFrom<ImageRaw> for Image {
    type Error = A2aLabError;

    fn try_from(raw: ImageRaw) -> Result<Self, Self::Error> {
        Self::from_base64(raw.descriptor, &raw.data, ImageTransportConfig::default())
    }
}

impl<'de> Deserialize<'de> for Image {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl Image {
    /// Pairs metadata with decoded bytes under the default transport limit.
    pub fn new(descriptor: ImageDescriptor, data: Vec<u8>) -> Result<Self, A2aLabError> {
        Self::with_transport(descriptor, data, ImageTransportConfig::default())
    }

    /// Pairs metadata with decoded bytes under `transport`.
    pub fn with_transport(
        descriptor: ImageDescriptor,
        data: Vec<u8>,
        transport: ImageTransportConfig,
    ) -> Result<Self, A2aLabError> {
        ensure_consistent(&descriptor, &data)?;
        transport.check_payload(&data)?;
        Ok(Self {
            descriptor,
            data: ImageBytes(data),
        })
    }

    /// Decodes standard base64 and pairs it with `descriptor` under `transport`.
    pub fn from_base64(
        descriptor: ImageDescriptor,
        encoded: &str,
        transport: ImageTransportConfig,
    ) -> Result<Self, A2aLabError> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| A2aLabError::invalid("data", "must be standard base64"))?;
        Self::with_transport(descriptor, data, transport)
    }

    /// Metadata without pixel bytes.
    #[must_use]
    pub const fn descriptor(&self) -> &ImageDescriptor {
        &self.descriptor
    }

    /// Decoded payload bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data.0
    }
}

fn ensure_consistent(descriptor: &ImageDescriptor, data: &[u8]) -> Result<(), A2aLabError> {
    check_media_type(descriptor.media_type())?;
    check_dimension("width", descriptor.width())?;
    check_dimension("height", descriptor.height())?;
    if data.is_empty() {
        return Err(A2aLabError::invalid(
            "image",
            "payload is inconsistent with the descriptor",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageBytes(Vec<u8>);

impl Serialize for ImageBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.0))
    }
}

impl JsonSchema for ImageBytes {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ImageBytes".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "contentEncoding": "base64"
        })
    }
}

/// Request for a page of image sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListImageSourcesRequest {
    page: PageRequest,
}

impl ListImageSourcesRequest {
    /// Creates a request when `page` is valid.
    pub fn new(page: PageRequest) -> Result<Self, A2aLabError> {
        Ok(Self {
            page: checked_page(page)?,
        })
    }

    /// Page bounds.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }
}

impl TryFrom<PageRequestRaw> for ListImageSourcesRequest {
    type Error = A2aLabError;

    fn try_from(raw: PageRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.page)
    }
}

impl<'de> Deserialize<'de> for ListImageSourcesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PageRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Request for a page of image metadata from one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListImagesRequest {
    source_id: ImageSourceId,
    page: PageRequest,
}

#[derive(Deserialize)]
struct ListImagesRequestRaw {
    source_id: ImageSourceId,
    page: PageRequest,
}

impl ListImagesRequest {
    /// Creates a request when `page` is valid.
    pub fn new(source_id: ImageSourceId, page: PageRequest) -> Result<Self, A2aLabError> {
        Ok(Self {
            source_id,
            page: checked_page(page)?,
        })
    }

    /// Source to list.
    #[must_use]
    pub const fn source_id(&self) -> &ImageSourceId {
        &self.source_id
    }

    /// Page bounds.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }
}

impl TryFrom<ListImagesRequestRaw> for ListImagesRequest {
    type Error = A2aLabError;

    fn try_from(raw: ListImagesRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.source_id, raw.page)
    }
}

impl<'de> Deserialize<'de> for ListImagesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ListImagesRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Search by an optional source plus a half-open UTC range and/or nonblank text.
///
/// A source by itself is not a criterion. At least one of `range` or nonblank
/// `text` is required. `range` uses `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SearchImagesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_id: Option<ImageSourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    page: PageRequest,
}

#[derive(Deserialize)]
struct SearchImagesRequestRaw {
    #[serde(default)]
    source_id: Option<ImageSourceId>,
    #[serde(default)]
    range: Option<TimeRange>,
    #[serde(default)]
    text: Option<String>,
    page: PageRequest,
}

impl SearchImagesRequest {
    /// Creates a search when the page, range, and text criteria are valid.
    pub fn new(
        source_id: Option<ImageSourceId>,
        range: Option<TimeRange>,
        text: Option<String>,
        page: PageRequest,
    ) -> Result<Self, A2aLabError> {
        let request = Self {
            source_id,
            range,
            text,
            page,
        };
        request.check()?;
        Ok(request)
    }

    /// Rejects an invalid page or range, blank text, and a search with neither criterion.
    pub fn check(&self) -> Result<(), A2aLabError> {
        self.page.check()?;
        if let Some(range) = self.range {
            range.check()?;
        }
        if self
            .text
            .as_ref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err(A2aLabError::invalid("text", "must be nonblank"));
        }
        if self.range.is_none() && self.text.is_none() {
            return Err(A2aLabError::invalid(
                "search",
                "requires a time range or nonblank text",
            ));
        }
        Ok(())
    }

    /// Optional source filter.
    #[must_use]
    pub const fn source_id(&self) -> Option<&ImageSourceId> {
        self.source_id.as_ref()
    }

    /// Half-open UTC interval, when the search is bounded in time.
    #[must_use]
    pub const fn range(&self) -> Option<TimeRange> {
        self.range
    }

    /// Nonblank text query, when the search is bounded by text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Page bounds.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }
}

impl TryFrom<SearchImagesRequestRaw> for SearchImagesRequest {
    type Error = A2aLabError;

    fn try_from(raw: SearchImagesRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.source_id, raw.range, raw.text, raw.page)
    }
}

impl<'de> Deserialize<'de> for SearchImagesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SearchImagesRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Request for one image by its identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetImageRequest {
    id: ImageId,
}

impl GetImageRequest {
    /// Creates a request for `id`.
    #[must_use]
    pub const fn new(id: ImageId) -> Self {
        Self { id }
    }

    /// Image to read.
    #[must_use]
    pub const fn id(&self) -> &ImageId {
        &self.id
    }
}

/// Request for the current frame of one source.
///
/// This is a source lookup. It is not an image lookup with an omitted id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetCurrentImageRequest {
    source_id: ImageSourceId,
}

impl GetCurrentImageRequest {
    /// Creates a request for the current frame of `source_id`.
    #[must_use]
    pub const fn new(source_id: ImageSourceId) -> Self {
        Self { source_id }
    }

    /// Source whose current frame should be read.
    #[must_use]
    pub const fn source_id(&self) -> &ImageSourceId {
        &self.source_id
    }
}

#[derive(Deserialize)]
struct PageRequestRaw {
    page: PageRequest,
}

fn checked_page(page: PageRequest) -> Result<PageRequest, A2aLabError> {
    page.check()?;
    Ok(page)
}

fn empty_attributes() -> JsonObject {
    JsonObject::empty()
}

fn check_media_type(media_type: &str) -> Result<(), A2aLabError> {
    if media_type.is_empty() {
        return Err(A2aLabError::invalid("media_type", "must not be empty"));
    }
    let Some((kind, rest)) = media_type.split_once('/') else {
        return Err(A2aLabError::invalid(
            "media_type",
            "must be an image/* media type",
        ));
    };
    let subtype = rest.split(';').next().unwrap_or("");
    if !kind.eq_ignore_ascii_case("image")
        || subtype.is_empty()
        || subtype.chars().any(char::is_whitespace)
    {
        return Err(A2aLabError::invalid(
            "media_type",
            "must be an image/* media type",
        ));
    }
    Ok(())
}

fn check_dimension(field: &'static str, value: u32) -> Result<(), A2aLabError> {
    if value == 0 {
        return Err(A2aLabError::invalid(field, "must be positive"));
    }
    Ok(())
}
