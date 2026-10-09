//! In-memory image provider.

use std::cmp::Ordering;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId};
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageProvider, ImageSource,
    ListImageSourcesRequest, ListImagesRequest, SearchImagesRequest,
};
use crate::page::{Page, slice_page};

#[derive(Default)]
struct ImageState {
    sources: Vec<ImageSource>,
    images: Vec<Image>,
    current: Vec<(ImageSourceId, ImageId)>,
    unavailable: Option<String>,
}

/// In-memory [`ImageProvider`] for examples and tests.
#[derive(Clone, Default)]
pub struct MemoryImages {
    inner: Arc<Mutex<ImageState>>,
}

impl MemoryImages {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a source.
    pub async fn insert_source(&self, source: ImageSource) -> Result<(), A2aLabError> {
        let mut state = self.inner.lock().await;
        if state
            .sources
            .iter()
            .any(|existing| existing.id == source.id)
        {
            return Err(A2aLabError::invalid("id", "image source already exists"));
        }
        state.sources.push(source);
        Ok(())
    }

    /// Adds an image for an existing source.
    pub async fn insert_image(&self, image: Image) -> Result<(), A2aLabError> {
        let mut state = self.inner.lock().await;
        let source_id = image.descriptor().source_id().clone();
        let image_id = image.descriptor().id().clone();
        require_source(&state, &source_id)?;
        if state
            .images
            .iter()
            .any(|existing| existing.descriptor().id() == &image_id)
        {
            return Err(A2aLabError::invalid("id", "image already exists"));
        }
        state.images.push(image);
        Ok(())
    }

    /// Selects the current frame for a source.
    ///
    /// The image must already belong to that source. Without a selection, current
    /// retrieval uses the latest captured frame, with image id as the tie break.
    pub async fn set_current(
        &self,
        source_id: ImageSourceId,
        image_id: ImageId,
    ) -> Result<(), A2aLabError> {
        let mut state = self.inner.lock().await;
        require_source(&state, &source_id)?;
        let Some(belongs) = state
            .images
            .iter()
            .find(|image| image.descriptor().id() == &image_id)
            .map(|image| image.descriptor().source_id() == &source_id)
        else {
            return Err(A2aLabError::not_found("image", image_id.to_string()));
        };
        if !belongs {
            return Err(A2aLabError::invalid(
                "image",
                "does not belong to the image source",
            ));
        }
        if let Some(selected) = state
            .current
            .iter_mut()
            .find(|(source, _)| source == &source_id)
        {
            selected.1 = image_id;
        } else {
            state.current.push((source_id, image_id));
        }
        Ok(())
    }

    /// Makes later calls fail until [`Self::clear_unavailable`].
    pub async fn set_unavailable(&self, message: impl Into<String>) {
        self.inner.lock().await.unavailable = Some(message.into());
    }

    /// Clears a simulated provider failure.
    pub async fn clear_unavailable(&self) {
        self.inner.lock().await.unavailable = None;
    }
}

impl ImageProvider for MemoryImages {
    async fn list_image_sources(
        &self,
        request: ListImageSourcesRequest,
    ) -> Result<Page<ImageSource>, A2aLabError> {
        let state = self.inner.lock().await;
        require_available(&state)?;
        let mut sources = state.sources.clone();
        sources.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        slice_page(&sources, request.page())
    }

    async fn list_images(
        &self,
        request: ListImagesRequest,
    ) -> Result<Page<ImageDescriptor>, A2aLabError> {
        let state = self.inner.lock().await;
        require_available(&state)?;
        require_source(&state, request.source_id())?;
        let mut descriptors = descriptors_for(&state, |descriptor| {
            descriptor.source_id() == request.source_id()
        });
        descriptors.sort_by(descriptor_order);
        slice_page(&descriptors, request.page())
    }

    async fn search_images(
        &self,
        request: SearchImagesRequest,
    ) -> Result<Page<ImageDescriptor>, A2aLabError> {
        request.check()?;
        let state = self.inner.lock().await;
        require_available(&state)?;
        if let Some(source_id) = request.source_id() {
            require_source(&state, source_id)?;
        }
        let mut descriptors =
            descriptors_for(&state, |descriptor| matches_query(descriptor, &request));
        descriptors.sort_by(descriptor_order);
        slice_page(&descriptors, request.page())
    }

    async fn get_image(&self, request: GetImageRequest) -> Result<Image, A2aLabError> {
        let state = self.inner.lock().await;
        require_available(&state)?;
        let image = find_image(&state, request.id())?;
        require_source(&state, image.descriptor().source_id())?;
        Ok(image.clone())
    }

    async fn get_current_image(
        &self,
        request: GetCurrentImageRequest,
    ) -> Result<Image, A2aLabError> {
        let state = self.inner.lock().await;
        require_available(&state)?;
        require_source(&state, request.source_id())?;
        if let Some((_, image_id)) = state
            .current
            .iter()
            .find(|(source, _)| source == request.source_id())
        {
            let image = find_image(&state, image_id)?;
            if image.descriptor().source_id() != request.source_id() {
                return Err(A2aLabError::invalid(
                    "image",
                    "does not belong to the image source",
                ));
            }
            return Ok(image.clone());
        }
        state
            .images
            .iter()
            .filter(|image| image.descriptor().source_id() == request.source_id())
            .max_by(|left, right| descriptor_order(left.descriptor(), right.descriptor()))
            .cloned()
            .ok_or_else(|| A2aLabError::not_found("image", request.source_id().to_string()))
    }
}

fn descriptors_for(
    state: &ImageState,
    include: impl Fn(&ImageDescriptor) -> bool,
) -> Vec<ImageDescriptor> {
    state
        .images
        .iter()
        .filter(|image| include(image.descriptor()))
        .map(|image| image.descriptor().clone())
        .collect()
}

fn find_image<'a>(state: &'a ImageState, image_id: &ImageId) -> Result<&'a Image, A2aLabError> {
    state
        .images
        .iter()
        .find(|image| image.descriptor().id() == image_id)
        .ok_or_else(|| A2aLabError::not_found("image", image_id.to_string()))
}

fn matches_query(descriptor: &ImageDescriptor, request: &SearchImagesRequest) -> bool {
    if request
        .source_id()
        .is_some_and(|source_id| source_id != descriptor.source_id())
    {
        return false;
    }
    if request
        .range()
        .is_some_and(|range| !range.contains(descriptor.captured_at()))
    {
        return false;
    }
    request
        .text()
        .is_none_or(|text| caption_matches(descriptor.caption(), text))
}

fn caption_matches(caption: Option<&str>, query: &str) -> bool {
    caption.is_some_and(|caption| {
        let caption = caption.to_lowercase();
        let query = query.to_lowercase();
        caption.contains(&query)
    })
}

fn descriptor_order(left: &ImageDescriptor, right: &ImageDescriptor) -> Ordering {
    left.captured_at()
        .cmp(&right.captured_at())
        .then_with(|| left.id().as_str().cmp(right.id().as_str()))
}

fn require_source(state: &ImageState, source_id: &ImageSourceId) -> Result<(), A2aLabError> {
    if state.sources.iter().any(|source| &source.id == source_id) {
        Ok(())
    } else {
        Err(A2aLabError::not_found(
            "image source",
            source_id.to_string(),
        ))
    }
}

fn require_available(state: &ImageState) -> Result<(), A2aLabError> {
    match &state.unavailable {
        Some(message) => Err(A2aLabError::unavailable(message.clone())),
        None => Ok(()),
    }
}
