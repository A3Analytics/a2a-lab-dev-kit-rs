use super::*;
use crate::codec::Reply;
use crate::inventory::Kind as InventoryKind;
use crate::videos::track;
use serde_json::{Value, json};

#[test]
fn parses_timeline_and_live_follow_without_media_bytes() {
    let timeline = interpret_timeline(&reply(timeline_body(true)), None).unwrap();
    assert_eq!(timeline.intervals.len(), 2);
    assert!(timeline.live_available);
    assert!(timeline.newest_at.is_some());
    assert_eq!(timeline_command("cam")["operation"], "get_video_timeline");
    let follow = interpret_follow(&reply(follow_body("/media/video-live/live-1/")), None).unwrap();
    assert_eq!(follow.path, "/media/video-live/live-1/");
    assert_eq!(follow_command("cam")["operation"], "follow_video");
    assert!(interpret_timeline(&reply(with_bytes(timeline_body(true))), None).is_err());
    assert!(
        interpret_follow(
            &reply(follow_body("/media/video-live/live-1/?token=secret")),
            None
        )
        .is_err()
    );
    assert!(
        interpret_follow(
            &reply(follow_body("https://cdn/media/video-live/live-1")),
            None
        )
        .is_err()
    );
}

#[test]
fn rejects_malformed_intervals_and_capability_metadata() {
    assert!(interpret_timeline(&reply(gapped_overlap()), None).is_err());
    assert!(interpret_timeline(&reply(newest_outside()), None).is_err());
    assert!(interpret_timeline(&reply(live_flag_string()), None).is_err());
    let empty = interpret_timeline(&reply(empty_timeline()), None).unwrap();
    assert!(empty.intervals.is_empty());
    assert!(empty.newest_at.is_none());
    let capability = parse_source_capability(&json!({
        "id": "cam",
        "name": "Camera A",
        "width": 640,
        "height": 360,
        "fps": 30,
        "media_type": MJPEG_MEDIA_TYPE
    }))
    .unwrap();
    assert_eq!(capability.fps, 30);
    assert!(
        parse_source_capability(&json!({
            "id": "cam",
            "name": "Camera A",
            "width": 0,
            "height": 360,
            "fps": 30
        }))
        .is_err()
    );
    assert!(
        parse_source_capability(&json!({
            "id": "cam",
            "name": "Camera A",
            "width": "640",
            "height": 360,
            "fps": 30
        }))
        .is_err()
    );
    assert!(
        parse_source_capability(&json!({
            "id": "cam",
            "name": "Camera A",
            "width": 640,
            "height": 360,
            "fps": 30,
            "data": "AAAA"
        }))
        .is_err()
    );
}

#[test]
fn range_selection_spans_gaps_when_retained_coverage_overlaps() {
    let mut reader = ready(&["cam"]);
    accept(&mut reader, continuous_timeline());
    let spanning = reader
        .select_range("2024-01-01T00:00:20", "2024-01-01T00:00:40")
        .unwrap();
    assert!(matches!(spanning.kind, Kind::Clip { .. }));
    assert!(matches!(reader.selection, Selection::Recorded { .. }));
    assert_eq!(clip_command("cam", "a", "b")["operation"], "get_video_clip");

    let mut reader = ready(&["cam"]);
    accept(&mut reader, gapped_timeline());
    let gapped = reader
        .select_range("2024-01-01T00:00:05", "2024-01-01T00:00:25")
        .unwrap();
    assert!(matches!(gapped.kind, Kind::Clip { .. }));
    assert!(reader.failure.is_none());
    assert!(reader.validation.is_empty());
    let timeline = reader.timeline.as_ref().unwrap();
    let overlap_start = crate::videos::mjpeg::captured_us("2024-01-01T00:00:05Z").unwrap();
    let overlap_end = crate::videos::mjpeg::captured_us("2024-01-01T00:00:25Z").unwrap();
    assert!(check_coverage(timeline, overlap_start, overlap_end).is_ok());

    let miss = reader
        .select_range("2024-01-01T00:00:12", "2024-01-01T00:00:15")
        .unwrap_err();
    assert_eq!(miss, "no_recording");
    assert_eq!(reader.failure, Some(Failure::NoRecording));
    assert!(reader.validation.is_empty());
    assert_eq!(
        reader.failure.map(Failure::message),
        Some("No recording covers the requested range.")
    );
    assert!(!reader.can_retry());

    let clip = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
        .unwrap();
    assert!(reader.accept_clip(&clip, sample_clip("clip-gap")));
    assert!(reader.playing(clip.generation));
    assert!(reader.fail_media(clip.generation, &media_error("recording_gap", "ignored")));
    assert_eq!(reader.failure, None);
    assert_eq!(reader.playback_note, "Skipped gap.");
    assert_eq!(reader.playback, Playback::Playing);
    assert!(reader.note_gap_skipped(clip.generation));
    assert_eq!(playback_status(&reader), "Skipped gap.");
    assert!(reader.note_frame(clip.generation, "2024-01-01T00:00:01Z"));
    assert_eq!(playback_status(&reader), "Playing.");
    assert!(reader.note_gap_skipped(clip.generation));
    assert_eq!(playback_status(&reader), "Skipped gap.");
    assert!(reader.pause());
    assert_eq!(playback_status(&reader), "Paused.");
    assert!(reader.fault.is_empty());
    assert!(reader.failure.is_none());
}

