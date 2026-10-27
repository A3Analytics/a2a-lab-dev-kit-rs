//! Timeline-centered video reader model.
//!
//! Saved images stay in the Images primitive. Frame extraction keeps the
//! public operation name `get_video_frame_at`.

#![allow(dead_code)]

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::Fault;
use crate::metrics::{self, QueryRange};

use super::mjpeg::captured_us;

pub const CLIP_PATH_PREFIX: &str = "/media/video-clips/";
pub const LIVE_PATH_PREFIX: &str = "/media/video-live/";
pub const MAX_CLIP_US: i64 = 60 * 1_000_000;
pub const MJPEG_MEDIA_TYPE: &str = "multipart/x-mixed-replace; type=image/jpeg";
pub const MEDIA_CACHE_CONTROL: &str = "no-store";
pub const CAPTURED_AT_HEADER: &str = "X-Captured-At";

pub fn frame_at_time_label() -> &'static str {
    "Frame at time"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: String,
    pub end: String,
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub source_id: String,
    pub intervals: Vec<Interval>,
    pub newest_at: Option<String>,
    pub newest_us: Option<i64>,
    pub live_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveHandle {
    pub id: String,
    pub source_id: String,
    pub path: String,
    pub media_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipHandle {
    pub id: String,
    pub source_id: String,
    pub path: String,
    pub requested_start: String,
    pub requested_end: String,
    pub effective_start: String,
    pub effective_end: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameAtTime {
    pub captured_at: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCapability {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NoRecording,
    RecordingGap,
    ClipExpired,
    LiveUnavailable,
    Invalid,
    NotFound,
    Unavailable,
}

impl Failure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoRecording => "no_recording",
            Self::RecordingGap => "recording_gap",
            Self::ClipExpired => "clip_expired",
            Self::LiveUnavailable => "live_unavailable",
            Self::Invalid => "invalid",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "no_recording" => Self::NoRecording,
            "recording_gap" => Self::RecordingGap,
            "clip_expired" => Self::ClipExpired,
            "live_unavailable" => Self::LiveUnavailable,
            "invalid" => Self::Invalid,
            "not_found" => Self::NotFound,
            "unavailable" => Self::Unavailable,
            _ => return None,
        })
    }

    pub const fn allows_retry(self) -> bool {
        matches!(
            self,
            Self::ClipExpired | Self::LiveUnavailable | Self::Unavailable
        )
    }

    pub const fn allows_reselect(self) -> bool {
        matches!(
            self,
            Self::NoRecording | Self::RecordingGap | Self::LiveUnavailable
        )
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::NoRecording => "No recording covers the requested range.",
            Self::RecordingGap => "The recording contains a gap.",
            Self::ClipExpired => "The clip expired.",
            Self::LiveUnavailable => "Live video is unavailable.",
            Self::Invalid => "The video request is invalid.",
            Self::NotFound => "The requested video was not found.",
            Self::Unavailable => "Video is unavailable.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Empty,
    Live,
    Recorded {
        start: String,
        end: String,
        start_us: i64,
        end_us: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedRange {
    pub start: String,
    pub end: String,
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeFault {
    MissingBounds,
    NotUtc,
    Reversed,
    TooLong,
    NoRecording,
    RecordingGap,
}

impl RangeFault {
    pub const fn message(self) -> &'static str {
        match self {
            Self::MissingBounds => "Enter a start and an end.",
            Self::NotUtc => "Enter a UTC time range.",
            Self::Reversed => "The start must be before the end.",
            Self::TooLong => "The clip range must be at most 60 seconds.",
            Self::NoRecording => "no_recording",
            Self::RecordingGap => "recording_gap",
        }
    }

    pub const fn failure(self) -> Option<Failure> {
        match self {
            Self::NoRecording => Some(Failure::NoRecording),
            Self::RecordingGap => Some(Failure::RecordingGap),
            _ => None,
        }
    }
}

pub fn timeline_command(source_id: &str) -> Value {
    serde_json::json!({
        "operation": "get_video_timeline",
        "params": { "source_id": source_id }
    })
}

pub fn follow_command(source_id: &str) -> Value {
    serde_json::json!({
        "operation": "follow_video",
        "params": { "source_id": source_id }
    })
}

pub fn clip_command(source_id: &str, start: &str, end: &str) -> Value {
    serde_json::json!({
        "operation": "get_video_clip",
        "params": {
            "source_id": source_id,
            "range": { "start": start, "end": end }
        }
    })
}

pub fn frame_at_time_command(source_id: &str, at: &str) -> Value {
    serde_json::json!({
        "operation": "get_video_frame_at",
        "params": { "source_id": source_id, "at": at }
    })
}

pub fn interpret_timeline(reply: &Reply, secret: Option<&str>) -> Result<Timeline, Fault> {
    parse_timeline(&one_payload(&decode(reply, secret)?)?)
}

pub fn interpret_follow(reply: &Reply, secret: Option<&str>) -> Result<LiveHandle, Fault> {
    parse_follow(&one_payload(&decode(reply, secret)?)?)
}

pub fn interpret_clip(reply: &Reply, secret: Option<&str>) -> Result<ClipHandle, Fault> {
    parse_clip(&one_payload(&decode(reply, secret)?)?)
}

pub fn interpret_frame_at_time(reply: &Reply, secret: Option<&str>) -> Result<FrameAtTime, Fault> {
    parse_frame(&one_payload(&decode(reply, secret)?)?)
}

pub fn stable_failure(body: &str) -> Option<Failure> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?.as_object()?;
    if let Some(code) = metadata_code(error) {
        return Failure::parse(code);
    }
    match error.get("code") {
        Some(Value::String(code)) => Failure::parse(code),
        _ => None,
    }
}

pub fn parse_source_capability(value: &Value) -> Result<SourceCapability, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    if embeds_media(value) {
        return Err(Fault::Malformed);
    }
    match object.get("media_type") {
        None => {}
        Some(Value::String(media_type)) if media_type == MJPEG_MEDIA_TYPE => {}
        Some(_) => return Err(Fault::Malformed),
    }
    Ok(SourceCapability {
        id: required(object, "id")?,
        name: required(object, "name")?,
        width: positive_u32(object, "width")?,
        height: positive_u32(object, "height")?,
        fps: positive_u32(object, "fps")?,
    })
}

