//! `LabVideos` commands over [`A2aLabApi`](crate::service::A2aLabApi).
//!
//! Still JPEGs and clip bodies use binary download. Timeline responses are metadata.
//! No command delivers frames added after the call.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::{A2aLabError, VideoAvailability};
use crate::id::VideoSourceId;
use crate::page::PageRequest;
use crate::service::{A2aLabApi, A2aLabCommand, A2aLabResult};
use crate::sila::binary::BinaryStore;
use crate::sila::errors::{defined, reject_metadata, undefined, validation};
use crate::sila::values::{
    optional_sila_text, required_sila_text, sila_bool, sila_string, timestamp_of,
};
use crate::sila::wire::sila2::com::a3analytics::lab::labvideos::v1::data_type_time_range::TimeRangeStruct;
use crate::sila::wire::sila2::com::a3analytics::lab::labvideos::v1::data_type_video_source::VideoSourceStruct;
use crate::sila::wire::sila2::com::a3analytics::lab::labvideos::v1::lab_videos_server::LabVideos;
use crate::sila::wire::sila2::com::a3analytics::lab::labvideos::v1::{
    DataTypePageRequest, DataTypeTimeRange, DataTypeVideoSource, GetVideoClipParameters,
    GetVideoClipResponses, GetVideoFrameAtParameters, GetVideoFrameAtResponses,
    GetVideoTimelineParameters, GetVideoTimelineResponses, ListVideoSourcesParameters,
    ListVideoSourcesResponses,
};
use crate::sila::wire::sila2::org::silastandard::binary::Union;
use crate::sila::wire::sila2::org::silastandard::{
    Binary, Integer, String as SilaString, Timestamp,
};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::{
    GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult, GetVideoTimelineRequest,
    ListVideoSourcesRequest, VideoClip, VideoSource, VideoTimeline,
};

const LAB_VIDEOS: &str = "com.a3analytics/lab/LabVideos/v1";

#[derive(Clone)]
pub(crate) struct VideoFeature {
    pub lab: Arc<dyn A2aLabApi>,
    pub binaries: BinaryStore,
}

