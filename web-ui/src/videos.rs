//! Timeline-centered video reader.
//!
//! One selection follows the newest point or plays a bounded clip.
//! Saved images stay in the Images primitive.

#![allow(dead_code)]

pub(crate) mod hls;
pub(crate) mod mjpeg;
pub(crate) mod track;

mod domain;
pub(crate) mod speed;

#[allow(unused_imports)]
pub use domain::{
    CAPTURED_AT_HEADER, CLIP_PATH_PREFIX, CheckedRange, ClipHandle, Failure, FrameAtTime, Interval,
    LIVE_PATH_PREFIX, LiveHandle, MEDIA_CACHE_CONTROL, MJPEG_MEDIA_TYPE, RangeFault, Selection,
    SourceCapability, Timeline, check_coverage, check_range, clip_command, follow_command,
    frame_at_time_command, frame_at_time_label, frame_instant, interpret_clip, interpret_follow,
    interpret_frame_at_time, interpret_timeline, parse_source_capability, stable_failure,
    timeline_command,
};
#[allow(unused_imports)]
pub use speed::{
    Playback8xMetadata, PlaybackSpeed, StreamPlan, is_hls_session, join_gateway, plan_stream,
};

use crate::images::SourcePhase;

use track::{coverage_track, format_utc, newest_retained_window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Idle,
    Connecting,
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u32,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Timeline {
        source_id: String,
    },
    Follow {
        source_id: String,
    },
    Clip {
        source_id: String,
        start: String,
        end: String,
    },
    FrameAtTime {
        source_id: String,
        at: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailFact {
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    pub open: bool,
    pub facts: Vec<DetailFact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoPlayback {
    Follow(Ticket),
    Clip(Ticket),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    pub generation: u32,
    pub source_id: String,
    pub playback: Playback,
    pub selection: Selection,
    pub timeline: Option<Timeline>,
    pub live: Option<LiveHandle>,
    pub clip: Option<ClipHandle>,
    pub frame: Option<FrameAtTime>,
    pub frame_time: String,
    pub failure: Option<Failure>,
    pub loading: bool,
    pub fault: String,
    pub validation: String,
    pub playback_note: String,
    pub details_open: bool,
    expected: Option<Ticket>,
    last: Option<Kind>,
    catalog: Vec<String>,
    capabilities: Vec<SourceCapability>,
}

impl Reader {
    pub fn new() -> Self {
        Self {
            generation: 0,
            source_id: String::new(),
            playback: Playback::Idle,
            selection: Selection::Empty,
            timeline: None,
            live: None,
            clip: None,
            frame: None,
            frame_time: String::new(),
            failure: None,
            loading: false,
            fault: String::new(),
            validation: String::new(),
            playback_note: String::new(),
            details_open: false,
            expected: None,
            last: None,
            catalog: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    pub fn note_catalog(&mut self, phase: SourcePhase, ids: &[String]) {
        self.catalog = ids.to_vec();
        if !matches!(phase, SourcePhase::Ready) || ids.is_empty() {
            self.clear();
            return;
        }
        if self.source_id.is_empty() || !ids.iter().any(|id| id == &self.source_id) {
            self.source_id = ids[0].clone();
            self.enter_empty();
        }
    }

    pub fn note_capability(&mut self, capability: SourceCapability) {
        if let Some(slot) = self
            .capabilities
            .iter_mut()
            .find(|item| item.id == capability.id)
        {
            *slot = capability;
        } else {
            self.capabilities.push(capability);
        }
    }

    pub fn select_source(&mut self, id: &str) -> bool {
        if !self.catalog.iter().any(|item| item == id) || self.source_id == id {
            return false;
        }
        self.source_id = id.to_owned();
        self.enter_empty();
        true
    }

    pub fn load_timeline(&mut self) -> Result<Ticket, &'static str> {
        need_source(self)?;
        Ok(self.issue(Kind::Timeline {
            source_id: self.source_id.clone(),
        }))
    }

    pub fn install_timeline(&mut self, timeline: Timeline) -> bool {
        if timeline.source_id != self.source_id {
            return false;
        }
        self.timeline = Some(timeline);
        true
    }

    pub fn expects(&self, ticket: &Ticket) -> bool {
        self.matches(ticket)
    }

    pub fn accept_timeline(&mut self, ticket: &Ticket, timeline: Timeline) -> bool {
        if !self.matches(ticket) || timeline.source_id != self.source_id {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault.clear();
        self.failure = None;
        self.timeline = Some(timeline);
        true
    }

    pub fn follow_newest(&mut self) -> Result<Ticket, &'static str> {
        self.request_follow()
    }

    pub fn return_to_newest(&mut self) -> Result<Ticket, &'static str> {
        self.request_follow()
    }

    pub fn select_range(&mut self, start: &str, end: &str) -> Result<Ticket, &'static str> {
        let range = check_range(start, end).map_err(|fault| note_fault(self, fault))?;
        need_source(self)?;
        if let Some(timeline) = &self.timeline
            && let Err(fault) = check_coverage(timeline, range.start_us, range.end_us)
        {
            return Err(note_fault(self, fault));
        }
        self.validation.clear();
        prepare_media(self, Playback::Connecting);
        self.selection = Selection::Recorded {
            start: range.start.clone(),
            end: range.end.clone(),
            start_us: range.start_us,
            end_us: range.end_us,
        };
        Ok(self.issue(Kind::Clip {
            source_id: self.source_id.clone(),
            start: range.start,
            end: range.end,
        }))
    }

    pub fn frame_at_time(&mut self, at: &str) -> Result<Ticket, &'static str> {
        let at = match frame_instant(at) {
            Ok(at) => at,
            Err(message) => {
                self.validation = message.to_owned();
                return Err(message);
            }
        };
        need_source(self)?;
        self.validation.clear();
        self.fault.clear();
        self.failure = None;
        self.playback = Playback::Idle;
        Ok(self.issue(Kind::FrameAtTime {
            source_id: self.source_id.clone(),
            at,
        }))
    }

    pub fn accept_follow(&mut self, ticket: &Ticket, follow: LiveHandle) -> bool {
        if !self.matches(ticket) || follow.source_id != self.source_id {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault.clear();
        self.failure = None;
        self.live = Some(follow);
        self.selection = Selection::Live;
        if self.playback != Playback::Paused {
            self.playback = Playback::Connecting;
        }
        true
    }

    pub fn accept_clip(&mut self, ticket: &Ticket, clip: ClipHandle) -> bool {
        if !self.matches(ticket) || clip.source_id != self.source_id {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault.clear();
        self.failure = None;
        self.clip = Some(clip);
        if self.playback != Playback::Paused {
            self.playback = Playback::Connecting;
        }
        true
    }

    pub fn accept_frame(&mut self, ticket: &Ticket, frame: FrameAtTime) -> bool {
        if !self.matches(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault.clear();
        self.failure = None;
        self.frame_time = frame.captured_at.clone();
        self.frame = Some(frame);
        true
    }

    pub fn apply_failure(&mut self, ticket: &Ticket, failure: Failure) -> bool {
        if !self.matches(ticket) {
            return false;
        }
        settle_failure(self, failure);
        true
    }

    pub fn reject(&mut self, ticket: &Ticket, message: &str) -> bool {
        if !self.matches(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.playback = Playback::Stopped;
        self.failure = None;
        self.fault = message.to_owned();
        true
    }

    pub fn playing(&mut self, generation: u32) -> bool {
        if self.generation != generation {
            return false;
        }
        if self.playback != Playback::Paused {
            self.playback = Playback::Playing;
        }
        true
    }

    pub fn pause(&mut self) -> bool {
        if !matches!(self.playback, Playback::Playing | Playback::Connecting) {
            return false;
        }
        self.playback = Playback::Paused;
        true
    }

    pub fn resume(&mut self) -> bool {
        if self.playback != Playback::Paused {
            return false;
        }
        self.playback = Playback::Playing;
        true
    }

    pub fn note_frame(&mut self, generation: u32, captured_at: &str) -> bool {
        if self.generation != generation {
            return false;
        }
        self.frame_time = captured_at.to_owned();
        if self.playback_note == "Skipped gap." {
            self.playback_note.clear();
        }
        true
    }

    pub fn note_gap_skipped(&mut self, generation: u32) -> bool {
        if self.generation != generation {
            return false;
        }
        self.playback_note = "Skipped gap.".to_owned();
        true
    }

    pub fn finish(&mut self, generation: u32) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        self.expected = None;
        self.playback = Playback::Stopped;
        true
    }

    pub fn fail_stream(&mut self, generation: u32, message: &str) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        self.expected = None;
        self.playback = Playback::Stopped;
        self.failure = None;
        self.fault = message.to_owned();
        true
    }

    pub fn fail_media(&mut self, generation: u32, body: &str) -> bool {
        if self.generation != generation {
            return false;
        }
        let Some(failure) = stable_failure(body) else {
            return false;
        };
        if failure == Failure::RecordingGap
            && matches!(
                self.playback,
                Playback::Connecting | Playback::Playing | Playback::Paused
            )
        {
            self.playback_note = "Skipped gap.".to_owned();
            return true;
        }
        settle_failure(self, failure);
        true
    }

    pub fn stop(&mut self) {
        self.bump();
        self.loading = false;
        self.expected = None;
        self.playback = Playback::Stopped;
        self.fault.clear();
        self.failure = None;
        self.playback_note.clear();
    }

    pub fn retry(&mut self) -> Option<Ticket> {
        if !self.can_retry() {
            return None;
        }
        let kind = self.last.clone()?;
        self.fault.clear();
        self.validation.clear();
        self.failure = None;
        Some(self.issue(kind))
    }

    pub fn reselect(&mut self) -> bool {
        if !self.can_reselect() {
            return false;
        }
        let source_id = self.source_id.clone();
        let timeline = self.timeline.clone();
        let capabilities = self.capabilities.clone();
        let catalog = self.catalog.clone();
        let open = self.details_open;
        self.enter_empty();
        self.source_id = source_id;
        self.timeline = timeline;
        self.capabilities = capabilities;
        self.catalog = catalog;
        self.details_open = open;
        true
    }

    pub fn can_retry(&self) -> bool {
        if self.last.is_none() {
            return false;
        }
        match self.failure {
            Some(failure) => failure.allows_retry(),
            None => !self.fault.is_empty(),
        }
    }

    pub fn can_reselect(&self) -> bool {
        let Some(failure) = self.failure else {
            return false;
        };
        if self.source_id.is_empty() || !failure.allows_reselect() {
            return false;
        }
        if failure == Failure::LiveUnavailable {
            return self
                .timeline
                .as_ref()
                .is_some_and(|timeline| !timeline.intervals.is_empty());
        }
        true
    }

    pub fn can_return_to_newest(&self) -> bool {
        !self.source_id.is_empty()
            && self
                .timeline
                .as_ref()
                .is_some_and(|timeline| timeline.live_available || timeline.newest_at.is_some())
    }

    pub fn has_media(&self) -> bool {
        if self.clip.is_some() || self.live.is_some() || self.frame.is_some() {
            return true;
        }
        self.playback == Playback::Connecting
            && matches!(self.selection, Selection::Live | Selection::Recorded { .. })
    }

    pub fn set_details_open(&mut self, open: bool) {
        self.details_open = open;
    }

    pub fn details(&self) -> Details {
        let Some((start, end)) = video_range(self) else {
            return Details {
                open: false,
                facts: Vec::new(),
            };
        };
        Details {
            open: true,
            facts: vec![fact("Start", start), fact("End", end)],
        }
    }

    fn request_follow(&mut self) -> Result<Ticket, &'static str> {
        need_source(self)?;
        if self
            .timeline
            .as_ref()
            .is_some_and(|timeline| !timeline.live_available && timeline.newest_at.is_none())
        {
            self.validation = "There is no newest point.".to_owned();
            return Err("There is no newest point.");
        }
        self.validation.clear();
        prepare_media(self, Playback::Connecting);
        self.selection = Selection::Live;
        Ok(self.issue(Kind::Follow {
            source_id: self.source_id.clone(),
        }))
    }

    fn matches(&self, ticket: &Ticket) -> bool {
        self.expected.as_ref() == Some(ticket)
    }

    fn issue(&mut self, kind: Kind) -> Ticket {
        self.bump();
        self.loading = true;
        self.last = Some(kind.clone());
        let ticket = Ticket {
            generation: self.generation,
            kind,
        };
        self.expected = Some(ticket.clone());
        ticket
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn enter_empty(&mut self) {
        self.bump();
        self.playback = Playback::Idle;
        self.selection = Selection::Empty;
        self.timeline = None;
        self.live = None;
        self.clip = None;
        self.frame = None;
        self.frame_time.clear();
        self.failure = None;
        self.loading = false;
        self.expected = None;
        self.last = None;
        self.fault.clear();
        self.validation.clear();
        self.playback_note.clear();
    }

    fn clear(&mut self) {
        self.enter_empty();
        self.source_id.clear();
        self.details_open = false;
        self.capabilities.clear();
    }
}

fn prepare_media(reader: &mut Reader, playback: Playback) {
    reader.fault.clear();
    reader.failure = None;
    reader.playback_note.clear();
    reader.clip = None;
    reader.live = None;
    reader.frame = None;
    reader.frame_time.clear();
    reader.playback = playback;
}

fn note_fault(reader: &mut Reader, fault: RangeFault) -> &'static str {
    reader.failure = fault.failure();
    reader.fault.clear();
    reader.validation = if fault.failure().is_some() {
        String::new()
    } else {
        fault.message().to_owned()
    };
    fault.message()
}

fn need_source(reader: &mut Reader) -> Result<(), &'static str> {
    if !reader.source_id.is_empty() {
        return Ok(());
    }
    reader.validation = "Select a video source.".to_owned();
    Err("Select a video source.")
}

fn settle_failure(reader: &mut Reader, failure: Failure) {
    reader.expected = None;
    reader.loading = false;
    reader.playback = Playback::Stopped;
    reader.failure = Some(failure);
    reader.fault.clear();
    reader.validation.clear();
}

pub(crate) fn video_range(reader: &Reader) -> Option<(String, String)> {
    if let Some(clip) = &reader.clip {
        return Some((clip.effective_start.clone(), clip.effective_end.clone()));
    }
    if let Selection::Recorded { start, end, .. } = &reader.selection {
        return Some((start.clone(), end.clone()));
    }
    None
}

fn fact(label: &'static str, value: impl Into<String>) -> DetailFact {
    DetailFact {
        label,
        value: value.into(),
    }
}

pub fn source_phase(kind: crate::inventory::Kind, count: usize) -> SourcePhase {
    if kind == crate::inventory::Kind::Unavailable {
        SourcePhase::None
    } else {
        crate::images::source_phase(kind, count)
    }
}

pub fn source_copy(phase: SourcePhase) -> &'static str {
    match phase {
        SourcePhase::Loading => "Loading video sources.",
        SourcePhase::None => "No video sources.",
        SourcePhase::Error => "Video sources could not be read.",
        SourcePhase::Ready => "",
    }
}

