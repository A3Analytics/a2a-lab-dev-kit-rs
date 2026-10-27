//! Authenticated MJPEG downloads.
//!
//! `GET /media/video-clips/{id}` is a finite clip. `GET /media/video-live/{id}`
//! follows frames captured after the handle opens. A2A and MCP mount [`router`]
//! and build absolute URLs with [`clip_url`] and [`live_url`]. Failures are
//! JSON `{"error":{"code","message"}}` where `code` is [`A2aLabError::code`].

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use futures_util::stream::{self, Stream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::service::A2aLabApi;
use crate::time::UtcTimestamp;
use crate::videos::model::{
    DEFAULT_MJPEG_BOUNDARY, encode_mjpeg_end, encode_mjpeg_part, mjpeg_content_type,
};
use crate::videos::{
    VideoClipStream, VideoFrame, VideoLiveStream, video_clip_path, video_live_path,
};

/// Encoded live parts waiting on a slow client.
///
/// One part keeps the response queue bounded. Disconnect or cancellation drops
/// that part with the producer.
const LIVE_PART_QUEUE: usize = 1;

/// Serves clip and live-follow downloads.
///
/// Callers apply their own access gate. This router does not add a public
/// route, a shared frame cache, or byte-range responses.
pub fn router(lab: Arc<dyn A2aLabApi>) -> Router {
    Router::new()
        .route("/media/video-clips/{id}", get(get_clip))
        .route("/media/video-live/{id}", get(get_live))
        .with_state(lab)
}

async fn get_clip(State(lab): State<Arc<dyn A2aLabApi>>, Path(id): Path<String>) -> Response {
    let id = match VideoClipId::new(id) {
        Ok(id) => id,
        Err(error) => return media_error(&error),
    };
    match lab.open_video_clip(&id).await {
        Ok(frames) => mjpeg_response(Body::from_stream(clip_parts(frames))),
        Err(error) => media_error(&error),
    }
}

async fn get_live(State(lab): State<Arc<dyn A2aLabApi>>, Path(id): Path<String>) -> Response {
    let id = match VideoFollowId::new(id) {
        Ok(id) => id,
        Err(error) => return media_error(&error),
    };
    match lab.open_video_follow(&id).await {
        Ok(frames) => mjpeg_response(Body::from_stream(live_parts(frames))),
        Err(error) => media_error(&error),
    }
}

fn mjpeg_response(body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    apply_mjpeg_headers(&mut response);
    response
}

fn apply_mjpeg_headers(response: &mut Response) {
    let headers = response.headers_mut();
    if let Ok(content_type) = HeaderValue::from_str(&mjpeg_content_type(DEFAULT_MJPEG_BOUNDARY)) {
        headers.insert(CONTENT_TYPE, content_type);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
}

fn clip_parts(frames: VideoClipStream) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    enum Phase {
        Frames(VideoClipStream),
        End,
        Done,
    }
    stream::unfold(Phase::Frames(frames), |phase| async move {
        match phase {
            Phase::Frames(mut frames) => match frames.next().await {
                Some(Ok(frame)) => Some((
                    Ok(Bytes::from(encode_mjpeg_part(
                        &frame,
                        DEFAULT_MJPEG_BOUNDARY,
                    ))),
                    Phase::Frames(frames),
                )),
                Some(Err(error)) => {
                    Some((Err(std::io::Error::other(error.to_string())), Phase::Done))
                }
                None => Some((
                    Ok(Bytes::from(encode_mjpeg_end(DEFAULT_MJPEG_BOUNDARY))),
                    Phase::End,
                )),
            },
            Phase::End | Phase::Done => None,
        }
    })
}

struct LiveProducer {
    task: JoinHandle<()>,
    rx: mpsc::Receiver<Result<Bytes, std::io::Error>>,
}

impl Drop for LiveProducer {
    fn drop(&mut self) {
        self.task.abort();
        while self.rx.try_recv().is_ok() {}
    }
}

fn live_parts(frames: VideoLiveStream) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    let (tx, rx) = mpsc::channel(LIVE_PART_QUEUE);
    let task = tokio::spawn(produce_live(frames, tx));
    stream::unfold(LiveProducer { task, rx }, |mut producer| async move {
        producer.rx.recv().await.map(|item| (item, producer))
    })
}