#[test]
fn empty_timeline_is_success_and_hour_plus_ranges_validate() {
    let mut reader = ready(&["cam"]);
    let ticket = reader.load_timeline().unwrap();
    let empty = interpret_timeline(&reply(empty_timeline()), None).unwrap();
    assert!(reader.accept_timeline(&ticket, empty));
    assert!(reader.failure.is_none());
    assert_eq!(
        reader
            .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:01")
            .unwrap_err(),
        "no_recording"
    );

    let mut reader = ready(&["cam"]);
    accept(&mut reader, long_timeline());
    let hour = check_range("2024-01-01T00:00:00", "2024-01-01T00:01:00").unwrap();
    assert_eq!(hour.end_us - hour.start_us, 60_000_000);
    assert!(
        reader
            .select_range("2024-01-01T00:00:00", "2024-01-01T00:01:00")
            .is_ok()
    );
    let longer = check_range("2024-01-01T00:00:00", "2024-01-01T00:01:01").unwrap();
    assert_eq!(longer.end_us - longer.start_us, 61_000_000);
    assert!(
        reader
            .select_range("2024-01-01T00:00:00", "2024-01-01T00:01:01")
            .is_ok()
    );
}

#[test]
fn live_edge_transitions_drop_stale_generations_and_sources() {
    let mut reader = ready(&["cam", "other"]);
    accept(&mut reader, gapped_timeline());
    assert!(reader.can_return_to_newest());
    let recorded = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
        .unwrap();
    let live = reader.return_to_newest().unwrap();
    assert!(matches!(reader.selection, Selection::Live));
    assert!(matches!(live.kind, Kind::Follow { .. }));
    assert!(!reader.accept_clip(&recorded, sample_clip("clip-1")));
    assert!(reader.accept_follow(&live, sample_follow()));
    assert!(reader.has_media());
    let again = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
        .unwrap();
    assert!(reader.live.is_none());
    assert!(!reader.note_frame(live.generation, "late"));
    reader.select_source("other");
    assert!(reader.timeline.is_none());
    assert!(matches!(reader.selection, Selection::Empty));
    assert!(!reader.accept_clip(&again, sample_clip("clip-2")));
    assert_eq!(reader.playback, Playback::Idle);
}

