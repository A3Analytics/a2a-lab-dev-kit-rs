//! SQLite error mapping for chunk DVR.

use crate::error::A2aLabError;
use crate::videos::codec::VideoCodec;

pub(crate) fn db_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::unavailable(format!("video DVR database: {error}"))
}

pub(crate) fn join_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::unavailable(format!("video DVR task: {error}"))
}

pub(crate) fn codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::Mjpeg => "mjpeg",
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "h265",
    }
}

pub(crate) fn parse_codec(label: &str) -> Result<VideoCodec, A2aLabError> {
    match label {
        "mjpeg" => Ok(VideoCodec::Mjpeg),
        "h264" => Ok(VideoCodec::H264),
        "h265" => Ok(VideoCodec::H265),
        _ => Err(A2aLabError::invalid("codec", "unknown stored codec")),
    }
}