pub fn live_button_label(_reader: &Reader) -> &'static str {
    "LIVE"
}

pub fn shows_live_button(reader: &Reader) -> bool {
    reader
        .timeline
        .as_ref()
        .is_some_and(|timeline| timeline.live_available)
}

pub fn begin_auto_playback(reader: &mut Reader) -> Option<AutoPlayback> {
    if reader.selection != Selection::Empty {
        return None;
    }
    let timeline = reader.timeline.clone()?;
    if timeline.live_available {
        return reader.follow_newest().ok().map(AutoPlayback::Follow);
    }
    if timeline.intervals.is_empty() {
        return None;
    }
    let track = coverage_track(&timeline);
    let window = newest_retained_window(&track, timeline.newest_us)?;
    let start = format_utc(window.start_us);
    let end = format_utc(window.end_us);
    reader
        .select_range(&start, &end)
        .ok()
        .map(AutoPlayback::Clip)
}

pub fn playback_status(reader: &Reader) -> String {
    match reader.playback {
        Playback::Paused => "Paused.".to_owned(),
        Playback::Stopped => "Stopped.".to_owned(),
        Playback::Idle if reader.playback_note.is_empty() => String::new(),
        Playback::Connecting if reader.playback_note.is_empty() => "Connecting.".to_owned(),
        Playback::Playing if reader.playback_note.is_empty() => "Playing.".to_owned(),
        Playback::Idle | Playback::Connecting | Playback::Playing => reader.playback_note.clone(),
    }
}