#[test]
fn stable_codes_drive_retry_and_reselection_without_message_text() {
    assert_eq!(
        stable_failure(&a2a_error("live_unavailable", "no_recording: human text")),
        Some(Failure::LiveUnavailable)
    );
    assert_eq!(
        stable_failure(&media_error("clip_expired", "recording gap in the message")),
        Some(Failure::ClipExpired)
    );
    assert_eq!(
        stable_failure(&media_error("no_recording", "ignored")),
        Some(Failure::NoRecording)
    );
    assert_eq!(
        stable_failure(&media_error("recording_gap", "ignored")),
        Some(Failure::RecordingGap)
    );
    assert_eq!(
        stable_failure(&media_error("invalid", "ignored")),
        Some(Failure::Invalid)
    );
    assert_eq!(
        stable_failure(&media_error("not_found", "ignored")),
        Some(Failure::NotFound)
    );
    assert_eq!(
        stable_failure(&media_error("unavailable", "ignored")),
        Some(Failure::Unavailable)
    );
    assert!(stable_failure(&reason_only("no_recording is in the message")).is_none());

    let mut reader = ready(&["cam"]);
    accept(&mut reader, gapped_timeline());
    let clip = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
        .unwrap();
    assert!(reader.accept_clip(&clip, sample_clip("clip-1")));
    assert!(reader.fail_media(
        clip.generation,
        &media_error("clip_expired", "please parse me")
    ));
    assert_eq!(reader.failure, Some(Failure::ClipExpired));
    assert!(reader.fault.is_empty());
    assert_eq!(
        reader.failure.map(Failure::message),
        Some("The clip expired.")
    );
    assert!(reader.can_retry());
    let retried = reader.retry().unwrap();
    assert!(matches!(retried.kind, Kind::Clip { .. }));
    assert_eq!(reader.source_id, "cam");
    assert!(!reader.fail_media(clip.generation, &media_error("clip_expired", "stale")));

    let follow = reader.follow_newest().unwrap();
    assert!(reader.apply_failure(&follow, Failure::LiveUnavailable));
    assert!(reader.fault.is_empty());
    assert!(reader.validation.is_empty());
    assert_eq!(
        reader.failure.map(Failure::message),
        Some("Live video is unavailable.")
    );
    assert!(reader.can_retry());
    assert!(reader.can_reselect());
    let again = reader.retry().unwrap();
    assert!(matches!(again.kind, Kind::Follow { .. }));
    assert!(!reader.apply_failure(&follow, Failure::LiveUnavailable));

    let mut bare = ready(&["cam"]);
    let live = bare.follow_newest().unwrap();
    assert!(bare.apply_failure(&live, Failure::LiveUnavailable));
    assert!(bare.can_retry());
    assert!(!bare.can_reselect());
    let retry = bare.retry().unwrap();
    assert_eq!(bare.source_id, "cam");
    assert!(matches!(retry.kind, Kind::Follow { .. }));
}

#[test]
fn frame_at_time_keeps_the_public_operation_and_details_start_closed() {
    assert_eq!(frame_at_time_label(), "Seek");
    assert!(!frame_at_time_label().to_ascii_lowercase().contains("still"));
    let at = frame_instant("2024-01-01T00:00:00").unwrap();
    let command = frame_at_time_command("cam", &at);
    assert_eq!(command["operation"], "get_video_frame_at");
    assert!(command["params"].get("image").is_none());
    let frame = interpret_frame_at_time(&reply(frame_body()), None).unwrap();
    assert_eq!(frame.captured_at, "2024-01-01T00:00:00Z");

    let mut reader = ready(&["cam"]);
    assert!(!reader.details().open);
    assert!(!reader.has_media());
    assert!(reader.details().facts.is_empty());
    reader.note_capability(SourceCapability {
        id: "cam".to_owned(),
        name: "Camera A".to_owned(),
        width: 640,
        height: 360,
        fps: 30,
    });
    accept(&mut reader, gapped_timeline());
    let ticket = reader.frame_at_time("2024-01-01T00:00:00").unwrap();
    assert!(matches!(ticket.kind, Kind::FrameAtTime { .. }));
    assert!(reader.accept_frame(&ticket, frame));
    assert!(!reader.details().open);
    assert!(reader.details().facts.is_empty());
}

#[test]
fn auto_playback_starts_live_or_newest_retained_clip() {
    let mut reader = ready(&["cam"]);
    accept(&mut reader, continuous_timeline());
    let live = begin_auto_playback(&mut reader).unwrap();
    assert!(matches!(live, AutoPlayback::Follow(_)));
    assert!(matches!(reader.selection, Selection::Live));

    let mut reader = ready(&["cam"]);
    accept(&mut reader, long_timeline());
    let clip = begin_auto_playback(&mut reader).unwrap();
    assert!(matches!(clip, AutoPlayback::Clip(_)));
    assert!(matches!(reader.selection, Selection::Recorded { .. }));

    let mut reader = ready(&["cam"]);
    let ticket = reader.load_timeline().unwrap();
    let empty = interpret_timeline(&reply(empty_timeline()), None).unwrap();
    assert!(reader.accept_timeline(&ticket, empty));
    assert!(begin_auto_playback(&mut reader).is_none());
    assert!(matches!(reader.selection, Selection::Empty));
}

