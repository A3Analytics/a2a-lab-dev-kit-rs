//! UTC and local time field parsing for the Videos timeline.

use wasm_bindgen::JsCast;

use crate::videos::track;

pub(super) fn format_shown(us: i64, local: bool) -> String {
    if local && let Some(text) = browser_local_stamp(us) {
        return text;
    }
    pretty_utc(&track::format_utc(us))
}

fn pretty_utc(stored: &str) -> String {
    let body = stored.trim_end_matches('Z');
    let Some((date, time)) = body.split_once('T') else {
        return stored.to_owned();
    };
    let clock = time.split('.').next().unwrap_or(time);
    format!("{date} {clock} UTC")
}

pub(super) fn display_instant(stored: &str) -> String {
    display_clock(stored, false)
}

pub(super) fn display_clock(stored: &str, local: bool) -> String {
    if stored.is_empty() {
        return String::new();
    }
    crate::videos::mjpeg::captured_us(stored)
        .map(|us| format_shown(us, local))
        .unwrap_or_else(|| stored.to_owned())
}

pub(super) fn reshow(text: &str, from_local: bool, to_local: bool) -> String {
    if text.trim().is_empty() || from_local == to_local {
        return text.to_owned();
    }
    let utc = field_to_utc(text, from_local);
    if crate::videos::mjpeg::captured_us(&utc).is_none() {
        return text.to_owned();
    }
    display_clock(&utc, to_local)
}

pub(super) fn field_to_utc(value: &str, local: bool) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(us) = crate::videos::mjpeg::captured_us(value) {
        return track::format_utc(us);
    }
    let (body, zone) = split_zone(value);
    if zone.is_some_and(|item| item.eq_ignore_ascii_case("utc")) {
        return naive_text_to_utc(body).unwrap_or_else(|| value.to_owned());
    }
    if local && let Some(utc) = local_wall_to_utc(body) {
        return utc;
    }
    naive_text_to_utc(value).unwrap_or_else(|| value.to_owned())
}

fn split_zone(value: &str) -> (&str, Option<&str>) {
    let value = value.trim();
    let Some((body, zone)) = value.rsplit_once(' ') else {
        return (value, None);
    };
    if zone.is_empty() || zone.contains(':') || zone.chars().all(|ch| ch.is_ascii_digit()) {
        return (value, None);
    }
    (body.trim(), Some(zone))
}

struct Wall {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    digits: String,
}

fn naive_text_to_utc(value: &str) -> Option<String> {
    let wall = parse_wall(value)?;
    let rfc = wall_rfc(&wall);
    crate::videos::mjpeg::captured_us(&rfc)?;
    Some(rfc)
}

fn parse_wall(value: &str) -> Option<Wall> {
    let value = value.trim();
    let value = value
        .strip_suffix(" UTC")
        .or_else(|| value.strip_suffix(" utc"))
        .unwrap_or(value)
        .trim();
    let (date, time) = value.split_once('T').or_else(|| value.split_once(' '))?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, fraction),
        None => (time, ""),
    };
    let mut clock_parts = clock.split(':');
    let hour: u32 = clock_parts.next()?.parse().ok()?;
    let minute: u32 = clock_parts.next()?.parse().ok()?;
    let second: u32 = clock_parts.next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let digits: String = fraction
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.len() != fraction.len() || digits.len() > 6 {
        return None;
    }
    Some(Wall {
        year,
        month,
        day,
        hour,
        minute,
        second,
        digits,
    })
}

fn wall_rfc(wall: &Wall) -> String {
    let frac = if wall.digits.is_empty() {
        String::new()
    } else {
        format!(".{}", wall.digits)
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{frac}Z",
        wall.year, wall.month, wall.day, wall.hour, wall.minute, wall.second
    )
}

fn local_wall_to_utc(value: &str) -> Option<String> {
    browser_local_to_utc(&parse_wall(value)?)
}

fn browser_local_stamp(us: i64) -> Option<String> {
    let date = browser_date_from_us(us)?;
    let text = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        date.year, date.month, date.day, date.hour, date.minute, date.second
    );
    if date.zone.is_empty() {
        Some(text)
    } else {
        Some(format!("{text} {}", date.zone))
    }
}

