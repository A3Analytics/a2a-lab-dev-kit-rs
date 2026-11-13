//! Frame-at routing across live memory, warm stills, and SQLite DVR.

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId};
use crate::images::{Image, ImageDescriptor};
use crate::json_object::JsonObject;
use crate::memory::MemoryVideos;
use crate::time::UtcTimestamp;
use crate::videos::{
    DvrFrameAtRequest, DvrMediaPayload, DvrMediaStore, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, VideoProvider, jpeg_dimensions,
};

impl super::DvrVideoProvider {
    pub(super) async fn resolve_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> Result<GetVideoFrameAtResult, A2aLabError> {
        if let Ok(found) = self.live.get_video_frame_at(request.clone()).await {
            return Ok(found);
        }
        if let Some(cache) = self.warm_stills.as_ref()
            && let Some(still) = cache.latest(request.source_id().as_str())
            && warm_still_matches(&request, &still)
            && let Ok(image) = warm_still_image(request.source_id(), still.jpeg())
        {
            return Ok(GetVideoFrameAtResult::new(request.at(), image));
        }
        let track_id = MemoryVideos::mjpeg_track_id(request.source_id())?;
        let frame = self
            .dvr
            .frame_at(DvrFrameAtRequest::new(
                request.source_id().clone(),
                track_id,
                request.at(),
                request.max_delta_us(),
            ))
            .await?;
        let jpeg = match frame.payload() {
            DvrMediaPayload::MjpegFrame(bytes) => bytes.clone(),
            DvrMediaPayload::EncodedSample { .. } => {
                return Err(A2aLabError::invalid(
                    "codec",
                    "MJPEG frame-at requires an MJPEG track",
                ));
            }
        };
        let (width, height) = jpeg_dimensions(&jpeg)?;
        let descriptor = ImageDescriptor::new(
            ImageId::new(format!(
                "{}-{}",
                request.source_id(),
                frame.actual_at().as_micros()
            ))?,
            ImageSourceId::new(request.source_id().as_str())?,
            frame.actual_at(),
            "image/jpeg",
            width,
            height,
            None,
            JsonObject::empty(),
        )?;
        Ok(GetVideoFrameAtResult::new(
            request.at(),
            Image::new(descriptor, jpeg)?,
        ))
    }
}

fn warm_still_matches(request: &GetVideoFrameAtRequest, still: &crate::videos::WarmStill) -> bool {
    if still.jpeg().is_empty() {
        return false;
    }
    let now = UtcTimestamp::now();
    let delta = now.as_micros().abs_diff(request.at().as_micros());
    match request.max_delta_us() {
        Some(limit) => delta <= limit,
        None => delta <= 5_000_000,
    }
}

fn warm_still_image(
    source_id: &crate::id::VideoSourceId,
    jpeg: &[u8],
) -> Result<Image, A2aLabError> {
    let (width, height) = jpeg_dimensions(jpeg)?;
    let at = UtcTimestamp::now();
    let descriptor = ImageDescriptor::new(
        ImageId::new(format!("{}-{}-warm", source_id, at.as_micros()))?,
        ImageSourceId::new(source_id.as_str())?,
        at,
        "image/jpeg",
        width,
        height,
        None,
        JsonObject::empty(),
    )?;
    Image::new(descriptor, jpeg.to_vec())
}
