//! Incremental MJPEG multipart parser. No blob URLs.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub const BOUNDARY: &str = "a2alabframe";
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FRAMES: usize = 1_800;
pub const MAX_QUEUE: usize = 3;
pub const MAX_DURATION_US: i64 = 60 * 1_000_000;

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
    bytes_seen: usize,
    frames: usize,
    first_us: Option<i64>,
    closed: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            marker: format!("--{BOUNDARY}").into_bytes(),
            buf: Vec::new(),
            bytes_seen: 0,
            frames: 0,
            first_us: None,
            closed: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Event>, Error> {
        if self.bytes_seen.saturating_add(chunk.len()) > MAX_BYTES {
            return Err(Error::TooManyBytes);
        }
        self.bytes_seen += chunk.len();
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
        if let Some(first) = self.first_us {
            if captured_us.saturating_sub(first) > MAX_DURATION_US {
                return Err(Error::TooLong);
            }
        } else {
            self.first_us = Some(captured_us);
        }
        self.frames += 1;
        if self.frames > MAX_FRAMES {
            return Err(Error::TooManyFrames);
        }
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
    fn duration_limit_uses_capture_timestamps() {
        let mut parser = Parser::new();
        let first = encode_part("2024-01-01T00:00:00Z", JPEG);
        parser.push(&first).unwrap();
        let late = encode_part("2024-01-01T00:01:01Z", JPEG);
        assert_eq!(parser.push(&late), Err(Error::TooLong));
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