#[test]
fn playhead_percent_tracks_frame_time() {
    use crate::videos::track::{coverage_track, playhead_percent};

    let timeline = interpret_timeline(&reply(gapped_timeline()), None).unwrap();
    let track = coverage_track(&timeline);
    let early = playhead_percent(&track, "2024-01-01T00:00:05Z").unwrap();
    let later = playhead_percent(&track, "2024-01-01T00:00:25Z").unwrap();
    assert!(later > early);
    assert_eq!(live_button_label(&Reader::new()), "LIVE");
}

#[test]
fn live_action_enables_only_for_a_livestream() {
    let mut archive = ready(&["cam"]);
    let archive_ticket = archive.load_timeline().unwrap();
    let archived = interpret_timeline(&reply(timeline_body(false)), None).unwrap();
    assert!(!archived.live_available);
    assert!(archive.accept_timeline(&archive_ticket, archived));
    assert!(!shows_live_button(&archive));

    let mut live = ready(&["cam"]);
    let live_ticket = live.load_timeline().unwrap();
    let livestream = interpret_timeline(&reply(timeline_body(true)), None).unwrap();
    assert!(livestream.live_available);
    assert!(live.accept_timeline(&live_ticket, livestream));
    assert!(shows_live_button(&live));
}

#[test]
fn failure_sentences_stay_distinct_from_codes_and_saved_stills() {
    let failures = [
        Failure::NoRecording,
        Failure::RecordingGap,
        Failure::ClipExpired,
        Failure::LiveUnavailable,
        Failure::Invalid,
        Failure::NotFound,
        Failure::Unavailable,
    ];
    let mut seen = Vec::new();
    for failure in failures {
        let sentence = failure.message();
        assert_ne!(sentence, failure.code());
        assert!(!sentence.to_ascii_lowercase().contains("still"));
        assert!(!seen.contains(&sentence));
        seen.push(sentence);
    }
    assert_eq!(Failure::Invalid.message(), "The video request is invalid.");
    assert_eq!(
        Failure::NotFound.message(),
        "The requested video was not found."
    );
    assert_eq!(Failure::Unavailable.message(), "Video is unavailable.");
}

#[test]
fn details_omit_facts_that_are_not_available() {
    let mut reader = ready(&["cam"]);
    accept(&mut reader, gapped_timeline());
    assert!(labels_of(&reader).is_empty());

    let clip = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T00:00:02")
        .unwrap();
    assert_eq!(labels_of(&reader), ["Start", "End"]);
    assert!(reader.accept_clip(&clip, sample_clip("clip-1")));
    assert!(reader.playing(clip.generation));
    let facts = reader.details().facts;
    assert_eq!(
        facts.iter().map(|item| item.label).collect::<Vec<_>>(),
        ["Start", "End"]
    );
    assert_eq!(facts[0].value, "2024-01-01T00:00:00Z");
    assert_eq!(facts[1].value, "2024-01-01T00:00:02Z");
    assert!(facts.iter().all(|item| item.label != "Image"));
}

fn labels_of(reader: &Reader) -> Vec<&str> {
    reader
        .details()
        .facts
        .iter()
        .map(|item| item.label)
        .collect()
}

#[test]
fn selection_is_one_timeline_model() {
    let selection = Selection::Empty;
    match selection {
        Selection::Empty | Selection::Live | Selection::Recorded { .. } => {}
    }
    assert_eq!(MEDIA_CACHE_CONTROL, "no-store");
    assert_eq!(CAPTURED_AT_HEADER, "X-Captured-At");
    assert_eq!(LIVE_PATH_PREFIX, "/media/video-live/");
    assert_eq!(CLIP_PATH_PREFIX, "/media/video-clips/");
    let range: CheckedRange = check_range("2024-01-01T00:00:00", "2024-01-01T00:00:01").unwrap();
    let intervals: &[Interval] = &[];
    assert!(range.end_us > range.start_us);
    assert!(intervals.is_empty());
}

