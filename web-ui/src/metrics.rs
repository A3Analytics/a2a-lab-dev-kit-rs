//! Metric samples read from one metric.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::Fault;

const LIMIT: u32 = 100;
const RANGE_START: &str = "1970-01-01T00:00:00Z";
const RANGE_END: &str = "2100-01-01T00:00:00Z";

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub timestamp: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub samples: Vec<Sample>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRange {
    Window,
    Bounded { start: String, end: String },
}

pub fn query_range(start: &str, end: &str) -> Result<QueryRange, &'static str> {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() && end.is_empty() {
        return Ok(QueryRange::Window);
    }
    if start.is_empty() || end.is_empty() {
        return Err("Enter a start and an end.");
    }
    let start = utc_timestamp(start).ok_or("Enter a UTC time range.")?;
    let end = utc_timestamp(end).ok_or("Enter a UTC time range.")?;
    if start >= end {
        return Err("The start must be before the end.");
    }
    Ok(QueryRange::Bounded { start, end })
}

pub fn command(metric_id: &str, cursor: Option<&str>, range: &QueryRange) -> Value {
    let mut page = Map::new();
    page.insert("limit".to_owned(), Value::from(LIMIT));
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
        page.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
    }
    let (start, end) = match range {
        QueryRange::Window => (RANGE_START, RANGE_END),
        QueryRange::Bounded { start, end } => (start.as_str(), end.as_str()),
    };
    serde_json::json!({
        "operation": "query_metric",
        "params": {
            "metric_id": metric_id,
            "range": {"start": start, "end": end},
            "page": page
        }
    })
}

fn utc_timestamp(value: &str) -> Option<String> {
    let body = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .or_else(|| value.strip_suffix("-00:00"))
        .unwrap_or(value);
    if body.contains('+') || body.ends_with("Z") {
        return None;
    }
    let (date, time) = body.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?;
    let month = date_parts.next()?.parse::<u8>().ok()?;
    let day = date_parts.next()?.parse::<u8>().ok()?;
    if year.len() != 4 || date_parts.next().is_some() {
        return None;
    }
    let year = year.parse::<i32>().ok()?;
    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?.parse::<u8>().ok()?;
    let minute = clock_parts.next()?.parse::<u8>().ok()?;
    let second = match clock_parts.next() {
        Some(second) => second.parse::<u8>().ok()?,
        None => 0,
    };
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let fraction = match fraction {
        None => "000000000".to_owned(),
        Some(fraction)
            if !fraction.is_empty()
                && fraction.len() <= 9
                && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            format!("{fraction:0<9}")
        }
        Some(_) => return None,
    };
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction}Z"
    ))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

pub fn interpret(reply: &Reply, secret: Option<&str>) -> Result<Page, Fault> {
    let mut page = match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => return Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => return Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => return Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => {
            let unavailable = status.status == "INTERNAL"
                || status.status == "UNAVAILABLE"
                || status
                    .reasons
                    .iter()
                    .any(|reason| reason == "INTERNAL_ERROR");
            if unavailable {
                return Err(Fault::Unavailable(status.message));
            }
            return Err(Fault::Malformed);
        }
        Ok(outcome) => page_from(&outcome)?,
    };
    for sample in &mut page.samples {
        sample.timestamp = codec::redact(&sample.timestamp, secret);
        sample.value = codec::redact(&sample.value, secret);
    }
    Ok(page)
}

fn page_from(outcome: &Outcome) -> Result<Page, Fault> {
    let payloads = match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task
            .artifacts
            .iter()
            .flat_map(|artifact| artifact.data.clone())
            .collect::<Vec<_>>(),
        Outcome::Exchange(Exchange::Message(message)) => message.data.clone(),
        Outcome::Tasks(_) | Outcome::Status(_) => Vec::new(),
    };
    if payloads.is_empty() {
        return Err(Fault::Malformed);
    }
    let mut samples = Vec::new();
    let mut next_cursor = None;
    for payload in payloads {
        let page = parse_payload(&payload)?;
        next_cursor = page.next_cursor;
        samples.extend(page.samples);
    }
    Ok(Page {
        samples,
        next_cursor,
    })
}

