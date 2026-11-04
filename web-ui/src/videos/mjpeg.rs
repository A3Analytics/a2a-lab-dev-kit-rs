//! Incremental MJPEG multipart parser. No blob URLs.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub const BOUNDARY: &str = "a2alabframe";
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FRAMES: usize = 1_800;
pub const MAX_QUEUE: usize = 3;
pub const MAX_DURATION_US: i64 = 120_000_000;
pub const GAP_THRESHOLD_US: i64 = 1_500_000;

pub fn is_capture_gap(previous_us: i64, next_us: i64) -> bool {
    next_us.saturating_sub(previous_us) > GAP_THRESHOLD_US
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub captured_at: String,
    pub captured_us: i64,
    pub jpeg: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Frame(Frame),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Headers,
    Length,
    EmptyJpeg,
    CapturedAt,
    TooManyBytes,
    TooManyFrames,
    TooLong,
}

#[derive(Debug)]
pub struct Parser {
    marker: Vec<u8>,
    buf: Vec<u8>,
    frames: usize,
    first_us: Option<i64>,
    closed: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            marker: format!("--{BOUNDARY}").into_bytes(),
            buf: Vec::new(),
            frames: 0,
            first_us: None,
            closed: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Event>, Error> {
        if self.buf.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err(Error::TooManyBytes);
        }
        self.buf.extend_from_slice(chunk);
        self.extract()
    }

    fn extract(&mut self) -> Result<Vec<Event>, Error> {
        let mut events = Vec::new();
        while !self.closed {
            match self.next_part()? {
                None => break,
                Some(Event::Done) => {
                    events.push(Event::Done);
                    break;
                }
                Some(event) => events.push(event),
            }
        }
        Ok(events)
    }

    fn next_part(&mut self) -> Result<Option<Event>, Error> {
        let Some(start) = find(&self.buf, &self.marker) else {
            return Ok(None);
        };
        if start > 0 {
            self.buf.drain(..start);
        }
        let after = self.marker.len();
        if self.buf.len() < after + 2 {
            return Ok(None);
        }
        if self.buf[after..after + 2] == *b"--" {
            self.closed = true;
            self.buf.clear();
            return Ok(Some(Event::Done));
        }
        if self.buf[after..after + 2] != *b"\r\n" {
            return Err(Error::Headers);
        }
        let header_start = after + 2;
        let Some(header_span) = find(&self.buf[header_start..], b"\r\n\r\n") else {
            return Ok(None);
        };
        let headers = std::str::from_utf8(&self.buf[header_start..header_start + header_span])
            .map_err(|_| Error::Headers)?;
        let captured = header(headers, "X-Captured-At")
            .ok_or(Error::CapturedAt)?
            .to_owned();
        let length = header(headers, "Content-Length")
            .ok_or(Error::Length)?
            .parse::<usize>()
            .map_err(|_| Error::Length)?;
        let body_start = header_start + header_span + 4;
        let body_end = body_start.checked_add(length).ok_or(Error::Length)?;
        if self.buf.len() < body_end {
            return Ok(None);
        }
        let jpeg = self.buf[body_start..body_end].to_vec();
        let mut consume = body_end;
        if self.buf.get(body_end..body_end + 2) == Some(&b"\r\n"[..]) {
            consume = body_end + 2;
        }
        self.buf.drain(..consume);
        if jpeg.is_empty() {
            return Err(Error::EmptyJpeg);
        }
        let captured_us = captured_us(&captured).ok_or(Error::CapturedAt)?;
        if self
            .first_us
            .is_some_and(|first| captured_us.saturating_sub(first) > MAX_DURATION_US)
            || self.frames >= MAX_FRAMES
        {
            self.first_us = None;
            self.frames = 0;
        }
        self.first_us.get_or_insert(captured_us);
        self.frames += 1;
        Ok(Some(Event::Frame(Frame {
            captured_at: captured,
            captured_us,
            jpeg,
        })))
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.eq_ignore_ascii_case(name)).then(|| value.trim())
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(crate) const MIN_MJPEG_RATE: f64 = 0.01;

pub fn due_wall_ms(captured_us: i64, origin_us: i64, origin_wall_ms: f64, rate: f64) -> f64 {
    let rate = rate.max(MIN_MJPEG_RATE);
    origin_wall_ms + ((captured_us - origin_us) as f64 / 1_000.0 / rate)
}

pub fn is_late(
    captured_us: i64,
    origin_us: i64,
    origin_wall_ms: f64,
    now_ms: f64,
    rate: f64,
) -> bool {
    now_ms > due_wall_ms(captured_us, origin_us, origin_wall_ms, rate)
}