async fn produce_live(
    mut frames: VideoLiveStream,
    tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
) {
    while let Some(frame) = frames.next().await {
        match part_bytes(frame) {
            Ok(bytes) => {
                if tx.send(Ok(bytes)).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = tx.send(Err(std::io::Error::other(error.to_string()))).await;
                return;
            }
        }
    }
    let _ = tx
        .send(Ok(Bytes::from(encode_mjpeg_end(DEFAULT_MJPEG_BOUNDARY))))
        .await;
}

fn part_bytes(frame: Result<VideoFrame, A2aLabError>) -> Result<Bytes, A2aLabError> {
    encoded_part(&frame?)
}

fn encoded_part(frame: &VideoFrame) -> Result<Bytes, A2aLabError> {
    let stamp = frame.captured_at().to_rfc3339();
    UtcTimestamp::parse(&stamp)?;
    Ok(Bytes::from(encode_mjpeg_part(
        frame,
        DEFAULT_MJPEG_BOUNDARY,
    )))
}

fn media_error(error: &A2aLabError) -> Response {
    let status = match error {
        A2aLabError::Invalid { .. } => StatusCode::BAD_REQUEST,
        A2aLabError::Unavailable { .. }
        | A2aLabError::Video {
            availability: crate::error::VideoAvailability::LiveUnavailable,
            ..
        } => StatusCode::SERVICE_UNAVAILABLE,
        A2aLabError::Transport { .. } | A2aLabError::Protocol { .. } => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        A2aLabError::NotFound { .. } | A2aLabError::Video { .. } => StatusCode::NOT_FOUND,
    };
    let body = serde_json::json!({
        "error": {
            "code": error.code(),
            "message": error.to_string(),
        }
    });
    let mut response = body.to_string().into_response();
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

/// Absolute clip URL at `base` for `id`.
#[must_use]
pub fn clip_url(base: &str, id: &VideoClipId) -> String {
    format!("{}{}", base.trim_end_matches('/'), video_clip_path(id))
}

/// Absolute live-follow URL at `base` for `id`.
///
/// `base` is the listener origin. The path stays the opaque relative media path.
#[must_use]
pub fn live_url(base: &str, id: &VideoFollowId) -> String {
    format!("{}{}", base.trim_end_matches('/'), video_live_path(id))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::{LIVE_PART_QUEUE, live_parts, live_url};
    use crate::error::A2aLabError;
    use crate::id::VideoFollowId;
    use crate::time::UtcTimestamp;
    use crate::videos::VideoFrame;

    #[test]
    fn live_url_joins_the_origin_to_the_relative_path() {
        let id = VideoFollowId::new("follow-1").unwrap();
        assert_eq!(
            live_url("http://127.0.0.1:9/", &id),
            "http://127.0.0.1:9/media/video-live/follow-1"
        );
    }

    #[tokio::test]
    async fn unpolled_live_body_keeps_a_bounded_queue_and_drops_it() {
        let pulled = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let frames = counting_frames(Arc::clone(&pulled), Arc::clone(&dropped), 30);
        let body = live_parts(Box::pin(frames));
        let start = Instant::now();
        while pulled.load(Ordering::SeqCst) == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "producer did not start"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
        let seen = pulled.load(Ordering::SeqCst);
        assert!(
            seen > 0 && seen <= LIVE_PART_QUEUE + 1,
            "producer queued {seen} frames"
        );
        drop(body);
        let start = Instant::now();
        while !dropped.load(Ordering::SeqCst) {
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "producer was retained"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(pulled.load(Ordering::SeqCst), seen);
    }

    struct Counted {
        index: usize,
        pulled: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
        count: usize,
        stamp: UtcTimestamp,
        jpeg: Vec<u8>,
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn counting_frames(
        pulled: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
        count: usize,
    ) -> impl futures_util::Stream<Item = Result<VideoFrame, A2aLabError>> + Send {
        let state = Counted {
            index: 0,
            pulled,
            dropped,
            count,
            stamp: UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
            jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9],
        };
        futures_util::stream::unfold(state, |mut state| async move {
            if state.index >= state.count {
                loop {
                    std::future::pending::<()>().await;
                }
            }
            state.pulled.fetch_add(1, Ordering::SeqCst);
            state.index += 1;
            let frame = VideoFrame::new(state.stamp, state.jpeg.clone()).unwrap();
            Some((Ok(frame), state))
        })
    }
}