pub fn check_range(start: &str, end: &str) -> Result<CheckedRange, RangeFault> {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() || end.is_empty() {
        return Err(RangeFault::MissingBounds);
    }
    let (start, start_us) = utc_point(start).ok_or(RangeFault::NotUtc)?;
    let (end, end_us) = utc_point(end).ok_or(RangeFault::NotUtc)?;
    if start_us >= end_us {
        return Err(RangeFault::Reversed);
    }
    if end_us.saturating_sub(start_us) > MAX_CLIP_US {
        return Err(RangeFault::TooLong);
    }
    Ok(CheckedRange {
        start,
        end,
        start_us,
        end_us,
    })
}

pub fn check_coverage(timeline: &Timeline, start_us: i64, end_us: i64) -> Result<(), RangeFault> {
    match coverage(&timeline.intervals, start_us, end_us) {
        None => Ok(()),
        Some(Failure::RecordingGap) => Err(RangeFault::RecordingGap),
        Some(_) => Err(RangeFault::NoRecording),
    }
}

pub fn frame_instant(at: &str) -> Result<String, &'static str> {
    let at = at.trim();
    if at.is_empty() {
        return Err("Enter a UTC time.");
    }
    utc_point(at)
        .map(|(stamp, _)| stamp)
        .ok_or("Enter a UTC time.")
}

fn coverage(intervals: &[Interval], start_us: i64, end_us: i64) -> Option<Failure> {
    let logical = logical_intervals(intervals);
    let inside = logical
        .iter()
        .any(|interval| start_us >= interval.start_us && end_us <= interval.end_us);
    if inside {
        return None;
    }
    let hits = logical
        .iter()
        .filter(|interval| start_us < interval.end_us && end_us > interval.start_us)
        .count();
    if hits == 0 {
        Some(Failure::NoRecording)
    } else {
        Some(Failure::RecordingGap)
    }
}

