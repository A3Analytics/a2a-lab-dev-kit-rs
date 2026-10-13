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

pub fn command(source_id: &str, cursor: Option<&str>) -> Value {
    let mut page = Map::new();
    page.insert("limit".to_owned(), Value::from(LIMIT));
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
        page.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
    }
    serde_json::json!({
        "operation": "query_logs",
        "params": {
            "source_id": source_id,
            "range": {"start": RANGE_START, "end": RANGE_END},
            "page": page
        }
    })
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
        let first = command("app", None);
        assert_eq!(first["operation"], "query_logs");
        assert_eq!(first["params"]["source_id"], "app");
        assert_eq!(first["params"]["range"]["start"], RANGE_START);
        assert_eq!(first["params"]["range"]["end"], RANGE_END);
        assert!(first["params"]["page"].get("cursor").is_none());
        assert_eq!(command("app", Some("2"))["params"]["page"]["cursor"], "2");
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