pub fn captured_us(value: &str) -> Option<i64> {
    let body = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .or_else(|| value.strip_suffix("-00:00"))?;
    if body.contains('+') {
        return None;
    }
    let (date, time) = body.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.split(':');
    let hour: u32 = clock_parts.next()?.parse().ok()?;
    let minute: u32 = clock_parts.next()?.parse().ok()?;
    let second: u32 = clock_parts.next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let nanos: u32 = match fraction {
        None => 0,
        Some(fraction)
            if !fraction.is_empty()
                && fraction.len() <= 9
                && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            format!("{fraction:0<9}").parse().ok()?
        }
        Some(_) => return None,
    };
    let days = days_from_civil(year, month, day)?;
    Some(
        days * 86_400_000_000
            + i64::from(hour) * 3_600_000_000
            + i64::from(minute) * 60_000_000
            + i64::from(second) * 1_000_000
            + i64::from(nanos / 1_000),
    )
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = i64::from(year.div_euclid(400));
    let yoe = year.rem_euclid(400) as u32;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + i64::from(doe) - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];

    #[test]
    fn parses_parts_incrementally_and_closes() {
        let body = encode(&[
            ("2024-01-01T00:00:00Z", JPEG),
            ("2024-01-01T00:00:01Z", JPEG),
        ]);
        let mut parser = Parser::new();
        let mut frames = Vec::new();
        let mut done = false;
        for byte in body {
            for event in parser.push(&[byte]).unwrap() {
                match event {
                    Event::Frame(frame) => frames.push(frame.captured_at),
                    Event::Done => done = true,
                }
            }
        }
        assert!(done);
        assert_eq!(frames, ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]);
    }

    #[test]
    fn rejects_empty_jpeg_and_missing_headers() {
        let mut parser = Parser::new();
        let empty = encode(&[("2024-01-01T00:00:00Z", &[])]);
        assert_eq!(parser.push(&empty), Err(Error::EmptyJpeg));
        let mut parser = Parser::new();
        assert_eq!(
            parser.push(
                b"--a2alabframe\r\nContent-Type: image/jpeg\r\n\r\nxx\r\n--a2alabframe--\r\n"
            ),
            Err(Error::CapturedAt)
        );
    }

    #[test]
    fn capture_gaps_cover_leading_middle_consecutive_and_window_edges() {
        let t0 = captured_us("2024-01-01T00:00:00Z").unwrap();
        let t1 = captured_us("2024-01-01T00:00:01Z").unwrap();
        let t3 = captured_us("2024-01-01T00:00:03Z").unwrap();
        let t5 = captured_us("2024-01-01T00:00:05Z").unwrap();
        let window = captured_us("2024-01-01T00:02:00Z").unwrap();
        let window_next = captured_us("2024-01-01T00:02:01Z").unwrap();
        assert!(!is_capture_gap(t0, t1));
        assert!(is_capture_gap(t1, t3));
        assert!(is_capture_gap(t3, t5));
        assert!(!is_capture_gap(window, window_next));
        assert!(is_capture_gap(t0, t0 + GAP_THRESHOLD_US + 1));
        assert!(!is_capture_gap(t0, t0 + GAP_THRESHOLD_US));
    }

    #[test]
    fn byte_limit_applies_to_the_buffered_part_not_the_session() {
        let mut parser = Parser::new();
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xD9].repeat(64 * 1024);
        let part = encode_part("2024-01-01T00:00:00Z", &jpeg);
        let parts = (MAX_BYTES / part.len()) + 4;
        for _ in 0..parts {
            let events = parser.push(&part).unwrap();
            assert!(matches!(events.as_slice(), [Event::Frame(_)]));
        }
    }

    #[test]
    fn duration_limit_uses_capture_timestamps() {
        let mut parser = Parser::new();
        let first = encode_part("2024-01-01T00:00:00Z", JPEG);
        parser.push(&first).unwrap();
        let late = encode_part("2024-01-01T00:02:01Z", JPEG);
        let events = parser.push(&late).unwrap();
        assert!(matches!(events.as_slice(), [Event::Frame(_)]));
    }

    #[test]
    fn eight_x_hour_span_is_late_after_seven_and_a_half_minutes() {
        let origin_us = captured_us("2024-01-01T00:00:00Z").unwrap();
        let hour_us = captured_us("2024-01-01T01:00:00Z").unwrap();
        let rate = 8.0;
        assert_eq!(due_wall_ms(hour_us, origin_us, 0.0, rate), 450_000.0);
        assert!(!is_late(hour_us, origin_us, 0.0, 449_999.0, rate));
        assert!(is_late(hour_us, origin_us, 0.0, 450_001.0, rate));
        let late = (0..=3_600)
            .filter(|second| {
                is_late(
                    origin_us + i64::from(*second) * 1_000_000,
                    origin_us,
                    0.0,
                    10_000.0,
                    rate,
                )
            })
            .count();
        assert_eq!(late, 80);
    }

    #[test]
    fn half_x_minute_of_capture_is_due_after_two_minutes() {
        let origin_us = captured_us("2024-01-01T00:00:00Z").unwrap();
        let minute_us = origin_us + 60_000_000;
        assert_eq!(due_wall_ms(minute_us, origin_us, 0.0, 0.5), 120_000.0);
    }

    fn encode(frames: &[(&str, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (at, jpeg) in frames {
            body.extend_from_slice(&encode_part(at, jpeg));
        }
        body.extend_from_slice(b"--a2alabframe--\r\n");
        body
    }

    fn encode_part(at: &str, jpeg: &[u8]) -> Vec<u8> {
        let mut part = format!(
            "--a2alabframe\r\nContent-Type: image/jpeg\r\nX-Captured-At: {at}\r\nContent-Length: {}\r\n\r\n",
            jpeg.len()
        )
        .into_bytes();
        part.extend_from_slice(jpeg);
        part.extend_from_slice(b"\r\n");
        part
    }
}