pub(crate) fn logical_intervals(intervals: &[Interval]) -> Vec<Interval> {
    let mut merged: Vec<Interval> = Vec::new();
    for interval in intervals {
        if let Some(last) = merged.last_mut()
            && interval.start_us == last.end_us
        {
            last.end_us = interval.end_us;
            last.end = interval.end.clone();
            continue;
        }
        merged.push(interval.clone());
    }
    merged
}

fn utc_point(value: &str) -> Option<(String, i64)> {
    let QueryRange::Bounded { start, .. } =
        metrics::query_range(value, "2100-01-01T00:00:00").ok()?
    else {
        return None;
    };
    let us = captured_us(&start)?;
    Some((start, us))
}

fn decode(reply: &Reply, secret: Option<&str>) -> Result<Outcome, Fault> {
    match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => Err(status_fault(&status)),
        Ok(outcome) => Ok(outcome),
    }
}

fn status_fault(status: &codec::RpcStatus) -> Fault {
    let unavailable = status.status == "INTERNAL"
        || status.status == "UNAVAILABLE"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "INTERNAL_ERROR");
    if unavailable {
        Fault::Unavailable(status.message.clone())
    } else {
        Fault::Malformed
    }
}

fn one_payload(outcome: &Outcome) -> Result<String, Fault> {
    let payloads = match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task
            .artifacts
            .iter()
            .flat_map(|artifact| artifact.data.clone())
            .collect::<Vec<_>>(),
        Outcome::Exchange(Exchange::Message(message)) => message.data.clone(),
        Outcome::Tasks(_) | Outcome::Status(_) => Vec::new(),
    };
    let [payload] = payloads.as_slice() else {
        return Err(Fault::Malformed);
    };
    Ok(payload.clone())
}

fn parse_timeline(payload: &str) -> Result<Timeline, Fault> {
    let result = owned_result(payload, "get_video_timeline")?;
    let source_id = required(&result, "source_id")?;
    let intervals = parse_intervals(result.get("intervals").ok_or(Fault::Malformed)?)?;
    let (newest_at, newest_us) = parse_newest(result.get("newest_at"), &intervals)?;
    let live_available = result
        .get("live_available")
        .and_then(Value::as_bool)
        .ok_or(Fault::Malformed)?;
    Ok(Timeline {
        source_id,
        intervals,
        newest_at,
        newest_us,
        live_available,
    })
}

fn parse_follow(payload: &str) -> Result<LiveHandle, Fault> {
    let result = owned_result(payload, "follow_video")?;
    let id = required(&result, "id")?;
    let path = required(&result, "path")?;
    let media_type = required(&result, "media_type")?;
    if media_type != MJPEG_MEDIA_TYPE || !safe_media_path(&path, LIVE_PATH_PREFIX, &id) {
        return Err(Fault::Malformed);
    }
    Ok(LiveHandle {
        id,
        source_id: required(&result, "source_id")?,
        path,
        media_type,
    })
}

fn parse_clip(payload: &str) -> Result<ClipHandle, Fault> {
    let result = owned_result(payload, "get_video_clip")?;
    let id = required(&result, "id")?;
    let path = required(&result, "path")?;
    let media_type = required(&result, "media_type")?;
    if media_type != MJPEG_MEDIA_TYPE || !safe_media_path(&path, CLIP_PATH_PREFIX, &id) {
        return Err(Fault::Malformed);
    }
    let requested = range_of(&result, "requested_range")?;
    let effective = range_of(&result, "effective_range")?;
    Ok(ClipHandle {
        id,
        source_id: required(&result, "source_id")?,
        path,
        requested_start: requested.0,
        requested_end: requested.1,
        effective_start: effective.0,
        effective_end: effective.1,
    })
}