pub fn clip_range(start: &str, end: &str) -> Result<(String, String), &'static str> {
    check_range(start, end)
        .map(|range| (range.start, range.end))
        .map_err(|fault| fault.message())
}

#[cfg(test)]
mod domain_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Reply;
    use crate::inventory::Kind as InventoryKind;
    use serde_json::{Value, json};

    #[test]
    fn clip_range_requires_utc_bounds() {
        assert_eq!(clip_range("", "").unwrap_err(), "Enter a start and an end.");
        let (start, end) = clip_range("2024-01-01T00:00:00", "2024-01-01T00:01:01").unwrap();
        assert!(start.ends_with('Z'));
        assert!(end.ends_with('Z'));
        let hour = clip_range("2024-01-01T00:00:00", "2024-01-01T01:00:00").unwrap();
        assert!(hour.1.ends_with('Z'));
        let command = clip_command("cam", &start, &end);
        assert_eq!(command["operation"], "get_video_clip");
        assert!(command["params"].get("data").is_none());
    }

    #[test]
    fn unconfigured_provider_looks_like_no_sources() {
        assert_eq!(
            source_phase(InventoryKind::Unavailable, 0),
            SourcePhase::None
        );
        assert_eq!(source_copy(SourcePhase::None), "No video sources.");
        assert_eq!(
            source_phase(InventoryKind::Transport, 0),
            SourcePhase::Error
        );
    }

    #[test]
    fn stale_tickets_do_not_replace_playback() {
        let mut reader = Reader::new();
        reader.note_catalog(source_phase(InventoryKind::Populated, 1), &["cam".into()]);
        let first = reader
            .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
            .unwrap();
        let second = reader
            .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:03")
            .unwrap();
        assert!(!reader.accept_clip(&first, sample_clip("clip-1")));
        assert!(reader.accept_clip(&second, sample_clip("clip-2")));
        assert_eq!(reader.clip.as_ref().unwrap().id, "clip-2");
        reader.stop();
        assert!(!reader.note_frame(second.generation, "late"));
        assert_eq!(reader.playback, Playback::Stopped);
    }

    #[test]
    fn clip_json_rejects_embedded_bytes_and_non_relative_paths() {
        let ok = interpret_clip(
            &clip_reply(sample_clip_json("clip-1", "/media/video-clips/clip-1/")),
            None,
        )
        .unwrap();
        assert_eq!(ok.path, "/media/video-clips/clip-1/");
        let with_data = json!({
            "operation": "get_video_clip",
            "result": {
                "id": "clip-1",
                "source_id": "cam",
                "path": "/media/video-clips/clip-1/",
                "requested_range": range(),
                "effective_range": range(),
                "media_type": "multipart/x-mixed-replace; type=image/jpeg",
                "data": "AAAA"
            }
        });
        assert!(interpret_clip(&page_reply(with_data), None).is_err());
        let query = interpret_clip(
            &clip_reply(sample_clip_json(
                "clip-1",
                "/media/video-clips/clip-1/?token=secret",
            )),
            None,
        );
        assert!(query.is_err());
    }

    fn sample_clip(id: &str) -> ClipHandle {
        ClipHandle {
            id: id.to_owned(),
            source_id: "cam".to_owned(),
            path: format!("/media/video-clips/{id}/"),
            media_type: MJPEG_MEDIA_TYPE.to_owned(),
            playback_8x: Playback8xMetadata::absent(),
            requested_start: "2024-01-01T00:00:00Z".to_owned(),
            requested_end: "2024-01-01T00:00:02Z".to_owned(),
            effective_start: "2024-01-01T00:00:00Z".to_owned(),
            effective_end: "2024-01-01T00:00:02Z".to_owned(),
        }
    }

    fn sample_clip_json(id: &str, path: &str) -> Value {
        json!({
            "id": id,
            "source_id": "cam",
            "requested_range": range(),
            "effective_range": range(),
            "media_type": "multipart/x-mixed-replace; type=image/jpeg",
            "path": path
        })
    }

    fn range() -> Value {
        json!({"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:02Z"})
    }

    fn clip_reply(result: Value) -> Reply {
        page_reply(json!({"operation": "get_video_clip", "result": result}))
    }

    fn page_reply(data: Value) -> Reply {
        Reply {
            status: 200,
            body: json!({
                "task": {
                    "id": "protocol-task",
                    "contextId": "protocol-context",
                    "status": {"state": "TASK_STATE_COMPLETED"},
                    "artifacts": [{
                        "artifactId": "art-0",
                        "parts": [{"data": data, "mediaType": "application/json"}]
                    }]
                }
            })
            .to_string(),
        }
    }
}