fn ready(ids: &[&str]) -> Reader {
    let mut reader = Reader::new();
    let owned: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    reader.note_catalog(source_phase(InventoryKind::Populated, owned.len()), &owned);
    assert_eq!(reader.playback, Playback::Idle);
    reader
}

fn accept(reader: &mut Reader, body: Value) {
    let ticket = reader.load_timeline().unwrap();
    let timeline = interpret_timeline(&reply(body), None).unwrap();
    assert!(reader.accept_timeline(&ticket, timeline));
    assert!(reader.failure.is_none());
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

fn sample_follow() -> LiveHandle {
    LiveHandle {
        id: "live-1".to_owned(),
        source_id: "cam".to_owned(),
        path: "/media/video-live/live-1/".to_owned(),
        media_type: MJPEG_MEDIA_TYPE.to_owned(),
        playback_8x: Playback8xMetadata::absent(),
    }
}

fn reply(data: Value) -> Reply {
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

fn timeline_body(live: bool) -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:30Z"},
                {"start": "2024-01-01T00:00:30Z", "end": "2024-01-01T00:01:00Z"}
            ],
            "newest_at": "2024-01-01T00:00:59Z",
            "live_available": live
        }
    })
}

fn continuous_timeline() -> Value {
    timeline_body(true)
}

fn gapped_timeline() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:10Z"},
                {"start": "2024-01-01T00:00:20Z", "end": "2024-01-01T00:00:40Z"}
            ],
            "newest_at": "2024-01-01T00:00:30Z",
            "live_available": true
        }
    })
}

#[test]
fn dvr_timeline_accepts_long_selections_and_rebases_sessions() {
    use crate::videos::track::{
        PLAYBACK_WINDOW_US, Window, coverage_track, drag_window, extend_window, move_window,
        needs_session_rebase, newest_retained_window, session_in_selection,
    };

    let timeline = interpret_timeline(&reply(three_hour_timeline()), None).unwrap();
    let track = coverage_track(&timeline);
    let dragged = drag_window(
        &track,
        stamp("2024-01-01T00:00:00Z"),
        stamp("2024-01-01T00:01:30Z"),
    )
    .unwrap();
    assert_eq!(dragged.duration_us(), 90_000_000);

    let mut reader = ready(&["cam"]);
    accept(&mut reader, three_hour_timeline());
    assert!(matches!(
        begin_auto_playback(&mut reader),
        Some(AutoPlayback::Clip(_))
    ));
    assert!(matches!(reader.selection, Selection::Recorded { .. }));

    let selection = Window {
        start_us: stamp("2024-01-01T00:00:00Z"),
        end_us: stamp("2024-01-01T00:02:30Z"),
    };
    let first = session_in_selection(&track, selection, selection.start_us).unwrap();
    assert!(first.duration_us() <= PLAYBACK_WINDOW_US);
    let later = session_in_selection(&track, selection, stamp("2024-01-01T00:02:20Z")).unwrap();
    assert!(needs_session_rebase(first, stamp("2024-01-01T00:02:20Z")));
    assert!(later.start_us > first.start_us);

    let draft = newest_retained_window(&track, timeline.newest_us).unwrap();
    let extended = extend_window(&track, draft, 30);
    assert!(extended.duration_us() <= PLAYBACK_WINDOW_US || extended.end_us > draft.end_us);
    let shifted = move_window(&track, extended, -1);
    assert!(shifted.start_us < extended.start_us || shifted.end_us <= extended.end_us);
}

fn stamp(value: &str) -> i64 {
    crate::videos::mjpeg::captured_us(value).unwrap()
}

fn three_hour_timeline() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:03:00Z"}
            ],
            "newest_at": "2024-01-01T00:02:30Z",
            "live_available": false
        }
    })
}

fn long_timeline() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:02:00Z"}
            ],
            "newest_at": "2024-01-01T00:01:30Z",
            "live_available": false
        }
    })
}