struct LocalParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    zone: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_local_to_utc(_wall: &Wall) -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_date_from_us(_us: i64) -> Option<LocalParts> {
    None
}

#[cfg(target_arch = "wasm32")]
fn browser_local_to_utc(wall: &Wall) -> Option<String> {
    if wall.year < 0 {
        return None;
    }
    let millis = wall_millis(&wall.digits);
    let date = js_sys::Date::new_with_year_month_day_hr_min_sec(
        wall.year as u32,
        wall.month as i32 - 1,
        wall.day as i32,
        wall.hour as i32,
        wall.minute as i32,
        wall.second as i32,
    );
    date.set_milliseconds(millis);
    if date.get_full_year() as i32 != wall.year
        || date.get_month() + 1 != wall.month
        || date.get_date() != wall.day
        || date.get_hours() != wall.hour
        || date.get_minutes() != wall.minute
        || date.get_seconds() != wall.second
    {
        return None;
    }
    let micros = wall_micros(&wall.digits);
    let us = date.get_time().round() as i64 * 1000 + (micros % 1000) as i64;
    Some(track::format_utc(us))
}

#[cfg(target_arch = "wasm32")]
fn browser_date_from_us(us: i64) -> Option<LocalParts> {
    let ms = us as f64 / 1000.0;
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    if date.get_time().is_nan() {
        return None;
    }
    Some(LocalParts {
        year: date.get_full_year() as i32,
        month: date.get_month() + 1,
        day: date.get_date(),
        hour: date.get_hours(),
        minute: date.get_minutes(),
        second: date.get_seconds(),
        zone: zone_abbrev(ms),
    })
}

#[cfg(target_arch = "wasm32")]
fn zone_abbrev(ms: f64) -> String {
    let Ok(intl) = js_sys::Reflect::get(&js_sys::global(), &"Intl".into()) else {
        return String::new();
    };
    let Ok(ctor) = js_sys::Reflect::get(&intl, &"DateTimeFormat".into()) else {
        return String::new();
    };
    let Ok(ctor) = ctor.dyn_into::<js_sys::Function>() else {
        return String::new();
    };
    let options = js_sys::Object::new();
    if js_sys::Reflect::set(&options, &"timeZoneName".into(), &"short".into()).is_err() {
        return String::new();
    }
    let Ok(formatter) = js_sys::Reflect::construct(
        &ctor,
        &js_sys::Array::of2(&wasm_bindgen::JsValue::UNDEFINED, &options),
    ) else {
        return String::new();
    };
    let Ok(format_to_parts) = js_sys::Reflect::get(&formatter, &"formatToParts".into()) else {
        return String::new();
    };
    let Ok(format_to_parts) = format_to_parts.dyn_into::<js_sys::Function>() else {
        return String::new();
    };
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let Ok(parts) = format_to_parts.call1(&formatter, &date) else {
        return String::new();
    };
    for part in js_sys::Array::from(&parts).iter() {
        let Ok(kind) = js_sys::Reflect::get(&part, &"type".into()) else {
            continue;
        };
        if kind.as_string().as_deref() != Some("timeZoneName") {
            continue;
        }
        return js_sys::Reflect::get(&part, &"value".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
    }
    String::new()
}

#[cfg(target_arch = "wasm32")]
fn wall_millis(digits: &str) -> u32 {
    wall_micros(digits) / 1000
}

#[cfg(target_arch = "wasm32")]
fn wall_micros(digits: &str) -> u32 {
    if digits.is_empty() {
        return 0;
    }
    format!("{digits:0<6}").parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::field_to_utc;

    #[test]
    fn explicit_utc_text_stays_utc_in_either_mode() {
        assert_eq!(
            field_to_utc("2024-01-01 00:00:00 UTC", false),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            field_to_utc("2024-01-01 00:00:00 UTC", true),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            field_to_utc("2024-01-01T00:00:00Z", true),
            "2024-01-01T00:00:00Z"
        );
    }
}
