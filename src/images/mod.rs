//! Image sources, searchable descriptors, and inline image payloads.
//!
//! This crate does not capture frames and does not include a built-in image adapter.

mod model;
mod provider;

pub use model::{
    DEFAULT_MAX_IMAGE_BYTES, GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor,
    ImageSource, ImageTransportConfig, ListImageSourcesRequest, ListImagesRequest,
    SearchImagesRequest, TckMalformedImageRequests,
};
pub use provider::ImageProvider;
