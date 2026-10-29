//! Serves the web interface with the local webcam as the only video source.
//!
//! Open the printed `/ui/` URL and choose Videos. The process keeps the latest
//! two minutes of JPEG frames in memory and does not write a recording.

use std::io::{Write, stdout};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics,
    MemoryTasks, MemoryVideos, UtcTimestamp, VideoSource, VideoSourceId, WEB_UI_PATH,
};
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::{Camera, nokhwa_initialize, query};
use tokio::net::TcpListener;

const DEFAULT_BIND: &str = "127.0.0.1:3847";
const FPS: u32 = 4;
const FRAME_BUDGET: Duration = Duration::from_millis(200);
const RETAIN_US: i64 = 120 * 1_000_000;
const JPEG_QUALITY: u8 = 70;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let index = camera_index()?;
    if !permission() {
        return Err("the camera permission was denied".into());
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    std::thread::spawn(move || capture(index, handle, sender));
    let videos = receiver.recv()??;
    let listener = runtime.block_on(bind())?;
    let address = listener.local_addr()?;
    writeln!(stdout(), "http://{address}{WEB_UI_PATH}")?;
    stdout().flush()?;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share();
    runtime.block_on(async move {
        A2aServer::new(&service)
            .with_web_ui()
            .listen(listener)
            .await
    })?;
    Ok(())
}

fn permission() -> bool {
    let (sender, receiver) = std::sync::mpsc::channel();
    nokhwa_initialize(move |granted| {
        let _ = sender.send(granted);
    });
    receiver.recv().unwrap_or(false)
}

fn camera_index() -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let Some(value) = std::env::var("WEB_UI_WEBCAM_INDEX").ok() else {
        return Ok(0);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse()
        .map_err(|_| format!("WEB_UI_WEBCAM_INDEX `{value}` is not a camera index").into())
}

fn capture(
    index: u32,
    handle: tokio::runtime::Handle,
    ready: std::sync::mpsc::Sender<Result<MemoryVideos, String>>,
) {
    let opened = open_camera(index);
    let (mut camera, name, width, height, jpeg) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let videos = MemoryVideos::new();
    let source = match VideoSourceId::new("webcam") {
        Ok(source) => source,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let setup = handle.block_on(async {
        videos
            .insert_source(VideoSource::new(
                source.clone(),
                "Webcam",
                name,
                None,
                None,
                width,
                height,
                FPS,
                MJPEG_MEDIA_TYPE,
            )?)
            .await?;
        videos.set_live_available(&source, true).await?;
        publish(&videos, &source, jpeg).await?;
        Ok::<(), A2aLabError>(())
    });
    if let Err(error) = setup {
        let _ = ready.send(Err(error.to_string()));
        return;
    }
    let _ = ready.send(Ok(videos.clone()));
    while let Ok(jpeg) = jpeg_frame(&mut camera).map(|(_, _, jpeg)| jpeg) {
        let started = Instant::now();
        if handle.block_on(publish(&videos, &source, jpeg)).is_err() {
            break;
        }
        if let Some(rest) = FRAME_BUDGET.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn open_camera(index: u32) -> Result<(Camera, String, u32, u32, Vec<u8>), String> {
    let listed = query(ApiBackend::Auto).map_err(|error| error.to_string())?;
    if listed.is_empty() {
        return Err("no webcam was found".to_owned());
    }
    let mut camera = Camera::new(
        CameraIndex::Index(index),
        RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
    )
    .map_err(|error| error.to_string())?;
    camera.open_stream().map_err(|error| error.to_string())?;
    let name = camera.info().human_name();
    let (width, height, jpeg) = jpeg_frame(&mut camera).map_err(|error| error.to_string())?;
    Ok((camera, name, width, height, jpeg))
}

async fn publish(
    videos: &MemoryVideos,
    source: &VideoSourceId,
    jpeg: Vec<u8>,
) -> Result<(), A2aLabError> {
    let stamp = UtcTimestamp::now();
    videos.insert_frame(source.clone(), stamp, jpeg).await?;
    videos
        .forget_before(source, stamp.saturating_add_micros(-RETAIN_US))
        .await
}

fn jpeg_frame(camera: &mut Camera) -> Result<(u32, u32, Vec<u8>), nokhwa::NokhwaError> {
    let decoded = camera.frame()?.decode_image::<RgbFormat>()?;
    let width = NonZeroU32::new(decoded.width()).ok_or_else(|| {
        nokhwa::NokhwaError::GeneralError("the webcam frame had no width".to_owned())
    })?;
    let height = NonZeroU32::new(decoded.height()).ok_or_else(|| {
        nokhwa::NokhwaError::GeneralError("the webcam frame had no height".to_owned())
    })?;
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .write_image(
            decoded.as_raw(),
            width.get(),
            height.get(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| nokhwa::NokhwaError::GeneralError(error.to_string()))?;
    Ok((width.get(), height.get(), jpeg))
}

async fn bind() -> Result<TcpListener, Box<dyn std::error::Error + Send + Sync>> {
    let address = std::env::var("WEB_UI_BIND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BIND.to_owned());
    Ok(TcpListener::bind(&address).await?)
}