fn parse_payload(payload: &str) -> Result<Page, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    if object.get("operation").and_then(Value::as_str) != Some("query_metric") {
        return Err(Fault::Malformed);
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let raw = result
        .get("items")
        .and_then(Value::as_array)
        .ok_or(Fault::Malformed)?;
    let mut samples = Vec::new();
    for item in raw {
        samples.push(parse_sample(item)?);
    }
    Ok(Page {
        samples,
        next_cursor: match result.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
            Some(_) => return Err(Fault::Malformed),
        },
    })
}

fn parse_sample(value: &Value) -> Result<Sample, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    Ok(Sample {
        timestamp: required(object, "timestamp")?,
        value: number(object.get("value"))?,
    })
}

fn required(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(Fault::Malformed),
    }
}

pub fn chart_payload(samples: &[Sample]) -> String {
    let items = samples
        .iter()
        .filter_map(|sample| {
            let value = sample.value.parse::<f64>().ok()?;
            value.is_finite().then(|| {
                serde_json::json!({
                    "t": sample.timestamp,
                    "v": value,
                })
            })
        })
        .collect::<Vec<_>>();
    Value::Array(items).to_string()
}

fn number(value: Option<&Value>) -> Result<String, Fault> {
    let Some(Value::Number(number)) = value else {
        return Err(Fault::Malformed);
    };
    let Some(value) = number.as_f64().filter(|value| value.is_finite()) else {
        return Err(Fault::Malformed);
    };
    Ok(format!("{value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_reads_one_metric_across_the_viewer_window() {
        let first = command("latency", None, &QueryRange::Window);
        assert_eq!(first["operation"], "query_metric");
        assert_eq!(first["params"]["metric_id"], "latency");
        assert_eq!(first["params"]["range"]["start"], RANGE_START);
        assert_eq!(first["params"]["range"]["end"], RANGE_END);
        assert!(first["params"]["page"].get("cursor").is_none());
        assert_eq!(
            command("latency", Some("2"), &QueryRange::Window)["params"]["page"]["cursor"],
            "2"
        );
    }

    #[test]
    fn time_range_is_half_open_and_sent_as_utc() {
        let range = query_range("2024-01-01T00:01", "2024-01-01T00:02:00").unwrap();
        let query = command("door", None, &range);
        assert_eq!(
            query["params"]["range"]["start"],
            "2024-01-01T00:01:00.000000000Z"
        );
        assert_eq!(
            query["params"]["range"]["end"],
            "2024-01-01T00:02:00.000000000Z"
        );
        assert_eq!(query_range("", "").unwrap(), QueryRange::Window);
        assert_eq!(
            query_range("2024-01-01T00:02:00Z", "").unwrap_err(),
            "Enter a start and an end."
        );
        assert_eq!(
            query_range("2024-01-01T00:02:00Z", "2024-01-01T00:01:00Z").unwrap_err(),
            "The start must be before the end."
        );
        assert!(query_range("2024-01-01T00:00:00+01:00", "2024-01-02T00:00:00Z").is_err());
    }

    #[test]
    fn samples_keep_order_and_print_whole_numbers_without_a_decimal() {
        let reply = page_reply(json!({
            "operation": "query_metric",
            "result": {
                "items": [
                    {"timestamp": "2024-01-01T00:00:00Z", "value": 5.0},
                    {"timestamp": "2024-01-01T00:01:00Z", "value": 21.5}
                ],
                "next_cursor": "2"
            }
        }));
        let page = interpret(&reply, Some("2024-01-01T00:00:00Z")).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
        assert_eq!(page.samples[0].timestamp, "[redacted]");
        assert_eq!(page.samples[0].value, "5");
        assert_eq!(page.samples[1].value, "21.5");
    }

    #[test]
    fn chart_payload_keeps_finite_samples_in_order() {
        let samples = vec![
            Sample {
                timestamp: "2024-01-01T00:00:00Z".to_owned(),
                value: "5".to_owned(),
            },
            Sample {
                timestamp: "2024-01-01T00:01:00Z".to_owned(),
                value: "not-a-number".to_owned(),
            },
            Sample {
                timestamp: "2024-01-01T00:02:00Z".to_owned(),
                value: "21.5".to_owned(),
            },
        ];
        assert_eq!(
            serde_json::from_str::<Value>(&chart_payload(&samples)).unwrap(),
            json!([
                {"t": "2024-01-01T00:00:00Z", "v": 5.0},
                {"t": "2024-01-01T00:02:00Z", "v": 21.5}
            ])
        );
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