#[tonic::async_trait]
impl LabVideos for VideoFeature {
    async fn list_video_sources(
        &self,
        request: Request<ListVideoSourcesParameters>,
    ) -> Result<Response<ListVideoSourcesResponses>, Status> {
        reject_metadata(&request)?;
        let page = page_from(request.into_inner().page.as_ref())?;
        let result = self
            .call(A2aLabCommand::ListVideoSources(
                ListVideoSourcesRequest::new(page).map_err(video_error)?,
            ))
            .await?;
        let A2aLabResult::ListVideoSources(page) = result else {
            return Err(undefined("unexpected list video sources result"));
        };
        Ok(Response::new(ListVideoSourcesResponses {
            sources: page.items().iter().map(source_message).collect(),
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn get_video_frame_at(
        &self,
        request: Request<GetVideoFrameAtParameters>,
    ) -> Result<Response<GetVideoFrameAtResponses>, Status> {
        reject_metadata(&request)?;
        let body = request.into_inner();
        let source_id = read_source_id(body.source_id.as_ref(), "GetVideoFrameAt")?;
        let at = required_timestamp(body.at.as_ref(), "GetVideoFrameAt", "At")?;
        let max_delta_us = if body
            .has_max_delta_us
            .as_ref()
            .is_some_and(|flag| flag.value)
        {
            let value = body.max_delta_us.as_ref().map_or(0, |item| item.value);
            Some(u64::try_from(value).map_err(|_| {
                validation(
                    parameter("GetVideoFrameAt", "MaxDeltaUs"),
                    "must be non-negative",
                )
            })?)
        } else {
            None
        };
        let result = self
            .call(A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
                source_id,
                at,
                max_delta_us,
            )))
            .await?;
        let A2aLabResult::GetVideoFrameAt(frame) = result else {
            return Err(undefined("unexpected get video frame result"));
        };
        Ok(Response::new(self.frame_response(&frame)?))
    }

    async fn get_video_timeline(
        &self,
        request: Request<GetVideoTimelineParameters>,
    ) -> Result<Response<GetVideoTimelineResponses>, Status> {
        reject_metadata(&request)?;
        let source_id =
            read_source_id(request.into_inner().source_id.as_ref(), "GetVideoTimeline")?;
        let result = self
            .call(A2aLabCommand::GetVideoTimeline(
                GetVideoTimelineRequest::new(source_id),
            ))
            .await?;
        let A2aLabResult::GetVideoTimeline(timeline) = result else {
            return Err(undefined("unexpected get video timeline result"));
        };
        Ok(Response::new(timeline_response(&timeline)?))
    }

    async fn get_video_clip(
        &self,
        request: Request<GetVideoClipParameters>,
    ) -> Result<Response<GetVideoClipResponses>, Status> {
        reject_metadata(&request)?;
        let body = request.into_inner();
        let source_id = read_source_id(body.source_id.as_ref(), "GetVideoClip")?;
        let range = required_range(body.range.as_ref(), "GetVideoClip")?;
        let result = self
            .call(A2aLabCommand::GetVideoClip(
                GetVideoClipRequest::new(source_id, range).map_err(video_error)?,
            ))
            .await?;
        let A2aLabResult::GetVideoClip(clip) = result else {
            return Err(undefined("unexpected get video clip result"));
        };
        Ok(Response::new(self.clip_response(clip).await?))
    }
}

impl VideoFeature {
    async fn call(&self, command: A2aLabCommand) -> Result<A2aLabResult, Status> {
        self.lab
            .execute(command)
            .await
            .map(|outcome| outcome.task.result)
            .map_err(video_error)
    }

    fn frame_response(
        &self,
        frame: &GetVideoFrameAtResult,
    ) -> Result<GetVideoFrameAtResponses, Status> {
        let image = frame.image();
        let descriptor = image.descriptor();
        let transfer = self.binaries.insert(image.data())?;
        Ok(GetVideoFrameAtResponses {
            requested_at: Some(stamp(frame.requested_at())?),
            actual_captured_at: Some(stamp(frame.actual_captured_at())?),
            delta_us: Some(Integer {
                value: frame.delta_us(),
            }),
            image_id: Some(sila_string(descriptor.id().as_str())),
            source_id: Some(sila_string(descriptor.source_id().as_str())),
            media_type: Some(sila_string(descriptor.media_type())),
            width: Some(Integer {
                value: i64::from(descriptor.width()),
            }),
            height: Some(Integer {
                value: i64::from(descriptor.height()),
            }),
            image: Some(Binary {
                union: Some(Union::BinaryTransferUuid(transfer)),
            }),
        })
    }

    async fn clip_response(&self, clip: VideoClip) -> Result<GetVideoClipResponses, Status> {
        let transfer = self.store_clip(&clip).await?;
        Ok(GetVideoClipResponses {
            clip_id: Some(sila_string(clip.id().as_str())),
            source_id: Some(sila_string(clip.source_id().as_str())),
            requested_range: Some(range_message(clip.requested_range())?),
            effective_range: Some(range_message(clip.effective_range())?),
            media_type: Some(sila_string(clip.media_type())),
            path: Some(sila_string(clip.path())),
            clip: Some(Binary {
                union: Some(Union::BinaryTransferUuid(transfer)),
            }),
        })
    }

    async fn store_clip(&self, clip: &VideoClip) -> Result<String, Status> {
        let asset = self
            .lab
            .export_finite_video_clip(clip.id())
            .await
            .map_err(video_error)?;
        self.binaries
            .insert_shared(Arc::from(asset.bytes().to_vec()))
    }
}

fn source_message(source: &VideoSource) -> DataTypeVideoSource {
    let (has_asset, asset) = optional_sila_text(source.asset_id());
    let (has_semantic, semantic) = optional_sila_text(source.semantic_id());
    DataTypeVideoSource {
        video_source: Some(VideoSourceStruct {
            id: Some(sila_string(source.id().as_str())),
            name: Some(sila_string(source.name())),
            description: Some(sila_string(source.description())),
            has_asset_id: Some(has_asset),
            asset_id: Some(asset),
            has_semantic_id: Some(has_semantic),
            semantic_id: Some(semantic),
            width: Some(Integer {
                value: i64::from(source.width()),
            }),
            height: Some(Integer {
                value: i64::from(source.height()),
            }),
            fps: Some(Integer {
                value: i64::from(source.fps()),
            }),
            media_type: Some(sila_string(source.media_type())),
        }),
    }
}

fn page_from(page: Option<&DataTypePageRequest>) -> Result<PageRequest, Status> {
    let page =
        page.ok_or_else(|| validation(parameter("ListVideoSources", "Page"), "is required"))?;
    let inner = page
        .page_request
        .as_ref()
        .ok_or_else(|| validation(parameter("ListVideoSources", "Page"), "is required"))?;
    let cursor = if inner.has_cursor.as_ref().is_some_and(|flag| flag.value) {
        let cursor = inner
            .cursor
            .as_ref()
            .map(|item| item.value.clone())
            .unwrap_or_default();
        if cursor.is_empty() {
            return Err(validation(
                parameter("ListVideoSources", "Page"),
                "is not a valid page cursor",
            ));
        }
        Some(cursor)
    } else {
        None
    };
    let limit = inner.limit.as_ref().map_or(0, |item| item.value);
    let limit = u32::try_from(limit).unwrap_or(0);
    PageRequest::new(cursor, limit)
        .map_err(|error| validation(parameter("ListVideoSources", "Page"), error.to_string()))
}

fn read_source_id(value: Option<&SilaString>, command: &str) -> Result<VideoSourceId, Status> {
    let text = required_sila_text(value, parameter(command, "SourceId"))?;
    VideoSourceId::new(text)
        .map_err(|error| validation(parameter(command, "SourceId"), error.to_string()))
}

fn required_timestamp(
    value: Option<&Timestamp>,
    command: &str,
    name: &str,
) -> Result<UtcTimestamp, Status> {
    let stamp = value.ok_or_else(|| validation(parameter(command, name), "is required"))?;
    crate::sila::values::read_timestamp(stamp)
        .map_err(|error| validation(parameter(command, name), error.to_string()))
}

fn required_range(value: Option<&DataTypeTimeRange>, command: &str) -> Result<TimeRange, Status> {
    let range = value.ok_or_else(|| validation(parameter(command, "Range"), "is required"))?;
    let inner = range
        .time_range
        .as_ref()
        .ok_or_else(|| validation(parameter(command, "Range"), "is required"))?;
    let start = required_timestamp(inner.start.as_ref(), command, "Range")?;
    let end = required_timestamp(inner.end.as_ref(), command, "Range")?;
    TimeRange::new(start, end)
        .map_err(|error| validation(parameter(command, "Range"), error.to_string()))
}

fn timeline_response(timeline: &VideoTimeline) -> Result<GetVideoTimelineResponses, Status> {
    let mut intervals = Vec::with_capacity(timeline.intervals().len());
    for interval in timeline.intervals() {
        intervals.push(range_message(*interval)?);
    }
    let newest = timeline.newest_at();
    Ok(GetVideoTimelineResponses {
        source_id: Some(sila_string(timeline.source_id().as_str())),
        intervals,
        has_newest_at: Some(sila_bool(newest.is_some())),
        newest_at: newest.map(stamp).transpose()?,
        live_available: Some(sila_bool(timeline.live_available())),
    })
}

fn range_message(range: TimeRange) -> Result<DataTypeTimeRange, Status> {
    Ok(DataTypeTimeRange {
        time_range: Some(TimeRangeStruct {
            start: Some(stamp(range.start())?),
            end: Some(stamp(range.end())?),
        }),
    })
}

fn stamp(value: UtcTimestamp) -> Result<Timestamp, Status> {
    timestamp_of(value).map_err(|error| undefined(error.to_string()))
}

fn parameter(command: &str, name: &str) -> String {
    format!("{LAB_VIDEOS}/Command/{command}/Parameter/{name}")
}

fn video_error(error: A2aLabError) -> Status {
    match error {
        A2aLabError::NotFound { kind, id } => {
            let identifier = match kind {
                "video source" => "VideoSourceNotFound",
                "video frame" => "VideoFrameNotFound",
                _ => "VideoClipNotFound",
            };
            defined(
                LAB_VIDEOS,
                identifier,
                format!("{kind} `{id}` was not found"),
            )
        }
        A2aLabError::Unavailable { message } => defined(LAB_VIDEOS, "ProviderUnavailable", message),
        A2aLabError::Video {
            availability,
            message,
        } => defined(LAB_VIDEOS, video_defined_error(availability), message),
        A2aLabError::Invalid { message, .. }
        | A2aLabError::Transport { message }
        | A2aLabError::Protocol { message } => undefined(message),
    }
}

fn video_defined_error(availability: VideoAvailability) -> &'static str {
    match availability {
        VideoAvailability::NoRecording => "NoRecording",
        VideoAvailability::RecordingGap => "RecordingGap",
        VideoAvailability::ClipExpired => "ClipExpired",
        VideoAvailability::LiveUnavailable => "LiveUnavailable",
    }
}
