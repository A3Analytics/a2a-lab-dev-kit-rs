//! Recording-availability geometry.
//!
//! Abutting retained intervals are one solid region. A later interval is a gap.
//! Selection stays inside one region and is at most 60 seconds.

use super::domain::{self, MAX_CLIP_US, Timeline};

pub const STEP_US: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Retained,
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub kind: SpanKind,
    pub start: String,
    pub end: String,
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub origin_us: i64,
    pub end_us: i64,
    pub spans: Vec<Span>,
}

impl Track {
    pub fn span_us(&self) -> i64 {
        self.end_us.saturating_sub(self.origin_us).max(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start_us: i64,
    pub end_us: i64,
}

impl Window {
    pub fn duration_us(self) -> i64 {
        self.end_us.saturating_sub(self.start_us)
    }
}

pub fn coverage_track(timeline: &Timeline) -> Track {
    let merged = domain::logical_intervals(&timeline.intervals);
    let Some(first) = merged.first() else {
        return Track {
            origin_us: 0,
            end_us: 0,
            spans: Vec::new(),
        };
    };
    let mut spans = Vec::new();
    let mut cursor = first.start_us;
    let mut cursor_label = first.start.clone();
    for interval in &merged {
        if interval.start_us > cursor {
            spans.push(span(
                SpanKind::Gap,
                cursor_label,
                interval.start.clone(),
                cursor,
                interval.start_us,
            ));
        }
        spans.push(span(
            SpanKind::Retained,
            interval.start.clone(),
            interval.end.clone(),
            interval.start_us,
            interval.end_us,
        ));
        cursor = interval.end_us;
        cursor_label = interval.end.clone();
    }
    Track {
        origin_us: first.start_us,
        end_us: cursor,
        spans,
    }
}

pub fn percent(track: &Track, at_us: i64) -> f64 {
    let span = track.span_us();
    if span <= 0 {
        return 0.0;
    }
    let delta = (at_us - track.origin_us).clamp(0, span);
    (delta as f64) * 100.0 / (span as f64)
}

pub fn initial_window(track: &Track) -> Option<Window> {
    let span = retained(track).next()?;
    window_at(track, span.start_us)
}

pub fn window_at(track: &Track, at_us: i64) -> Option<Window> {
    let span = containing(track, at_us)?;
    bounded(span, at_us, MAX_CLIP_US)
}

pub fn drag_window(track: &Track, anchor_us: i64, point_us: i64) -> Option<Window> {
    let span = containing(track, anchor_us)?;
    let mut start = anchor_us.min(point_us).max(span.start_us);
    let mut end = anchor_us.max(point_us).min(span.end_us);
    if end.saturating_sub(start) > MAX_CLIP_US {
        if point_us >= anchor_us {
            end = start.saturating_add(MAX_CLIP_US);
        } else {
            start = end.saturating_sub(MAX_CLIP_US);
        }
    }
    (end > start).then_some(Window {
        start_us: start,
        end_us: end,
    })
}

pub fn establish(track: &Track, current: Option<Window>) -> Option<Window> {
    current.or_else(|| initial_window(track))
}

pub fn move_window(track: &Track, window: Window, steps: i64) -> Window {
    let Some(span) = containing(track, window.start_us) else {
        return window;
    };
    let duration = window.duration_us().clamp(1, MAX_CLIP_US);
    let target = window
        .start_us
        .saturating_add(steps.saturating_mul(STEP_US));
    if target >= span.start_us && target.saturating_add(duration) <= span.end_us {
        return Window {
            start_us: target,
            end_us: target.saturating_add(duration),
        };
    }
    if steps > 0 {
        if let Some(next) = retained(track).find(|item| item.start_us >= span.end_us) {
            return place(next, next.start_us, duration);
        }
        return place(span, span.end_us.saturating_sub(duration), duration);
    }
    if let Some(previous) = retained(track).rfind(|item| item.end_us <= span.start_us) {
        let start = previous
            .end_us
            .saturating_sub(duration)
            .max(previous.start_us);
        return place(previous, start, duration);
    }
    place(span, span.start_us, duration)
}

pub fn extend_window(track: &Track, window: Window, steps: i64) -> Window {
    let Some(span) = containing(track, window.start_us) else {
        return window;
    };
    let mut end = window.end_us.saturating_add(steps.saturating_mul(STEP_US));
    let max_end = window.start_us.saturating_add(MAX_CLIP_US).min(span.end_us);
    let min_end = window.start_us.saturating_add(1).min(span.end_us);
    if end > max_end {
        end = max_end;
    }
    if end < min_end {
        end = min_end;
    }
    Window {
        start_us: window.start_us,
        end_us: end,
    }
}

pub fn jump_window(track: &Track, window: Window, to_start: bool) -> Window {
    let Some(span) = containing(track, window.start_us) else {
        return window;
    };
    let duration = window.duration_us().clamp(1, MAX_CLIP_US);
    if to_start {
        place(span, span.start_us, duration)
    } else {
        place(
            span,
            span.end_us.saturating_sub(duration).max(span.start_us),
            duration,
        )
    }
}

pub fn retained_instant(track: &Track, at_us: i64) -> Option<String> {
    containing(track, at_us).map(|_| format_utc(at_us))
}

pub fn format_utc(us: i64) -> String {
    let day_us = 86_400_000_000_i64;
    let days = us.div_euclid(day_us);
    let rem = us.rem_euclid(day_us);
    let hour = rem / 3_600_000_000;
    let rem = rem % 3_600_000_000;
    let minute = rem / 60_000_000;
    let rem = rem % 60_000_000;
    let second = rem / 1_000_000;
    let micros = rem % 1_000_000;
    let Some((year, month, day)) = civil_from_days(days) else {
        return "1970-01-01T00:00:00Z".to_owned();
    };
    if micros == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{micros:06}Z")
    }
}

pub fn at_ratio(track: &Track, ratio: f64) -> Option<i64> {
    let span = track.span_us();
    if span <= 0 {
        return None;
    }
    let ratio = ratio.clamp(0.0, 1.0);
    Some(track.origin_us + (ratio * span as f64).round() as i64)
}

fn retained(track: &Track) -> impl DoubleEndedIterator<Item = &Span> {
    track
        .spans
        .iter()
        .filter(|span| span.kind == SpanKind::Retained)
}

fn containing(track: &Track, at_us: i64) -> Option<&Span> {
    retained(track).find(|span| at_us >= span.start_us && at_us < span.end_us)
}

fn bounded(span: &Span, start_us: i64, duration_us: i64) -> Option<Window> {
    let end = start_us.saturating_add(duration_us).min(span.end_us);
    (end > start_us).then_some(Window {
        start_us,
        end_us: end,
    })
}

fn place(span: &Span, start_us: i64, duration_us: i64) -> Window {
    let start = start_us.clamp(span.start_us, span.end_us.saturating_sub(1));
    let end = start
        .saturating_add(duration_us)
        .min(span.end_us)
        .max(start.saturating_add(1).min(span.end_us));
    Window {
        start_us: start,
        end_us: end,
    }
}

fn span(kind: SpanKind, start: String, end: String, start_us: i64, end_us: i64) -> Span {
    Span {
        kind,
        start,
        end,
        start_us,
        end_us,
    }
}

fn civil_from_days(days: i64) -> Option<(i32, u32, u32)> {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z.rem_euclid(146_097)).ok()?;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = i64::try_from(yoe).ok()? + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    Some((
        i32::try_from(year).ok()?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Kind as InventoryKind;
    use crate::videos::mjpeg::captured_us;
    use crate::videos::{Failure, Interval, Reader, Selection, source_phase};

    #[test]
    fn abutting_chunks_are_one_region_and_gaps_stay_open() {
        let continuous = coverage_track(&timeline(&[
            ("2024-01-01T00:00:00Z", "2024-01-01T00:00:30Z"),
            ("2024-01-01T00:00:30Z", "2024-01-01T00:01:00Z"),
        ]));
        assert_eq!(retained_count(&continuous), 1);
        assert_eq!(gap_count(&continuous), 0);
        assert_eq!(continuous.spans[0].end, "2024-01-01T00:01:00Z");
        let across = window_at(&continuous, stamp("2024-01-01T00:00:20Z")).unwrap();
        assert!(across.end_us > stamp("2024-01-01T00:00:30Z"));
        assert!(across.duration_us() <= MAX_CLIP_US);
        let mut reader = ready();
        accept_timeline(&mut reader, &continuous_body());
        let ticket = reader
            .select_range(&format_utc(across.start_us), &format_utc(across.end_us))
            .unwrap();
        assert!(matches!(ticket.kind, crate::videos::Kind::Clip { .. }));
        assert!(matches!(reader.selection, Selection::Recorded { .. }));

        let gapped = coverage_track(&timeline(&[
            ("2024-01-01T00:00:00Z", "2024-01-01T00:00:10Z"),
            ("2024-01-01T00:00:20Z", "2024-01-01T00:00:40Z"),
        ]));
        assert_eq!(retained_count(&gapped), 2);
        assert_eq!(gap_count(&gapped), 1);
        let dragged = drag_window(
            &gapped,
            stamp("2024-01-01T00:00:00Z"),
            stamp("2024-01-01T00:00:25Z"),
        )
        .unwrap();
        assert!(dragged.end_us <= stamp("2024-01-01T00:00:10Z"));
        assert!(dragged.duration_us() <= MAX_CLIP_US);
    }

    #[test]
    fn keyboard_selection_stays_inside_coverage_and_sixty_seconds() {
        let long = coverage_track(&timeline(&[(
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:02:00Z",
        )]));
        let initial = initial_window(&long).unwrap();
        assert_eq!(initial.duration_us(), MAX_CLIP_US);
        let extended = extend_window(&long, initial, 5);
        assert_eq!(extended.duration_us(), MAX_CLIP_US);
        let shifted = move_window(&long, initial, 1);
        assert_eq!(shifted.duration_us(), MAX_CLIP_US);
        assert_eq!(shifted.start_us, initial.start_us.saturating_add(STEP_US));
        assert!(retained_instant(&long, shifted.start_us).is_some());

        let gapped = coverage_track(&timeline(&[
            ("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            ("2024-01-01T00:00:10Z", "2024-01-01T00:00:12Z"),
        ]));
        let first = establish(&gapped, None).unwrap();
        let jumped = move_window(&gapped, first, 1);
        assert_eq!(jumped.start_us, stamp("2024-01-01T00:00:10Z"));
        assert!(jumped.end_us <= stamp("2024-01-01T00:00:12Z"));
        assert!(jumped.start_us >= stamp("2024-01-01T00:00:10Z"));
    }

    #[test]
    fn utc_labels_round_trip_through_range_checks() {
        for stamp in [
            "2024-01-01T00:00:00Z",
            "2024-02-29T23:59:59Z",
            "2024-01-01T00:00:00.500000Z",
        ] {
            let us = captured_us(stamp).unwrap();
            let label = format_utc(us);
            assert_eq!(captured_us(&label), Some(us), "{stamp} -> {label}");
        }
        let start = stamp("2024-01-01T00:00:00Z");
        let range = domain::check_range(&format_utc(start), &format_utc(start + STEP_US)).unwrap();
        assert_eq!(range.end_us - range.start_us, STEP_US);
    }

    fn ready() -> Reader {
        let mut reader = Reader::new();
        reader.note_catalog(source_phase(InventoryKind::Populated, 1), &["cam".into()]);
        reader
    }

    fn accept_timeline(reader: &mut Reader, body: &serde_json::Value) {
        let ticket = reader.load_timeline().unwrap();
        let timeline = domain::interpret_timeline(&reply(body), None).unwrap();
        assert!(reader.accept_timeline(&ticket, timeline));
        assert_ne!(reader.failure, Some(Failure::NoRecording));
    }

    fn continuous_body() -> serde_json::Value {
        serde_json::json!({
            "operation": "get_video_timeline",
            "result": {
                "source_id": "cam",
                "intervals": [
                    {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:30Z"},
                    {"start": "2024-01-01T00:00:30Z", "end": "2024-01-01T00:01:00Z"}
                ],
                "newest_at": "2024-01-01T00:00:59Z",
                "live_available": true
            }
        })
    }

    fn timeline(intervals: &[(&str, &str)]) -> Timeline {
        let mut owned = Vec::new();
        for (start, end) in intervals {
            owned.push(Interval {
                start: (*start).to_owned(),
                end: (*end).to_owned(),
                start_us: stamp(start),
                end_us: stamp(end),
            });
        }
        Timeline {
            source_id: "cam".to_owned(),
            intervals: owned,
            newest_at: None,
            newest_us: None,
            live_available: false,
        }
    }

    fn stamp(value: &str) -> i64 {
        captured_us(value).unwrap()
    }

    fn retained_count(track: &Track) -> usize {
        track
            .spans
            .iter()
            .filter(|span| span.kind == SpanKind::Retained)
            .count()
    }

    fn gap_count(track: &Track) -> usize {
        track
            .spans
            .iter()
            .filter(|span| span.kind == SpanKind::Gap)
            .count()
    }

    fn reply(data: &serde_json::Value) -> crate::codec::Reply {
        crate::codec::Reply {
            status: 200,
            body: serde_json::json!({
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
