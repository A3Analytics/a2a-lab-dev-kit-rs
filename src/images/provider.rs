//! Image provider interface.

use crate::error::A2aLabError;
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageSource,
    ListImageSourcesRequest, ListImagesRequest, SearchImagesRequest,
};
use crate::page::Page;

/// Lists image sources and reads metadata or inline frames.
///
/// `get_current_image` asks for the provider's current frame. Whether that frame
/// is newly captured, extracted from video, or the latest persisted image is
/// provider-defined. The operation always names a source; it does not omit an
/// image id. Implementations supply frames. This crate has no HTTP, USB, or
/// IP-camera capture helper.
pub trait ImageProvider: Send + Sync {
    /// Returns the image sources this agent can read.
    fn list_image_sources(
        &self,
        request: ListImageSourcesRequest,
    ) -> impl Future<Output = Result<Page<ImageSource>, A2aLabError>> + Send;

    /// Returns metadata for images from one source.
    fn list_images(
        &self,
        request: ListImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send;

    /// Returns metadata matching an optional source plus a time range and/or text.
    fn search_images(
        &self,
        request: SearchImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send;

    /// Returns one image, including its inline bytes.
    fn get_image(
        &self,
        request: GetImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send;

    /// Returns the provider-defined current frame for one source.
    fn get_current_image(
        &self,
        request: GetCurrentImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send;
}
