//! Log records read from one source.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::Fault;

const LIMIT: u32 = 100;
const RANGE_START: &str = "1970-01-01T00:00:00Z";
const RANGE_END: &str = "2100-01-01T00:00:00Z";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub source_id: String,
    pub timestamp: String,
    pub level: String,
    pub message: String,
    pub attributes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub records: Vec<Record>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRange {
    Window,
    Bounded { start: String, end: String },
}

pub fn last_day(now: &str) -> Option<QueryRange> {
    let today = calendar_day(now.split('T').next()?)?;
    let start = previous_day(today)?;
    date_range(&format_day(start), &format_day(today)).ok()
}

pub fn input_days(start: &str, end: &str) -> Option<(String, String)> {
    let start_day = format_day(calendar_day(start.split('T').next()?)?);
    let end_day = format_day(previous_day(calendar_day(end.split('T').next()?)?)?);
    Some((start_day, end_day))
}

pub fn date_range(start: &str, end: &str) -> Result<QueryRange, &'static str> {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() && end.is_empty() {
        return Ok(QueryRange::Window);
    }
    if start.is_empty() || end.is_empty() {
        return Err("Enter a start and an end.");
    }
    let start = calendar_day(start).ok_or("Enter a UTC date range.")?;
    let end = calendar_day(end).ok_or("Enter a UTC date range.")?;
    if start > end {
        return Err("The start must be on or before the end.");
    }
    let end = next_day(end).ok_or("Enter a UTC date range.")?;
    Ok(QueryRange::Bounded {
        start: format!("{}T00:00:00Z", format_day(start)),
        end: format!("{}T00:00:00Z", format_day(end)),
    })
}

pub fn command(source_id: &str, cursor: Option<&str>, range: &QueryRange) -> Value {
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
        "operation": "query_logs",
        "params": {
            "source_id": source_id,
            "range": {"start": start, "end": end},
            "page": page
        }
    })
}

fn calendar_day(value: &str) -> Option<(i32, u8, u8)> {
    let mut parts = value.split('-');
    let year = parts.next()?;
    let month = parts.next()?.parse::<u8>().ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    if year.len() != 4 || parts.next().is_some() {
        return None;
    }
    let year = year.parse::<i32>().ok()?;
    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn previous_day((year, month, day): (i32, u8, u8)) -> Option<(i32, u8, u8)> {
    if day > 1 {
        return Some((year, month, day - 1));
    }
    if month > 1 {
        let month = month - 1;
        return Some((year, month, days_in_month(year, month)));
    }
    year.checked_sub(1).map(|year| (year, 12, 31))
}

fn next_day((year, month, day): (i32, u8, u8)) -> Option<(i32, u8, u8)> {
    if day < days_in_month(year, month) {
        return Some((year, month, day + 1));
    }
    if month < 12 {
        return Some((year, month + 1, 1));
    }
    year.checked_add(1).map(|year| (year, 1, 1))
}

fn format_day((year, month, day): (i32, u8, u8)) -> String {
    format!("{year:04}-{month:02}-{day:02}")
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
    for record in &mut page.records {
        record.source_id = codec::redact(&record.source_id, secret);
        record.timestamp = codec::redact(&record.timestamp, secret);
        record.level = codec::redact(&record.level, secret);
        record.message = codec::redact(&record.message, secret);
        record.attributes = codec::redact(&record.attributes, secret);
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
    let mut records = Vec::new();
    let mut next_cursor = None;
    for payload in payloads {
        let page = parse_payload(&payload)?;
        next_cursor = page.next_cursor;
        records.extend(page.records);
    }
    Ok(Page {
        records,
        next_cursor,
    })
}

fn parse_payload(payload: &str) -> Result<Page, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    if object.get("operation").and_then(Value::as_str) != Some("query_logs") {
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
    let mut records = Vec::new();
    for item in raw {
        records.push(parse_record(item)?);
    }
    Ok(Page {
        records,
        next_cursor: match result.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
            Some(_) => return Err(Fault::Malformed),
        },
    })
}

fn parse_record(value: &Value) -> Result<Record, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    Ok(Record {
        source_id: required(object, "source_id")?,
        timestamp: required(object, "timestamp")?,
        level: required(object, "level")?,
        message: required(object, "message")?,
        attributes: attributes(object.get("attributes"))?,
    })
}

fn required(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(Fault::Malformed),
    }
}

fn attributes(value: Option<&Value>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::Object(fields)) if fields.is_empty() => Ok(String::new()),
        Some(value) => serde_json::to_string(value).map_err(|_| Fault::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_reads_one_source_across_the_viewer_window() {
        let first = command("app", None, &QueryRange::Window);
        assert_eq!(first["operation"], "query_logs");
        assert_eq!(first["params"]["source_id"], "app");
        assert_eq!(first["params"]["range"]["start"], RANGE_START);
        assert_eq!(first["params"]["range"]["end"], RANGE_END);
        assert!(first["params"]["page"].get("cursor").is_none());
        assert_eq!(
            command("app", Some("2"), &QueryRange::Window)["params"]["page"]["cursor"],
            "2"
        );
    }

    #[test]
    fn date_range_includes_the_end_date() {
        let range = date_range("2024-01-01", "2024-01-01").unwrap();
        let query = command("app", None, &range);
        assert_eq!(query["params"]["range"]["start"], "2024-01-01T00:00:00Z");
        assert_eq!(query["params"]["range"]["end"], "2024-01-02T00:00:00Z");
        assert_eq!(date_range("", "").unwrap(), QueryRange::Window);
        assert_eq!(
            last_day("2026-10-20T18:42:00.000Z"),
            Some(QueryRange::Bounded {
                start: "2026-10-19T00:00:00Z".to_owned(),
                end: "2026-10-21T00:00:00Z".to_owned(),
            })
        );
        assert_eq!(
            input_days("2026-10-19T00:00:00Z", "2026-10-21T00:00:00Z"),
            Some(("2026-10-19".to_owned(), "2026-10-20".to_owned()))
        );
        assert_eq!(
            last_day("2024-03-01T00:00:00Z"),
            Some(QueryRange::Bounded {
                start: "2024-02-29T00:00:00Z".to_owned(),
                end: "2024-03-02T00:00:00Z".to_owned(),
            })
        );
        assert_eq!(
            date_range("2024-01-02", "").unwrap_err(),
            "Enter a start and an end."
        );
        assert_eq!(
            date_range("2024-01-02", "2024-01-01").unwrap_err(),
            "The start must be on or before the end."
        );
    }

    #[test]
    fn records_keep_order_and_drop_empty_attributes() {
        let reply = page_reply(json!({
            "operation": "query_logs",
            "result": {
                "items": [
                    {
                        "source_id": "app",
                        "timestamp": "2024-01-01T00:00:00Z",
                        "level": "info",
                        "message": "lab ready",
                        "attributes": {}
                    },
                    {
                        "source_id": "app",
                        "timestamp": "2024-01-01T00:01:00Z",
                        "level": "warn",
                        "message": "retry",
                        "attributes": {"attempt": 2}
                    }
                ],
                "next_cursor": "2"
            }
        }));
        let page = interpret(&reply, Some("lab ready")).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
        assert_eq!(page.records[0].source_id, "app");
        assert_eq!(page.records[0].message, "[redacted]");
        assert_eq!(page.records[0].attributes, "");
        assert_eq!(page.records[1].level, "warn");
        assert_eq!(page.records[1].attributes, r#"{"attempt":2}"#);
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