fn parse_frame(payload: &str) -> Result<FrameAtTime, Fault> {
    let result = owned_result(payload, "get_video_frame_at")?;
    let image = result
        .get("image")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let descriptor = image
        .get("descriptor")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    Ok(FrameAtTime {
        captured_at: required(descriptor, "captured_at")
            .or_else(|_| required(&result, "actual_captured_at"))?,
        media_type: required(descriptor, "media_type")?,
        data: required(image, "data")?,
    })
}

fn owned_result(payload: &str, operation: &str) -> Result<Map<String, Value>, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    if object.get("operation").and_then(Value::as_str) != Some(operation) {
        return Err(Fault::Malformed);
    }
    if embeds_media(&value) && operation != "get_video_frame_at" {
        return Err(Fault::Malformed);
    }
    object
        .get("result")
        .and_then(Value::as_object)
        .cloned()
        .ok_or(Fault::Malformed)
}

fn parse_intervals(value: &Value) -> Result<Vec<Interval>, Fault> {
    let items = value.as_array().ok_or(Fault::Malformed)?;
    let mut intervals = Vec::new();
    let mut previous_end = None;
    for item in items {
        let object = item.as_object().ok_or(Fault::Malformed)?;
        let (start, start_us) =
            utc_point(object.get("start").and_then(Value::as_str).unwrap_or(""))
                .ok_or(Fault::Malformed)?;
        let (end, end_us) = utc_point(object.get("end").and_then(Value::as_str).unwrap_or(""))
            .ok_or(Fault::Malformed)?;
        if start_us >= end_us {
            return Err(Fault::Malformed);
        }
        if previous_end.is_some_and(|end| start_us < end) {
            return Err(Fault::Malformed);
        }
        previous_end = Some(end_us);
        intervals.push(Interval {
            start,
            end,
            start_us,
            end_us,
        });
    }
    Ok(intervals)
}

fn parse_newest(
    value: Option<&Value>,
    intervals: &[Interval],
) -> Result<(Option<String>, Option<i64>), Fault> {
    match value {
        None | Some(Value::Null) => Ok((None, None)),
        Some(Value::String(text)) => {
            let (stamp, us) = utc_point(text).ok_or(Fault::Malformed)?;
            let Some(newest) = intervals.last() else {
                return Err(Fault::Malformed);
            };
            if us >= newest.start_us && us < newest.end_us {
                Ok((Some(stamp), Some(us)))
            } else {
                Err(Fault::Malformed)
            }
        }
        Some(_) => Err(Fault::Malformed),
    }
}

fn embeds_media(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.keys()
                .any(|key| matches!(key.as_str(), "data" | "jpeg" | "bytes" | "blob" | "image"))
                || map.values().any(embeds_media)
        }
        Value::Array(items) => items.iter().any(embeds_media),
        _ => false,
    }
}

fn safe_media_path(path: &str, prefix: &str, id: &str) -> bool {
    let expected = format!("{prefix}{id}");
    path == expected
        && safe_id(id)
        && !path.contains('?')
        && !path.contains('#')
        && !path.contains("://")
        && !path.starts_with("//")
        && !path.contains('@')
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

fn metadata_code(error: &Map<String, Value>) -> Option<&str> {
    let details = error.get("details")?.as_array()?;
    for detail in details {
        let Some(code) = detail
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("code"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if Failure::parse(code).is_some() {
            return Some(code);
        }
    }
    None
}

fn range_of(object: &Map<String, Value>, key: &str) -> Result<(String, String), Fault> {
    let range = object
        .get(key)
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    Ok((required(range, "start")?, required(range, "end")?))
}

fn required(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(Fault::Malformed),
    }
}

fn positive_u32(object: &Map<String, Value>, key: &str) -> Result<u32, Fault> {
    let Some(number) = object.get(key).and_then(Value::as_u64) else {
        return Err(Fault::Malformed);
    };
    let value = u32::try_from(number).map_err(|_| Fault::Malformed)?;
    if value == 0 {
        Err(Fault::Malformed)
    } else {
        Ok(value)
    }
}