fn empty_timeline() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [],
            "live_available": false
        }
    })
}

fn gapped_overlap() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:20Z"},
                {"start": "2024-01-01T00:00:10Z", "end": "2024-01-01T00:00:30Z"}
            ],
            "live_available": false
        }
    })
}

fn newest_outside() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [
                {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:10Z"}
            ],
            "newest_at": "2024-01-01T00:00:30Z",
            "live_available": false
        }
    })
}

fn live_flag_string() -> Value {
    json!({
        "operation": "get_video_timeline",
        "result": {
            "source_id": "cam",
            "intervals": [],
            "live_available": "yes"
        }
    })
}

fn follow_body(path: &str) -> Value {
    json!({
        "operation": "follow_video",
        "result": {
            "id": "live-1",
            "source_id": "cam",
            "media_type": MJPEG_MEDIA_TYPE,
            "path": path
        }
    })
}

#[test]
fn encoded_follow_parses_playback_8x_metadata() {
    let body = json!({
        "operation": "follow_video",
        "result": {
            "id": "live-1",
            "source_id": "enc",
            "media_type": "application/vnd.apple.mpegurl",
            "path": "/media/video-live/live-1/",
            "playback_8x": {
                "rendition_path": "index-8x.m3u8",
                "fallback_path": "index.m3u8"
            }
        }
    });
    let follow = interpret_follow(&reply(body), None).unwrap();
    assert_eq!(
        follow.playback_8x.rendition_path.as_deref(),
        Some("index-8x.m3u8")
    );
    let plan = plan_stream(
        PlaybackSpeed::Eight,
        &follow.path,
        &follow.media_type,
        &follow.playback_8x,
    );
    assert!(plan.fetch_path.ends_with("index-8x.m3u8"));
}

#[test]
fn playback_speed_preserves_selection_pause_and_session_window() {
    let mut reader = ready(&["cam"]);
    accept(&mut reader, gapped_timeline());
    let ticket = reader
        .select_range("2024-01-01T00:00:00", "2024-01-01T01:00:00")
        .unwrap();
    let selection = reader.selection.clone();
    assert!(reader.accept_clip(&ticket, sample_clip("clip-1")));
    reader.pause();
    assert_eq!(reader.playback, Playback::Paused);
    for speed in PlaybackSpeed::all() {
        let plan = plan_stream(
            *speed,
            &reader.clip.as_ref().unwrap().path,
            MJPEG_MEDIA_TYPE,
            &Playback8xMetadata::absent(),
        );
        assert_eq!(plan.mjpeg_pace, speed.mjpeg_pace());
        assert_eq!(reader.selection, selection);
        assert_eq!(reader.playback, Playback::Paused);
    }
    let session = track::Window {
        start_us: 0,
        end_us: 3_600_000_000,
    };
    assert!(session.end_us > session.start_us);
}

fn with_bytes(mut body: Value) -> Value {
    body["result"]["jpeg"] = json!("AAAA");
    body
}

fn frame_body() -> Value {
    json!({
        "operation": "get_video_frame_at",
        "result": {
            "actual_captured_at": "2024-01-01T00:00:00Z",
            "image": {
                "descriptor": {
                    "captured_at": "2024-01-01T00:00:00Z",
                    "media_type": "image/jpeg"
                },
                "data": "AAAA"
            }
        }
    })
}

fn a2a_error(code: &str, message: &str) -> String {
    json!({
        "error": {
            "code": 13,
            "status": "INTERNAL",
            "message": message,
            "details": [{
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": "INTERNAL_ERROR",
                "domain": "a2a-protocol.org",
                "metadata": {"code": code}
            }]
        }
    })
    .to_string()
}

fn media_error(code: &str, message: &str) -> String {
    json!({"error": {"code": code, "message": message}}).to_string()
}

fn reason_only(message: &str) -> String {
    json!({
        "error": {
            "code": 13,
            "status": "INTERNAL",
            "message": message,
            "details": [{
                "reason": "INTERNAL_ERROR",
                "domain": "a2a-protocol.org"
            }]
        }
    })
    .to_string()
}
