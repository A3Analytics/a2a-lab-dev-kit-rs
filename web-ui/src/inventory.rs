//! Inventory of the four provider-backed list commands.
//!
//! Chat context ids and protocol task ids are not read.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::card::Card;
use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};

pub const DEFAULT_LIMIT: u32 = 100;
pub const MAX_LIMIT: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Primitive {
    Logs,
    Metrics,
    Tasks,
    Images,
}

#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub section: &'static str,
    pub status: &'static str,
    pub alert: &'static str,
    pub limit: &'static str,
    pub cursor: &'static str,
    pub more: &'static str,
    pub items: &'static str,
    pub heading: &'static str,
    pub operation: &'static str,
    pub skill: &'static str,
    pub noun: &'static str,
    pub limit_label: &'static str,
    pub cursor_label: &'static str,
    pub more_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pending,
    Advertised,
    Unadvertised,
    Loading,
    Empty,
    Populated,
    Unavailable,
    Unauthorized,
    Forbidden,
    Malformed,
    Transport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryFault {
    Limit,
    Cursor,
    NoPage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub description: String,
    pub details: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    Unauthorized(String),
    Forbidden(String),
    Unavailable(String),
    Malformed,
    Transport,
}

impl Primitive {
    pub const ALL: [Self; 4] = [Self::Logs, Self::Metrics, Self::Tasks, Self::Images];

    pub const fn spec(self) -> Spec {
        match self {
            Self::Logs => Spec {
                section: "inventory-logs",
                status: "inventory-logs-status",
                alert: "inventory-logs-alert",
                limit: "inventory-logs-limit",
                cursor: "inventory-logs-cursor",
                more: "inventory-logs-more",
                items: "inventory-logs-items",
                heading: "Log sources",
                operation: "list_log_sources",
                skill: "list-log-sources",
                noun: "Log sources",
                limit_label: "Log source limit",
                cursor_label: "Log source cursor",
                more_label: "More log sources",
            },
            Self::Metrics => Spec {
                section: "inventory-metrics",
                status: "inventory-metrics-status",
                alert: "inventory-metrics-alert",
                limit: "inventory-metrics-limit",
                cursor: "inventory-metrics-cursor",
                more: "inventory-metrics-more",
                items: "inventory-metrics-items",
                heading: "Metrics",
                operation: "list_metrics",
                skill: "list-metrics",
                noun: "Metrics",
                limit_label: "Metric limit",
                cursor_label: "Metric cursor",
                more_label: "More metrics",
            },
            Self::Tasks => Spec {
                section: "inventory-tasks",
                status: "inventory-tasks-status",
                alert: "inventory-tasks-alert",
                limit: "inventory-tasks-limit",
                cursor: "inventory-tasks-cursor",
                more: "inventory-tasks-more",
                items: "inventory-tasks-items",
                heading: "Task definitions",
                operation: "list_tasks",
                skill: "list-tasks",
                noun: "Task definitions",
                limit_label: "Task definition limit",
                cursor_label: "Task definition cursor",
                more_label: "More task definitions",
            },
            Self::Images => Spec {
                section: "inventory-images",
                status: "inventory-images-status",
                alert: "inventory-images-alert",
                limit: "inventory-images-limit",
                cursor: "inventory-images-cursor",
                more: "inventory-images-more",
                items: "inventory-images-items",
                heading: "Image sources",
                operation: "list_image_sources",
                skill: "list-image-sources",
                noun: "Image sources",
                limit_label: "Image source limit",
                cursor_label: "Image source cursor",
                more_label: "More image sources",
            },
        }
    }

    pub fn is_advertised(self, card: &Card) -> bool {
        let skill = self.spec().skill;
        card.skills.iter().any(|item| item.id == skill)
    }
}

impl Kind {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Advertised => "advertised",
            Self::Unadvertised => "unadvertised",
            Self::Loading => "loading",
            Self::Empty => "empty",
            Self::Populated => "populated",
            Self::Unavailable => "unavailable",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::Malformed => "malformed",
            Self::Transport => "transport",
        }
    }

    pub const fn configured(self) -> bool {
        matches!(self, Self::Empty | Self::Populated)
    }

    pub fn message(self, primitive: Primitive, extra: &str) -> String {
        let noun = primitive.spec().noun;
        match self {
            Self::Pending => "Waiting for the agent card.".to_owned(),
            Self::Advertised => {
                format!("{noun} are advertised.")
            }
            Self::Unadvertised => format!("{noun} are not advertised."),
            Self::Loading => format!("Loading {noun}."),
            Self::Empty => format!("{noun} are configured. The inventory is empty."),
            Self::Populated => format!("{noun} are configured. {extra} shown."),
            Self::Unavailable => unavailable_message(noun, extra),
            Self::Unauthorized => format!("{noun} were not authorized."),
            Self::Forbidden => format!("{noun} were forbidden."),
            Self::Malformed => format!("{noun} returned a malformed result."),
            Self::Transport => format!("{noun} could not be reached."),
        }
    }
}

impl QueryFault {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Limit => "Enter a limit from 1 to 1000.",
            Self::Cursor => "Enter a numeric cursor, or leave it blank.",
            Self::NoPage => "There is no further page.",
        }
    }
}

pub fn field_query(cursor: &str, limit: &str) -> Result<PageQuery, QueryFault> {
    let cursor = cursor.trim();
    let cursor = if cursor.is_empty() {
        None
    } else if cursor.bytes().all(|byte| byte.is_ascii_digit()) {
        Some(cursor.to_owned())
    } else {
        return Err(QueryFault::Cursor);
    };
    Ok(PageQuery {
        cursor,
        limit: parse_limit(limit)?,
    })
}

pub fn continued_query(cursor: &str, limit: &str) -> Result<PageQuery, QueryFault> {
    let cursor = cursor.trim();
    if cursor.is_empty() {
        return Err(QueryFault::NoPage);
    }
    Ok(PageQuery {
        cursor: Some(cursor.to_owned()),
        limit: parse_limit(limit)?,
    })
}

pub fn command_value(primitive: Primitive, query: &PageQuery) -> Value {
    let mut page = Map::new();
    if let Some(cursor) = &query.cursor {
        page.insert("cursor".to_owned(), Value::String(cursor.clone()));
    }
    page.insert("limit".to_owned(), Value::from(query.limit));
    serde_json::json!({
        "operation": primitive.spec().operation,
        "params": { "page": page }
    })
}

pub fn interpret(
    primitive: Primitive,
    reply: &Reply,
    secret: Option<&str>,
) -> Result<Listed, Fault> {
    match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => status_fault(&status),
        Ok(outcome) => listed_from(primitive, &outcome),
    }
}

pub fn merge_items(existing: &[Item], incoming: &[Item], replace: bool) -> Vec<Item> {
    let mut items = if replace {
        Vec::new()
    } else {
        existing.to_vec()
    };
    for item in incoming {
        if items.iter().any(|have| have.id == item.id) {
            continue;
        }
        items.push(item.clone());
    }
    items
}

pub fn fault_kind(fault: &Fault) -> Kind {
    match fault {
        Fault::Unavailable(_) => Kind::Unavailable,
        Fault::Unauthorized(_) => Kind::Unauthorized,
        Fault::Forbidden(_) => Kind::Forbidden,
        Fault::Malformed => Kind::Malformed,
        Fault::Transport => Kind::Transport,
    }
}

fn unavailable_message(noun: &str, extra: &str) -> String {
    if extra.is_empty() {
        format!("{noun} are unavailable and are not configured.")
    } else {
        format!("{noun} are unavailable and are not configured. {extra}")
    }
}

fn parse_limit(limit: &str) -> Result<u32, QueryFault> {
    let limit = limit.trim();
    let limit = if limit.is_empty() {
        DEFAULT_LIMIT
    } else {
        limit.parse::<u32>().map_err(|_| QueryFault::Limit)?
    };
    if limit == 0 || limit > MAX_LIMIT {
        return Err(QueryFault::Limit);
    }
    Ok(limit)
}

fn status_fault(status: &codec::RpcStatus) -> Result<Listed, Fault> {
    let unavailable = status.status == "INTERNAL"
        || status.status == "UNAVAILABLE"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "INTERNAL_ERROR");
    if unavailable {
        return Err(Fault::Unavailable(status.message.clone()));
    }
    Err(Fault::Malformed)
}

fn listed_from(primitive: Primitive, outcome: &Outcome) -> Result<Listed, Fault> {
    let payloads = payloads_of(outcome);
    if payloads.is_empty() {
        return Err(Fault::Malformed);
    }
    let mut items = Vec::new();
    let mut next_cursor = None;
    for payload in payloads {
        let page = parse_payload(primitive, &payload)?;
        next_cursor = page.next_cursor;
        items.extend(page.items);
    }
    Ok(Listed { items, next_cursor })
}

fn payloads_of(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task
            .artifacts
            .iter()
            .flat_map(|artifact| artifact.data.clone())
            .collect(),
        Outcome::Exchange(Exchange::Message(message)) => message.data.clone(),
        Outcome::Tasks(_) | Outcome::Status(_) => Vec::new(),
    }
}

fn parse_payload(primitive: Primitive, payload: &str) -> Result<Listed, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or(Fault::Malformed)?;
    if operation != primitive.spec().operation {
        return Err(Fault::Malformed);
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let raw_items = result
        .get("items")
        .and_then(Value::as_array)
        .ok_or(Fault::Malformed)?;
    let mut items = Vec::new();
    for item in raw_items {
        items.push(parse_item(primitive, item)?);
    }
    Ok(Listed {
        items,
        next_cursor: next_cursor(result.get("next_cursor"))?,
    })
}

fn next_cursor(value: Option<&Value>) -> Result<Option<String>, Fault> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if !cursor.is_empty() => Ok(Some(cursor.clone())),
        Some(_) => Err(Fault::Malformed),
    }
}

fn parse_item(primitive: Primitive, value: &Value) -> Result<Item, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let id = required_string(object, "id")?;
    let name = required_string(object, "name")?;
    let description = required_string(object, "description")?;
    let mut details = Vec::new();
    if primitive == Primitive::Metrics {
        details.push(("Unit", required_string(object, "unit")?));
    }
    push_optional(&mut details, "Asset id", object, "asset_id")?;
    push_optional(&mut details, "Semantic id", object, "semantic_id")?;
    if primitive == Primitive::Tasks {
        push_optional(&mut details, "Input schema", object, "input_schema")?;
        push_optional(&mut details, "Output schema", object, "output_schema")?;
    }
    Ok(Item {
        id,
        name,
        description,
        details,
    })
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(Fault::Malformed),
    }
}

fn push_optional(
    details: &mut Vec<(&'static str, String)>,
    label: &'static str,
    object: &Map<String, Value>,
    key: &str,
) -> Result<(), Fault> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(value)) => {
            details.push((label, value.clone()));
            Ok(())
        }
        Some(_) => Err(Fault::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_card;
    use serde_json::json;

    const SECRET: &str = "page-lifetime-token";
    const MARKUP: &str = r#"<img id="inventory-xss" src=x onerror="window.__inventoryXss=1">"#;

    #[test]
    fn commands_use_the_four_list_operations_and_valid_pages() {
        let first = field_query("", "1").unwrap();
        assert!(first.cursor.is_none());
        assert_eq!(first.limit, 1);
        let continued = continued_query("10", "").unwrap();
        assert_eq!(continued.cursor.as_deref(), Some("10"));
        assert_eq!(continued.limit, DEFAULT_LIMIT);
        assert_eq!(field_query(" 2 ", "1000").unwrap().limit, MAX_LIMIT);
        assert_eq!(field_query("abc", "1"), Err(QueryFault::Cursor));
        assert_eq!(field_query("", "0"), Err(QueryFault::Limit));
        assert_eq!(field_query("", "1001"), Err(QueryFault::Limit));
        assert_eq!(continued_query("  ", "1"), Err(QueryFault::NoPage));

        for primitive in Primitive::ALL {
            let query = continued_query("2", "1").unwrap();
            let command = command_value(primitive, &query);
            let body = codec::command_message("ui-1", &command, "").unwrap();
            let value: Value = serde_json::from_str(&body).unwrap();
            assert!(value["message"].get("contextId").is_none());
            assert_eq!(
                value["message"]["parts"][0]["data"]["operation"],
                primitive.spec().operation
            );
            assert_eq!(
                value["message"]["parts"][0]["data"]["params"]["page"]["cursor"],
                "2"
            );
            assert_eq!(
                value["message"]["parts"][0]["data"]["params"]["page"]["limit"],
                1
            );
            assert!(!body.contains("query_logs"));
            assert!(!body.contains("start_task"));
        }
    }

    #[test]
    fn advertised_skills_are_distinct_from_configured_results() {
        let card = parse_card(
            &json!({
                "name": "lab",
                "version": "1",
                "skills": [
                    {"id": "list-log-sources", "name": "Logs", "description": "List"},
                    {"id": "list-tasks", "name": "Tasks", "description": "List"}
                ]
            })
            .to_string(),
        )
        .unwrap();
        assert!(Primitive::Logs.is_advertised(&card));
        assert!(!Primitive::Metrics.is_advertised(&card));
        assert!(Primitive::Tasks.is_advertised(&card));
        assert!(!Primitive::Images.is_advertised(&card));
        assert!(!Kind::Advertised.configured());
        assert!(!Kind::Unadvertised.configured());
        assert!(Kind::Empty.configured());
        assert!(Kind::Populated.configured());
        assert!(!Kind::Unavailable.configured());
        let advertised = Kind::Advertised.message(Primitive::Logs, "");
        let missing = Kind::Unadvertised.message(Primitive::Images, "");
        let empty = Kind::Empty.message(Primitive::Metrics, "");
        assert!(advertised.contains("advertised"));
        assert!(missing.contains("not advertised"));
        assert!(empty.contains("empty"));
        assert_ne!(advertised, missing);
        assert_ne!(
            empty,
            Kind::Unavailable.message(Primitive::Images, "offline")
        );
    }

    #[test]
    fn parses_populated_empty_paginated_and_markup_pages() {
        let logs = interpret(Primitive::Logs, &page_reply(vec![log_page(true)]), None).unwrap();
        assert_eq!(logs.items.len(), 1);
        assert_eq!(logs.items[0].id, "log-a");
        assert_eq!(logs.items[0].description, MARKUP);
        assert!(
            logs.items[0]
                .details
                .iter()
                .any(|detail| detail.0 == "Asset id")
        );
        assert_eq!(logs.next_cursor.as_deref(), Some("1"));
        assert!(!logs.items[0].id.contains("protocol-task"));

        let metrics = interpret(
            Primitive::Metrics,
            &page_reply(vec![json!({
                "operation": "list_metrics",
                "result": {"items": [{
                    "id": "metric-a",
                    "name": "Latency",
                    "description": MARKUP,
                    "unit": "ms",
                    "asset_id": "asset-metric",
                    "semantic_id": "semantic-metric"
                }]}
            })]),
            None,
        )
        .unwrap();
        assert_eq!(metrics.items[0].details[0], ("Unit", "ms".to_owned()));
        assert!(metrics.items[0].description.contains("onerror"));

        let tasks = interpret(
            Primitive::Tasks,
            &page_reply(vec![json!({
                "operation": "list_tasks",
                "result": {"items": [{
                    "id": "task-a",
                    "name": MARKUP,
                    "description": "Build",
                    "input_schema": "{\"title\":\"<b>in</b>\"}",
                    "output_schema": "{\"title\":\"<b>out</b>\"}"
                }]}
            })]),
            None,
        )
        .unwrap();
        assert_eq!(tasks.items[0].id, "task-a");
        assert!(tasks.items[0].name.contains("<img"));
        assert!(
            tasks.items[0]
                .details
                .iter()
                .any(|detail| detail.1.contains("<b>in</b>"))
        );
        assert!(
            tasks.items[0]
                .details
                .iter()
                .any(|detail| detail.1.contains("<b>out</b>"))
        );

        let images = interpret(
            Primitive::Images,
            &page_reply(vec![
                json!({
                    "operation": "list_image_sources",
                    "result": {"items": [image("image-a")]}
                }),
                json!({
                    "operation": "list_image_sources",
                    "result": {"items": [image("image-b")], "next_cursor": "2"}
                }),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(images.items.len(), 2);
        assert_eq!(images.items[0].id, "image-a");
        assert_eq!(images.items[1].id, "image-b");
        assert_eq!(images.next_cursor.as_deref(), Some("2"));

        let empty = interpret(
            Primitive::Logs,
            &page_reply(vec![json!({
                "operation": "list_log_sources",
                "result": {"items": []}
            })]),
            None,
        )
        .unwrap();
        assert!(empty.items.is_empty());
        assert!(Kind::Empty.configured());

        let merged = merge_items(&logs.items, &second_log().items, false);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "log-a");
        assert_eq!(merged[1].id, "log-b");
        let replaced = merge_items(&merged, &logs.items, true);
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].id, "log-a");
        let again = merge_items(&merged, &logs.items, false);
        assert_eq!(again.len(), 2);
        assert_eq!(merge_items(&logs.items, &metrics.items, false).len(), 2);
    }

    #[test]
    fn failures_stay_distinct_and_do_not_count_as_configured() {
        let unavailable = interpret(
            Primitive::Images,
            &Reply {
                status: 500,
                body: json!({
                    "error": {
                        "code": 500,
                        "status": "INTERNAL",
                        "message": format!("image provider is not configured {SECRET}"),
                        "details": [{"reason": "INTERNAL_ERROR", "domain": "a2a-protocol.org"}]
                    }
                })
                .to_string(),
            },
            Some(SECRET),
        );
        assert!(
            matches!(unavailable, Err(Fault::Unavailable(ref message)) if !message.contains(SECRET))
        );
        assert_eq!(
            fault_kind(unavailable.as_ref().unwrap_err()),
            Kind::Unavailable
        );
        assert!(!fault_kind(unavailable.as_ref().unwrap_err()).configured());

        let unauthorized = interpret(
            Primitive::Logs,
            &Reply {
                status: 401,
                body: json!({
                    "error": {"code": 401, "status": "UNAUTHENTICATED", "message": SECRET}
                })
                .to_string(),
            },
            Some(SECRET),
        );
        assert!(
            matches!(unauthorized, Err(Fault::Unauthorized(ref message)) if !message.contains(SECRET))
        );
        assert_eq!(
            fault_kind(unauthorized.as_ref().unwrap_err()),
            Kind::Unauthorized
        );

        let forbidden = interpret(
            Primitive::Metrics,
            &Reply {
                status: 403,
                body: json!({
                    "error": {"code": 403, "status": "PERMISSION_DENIED", "message": "no"}
                })
                .to_string(),
            },
            None,
        );
        assert_eq!(fault_kind(forbidden.as_ref().unwrap_err()), Kind::Forbidden);

        let malformed = [
            Reply {
                status: 200,
                body: "not-json".to_owned(),
            },
            Reply {
                status: 200,
                body: "{\"nope\":1}".to_owned(),
            },
            page_reply(vec![
                json!({"operation": "query_logs", "result": {"items": []}}),
            ]),
            page_reply(vec![
                json!({"operation": "list_metrics", "result": {"items": [{"id": "m", "name": "n", "description": "d"}]}}),
            ]),
            protocol_tasks_reply(),
        ];
        for reply in &malformed {
            let result = interpret(Primitive::Metrics, reply, None);
            let fault = result.as_ref().unwrap_err();
            assert_eq!(fault_kind(fault), Kind::Malformed, "{reply:?}");
            assert!(!fault_kind(fault).configured());
        }
        assert_eq!(fault_kind(&Fault::Transport), Kind::Transport);
        assert_ne!(Kind::Transport.slug(), Kind::Unavailable.slug());
        assert_ne!(Kind::Malformed.slug(), Kind::Empty.slug());
        assert_ne!(Kind::Unauthorized.slug(), Kind::Advertised.slug());
    }

    fn second_log() -> Listed {
        interpret(
            Primitive::Logs,
            &page_reply(vec![json!({
                "operation": "list_log_sources",
                "result": {"items": [{
                    "id": "log-b",
                    "name": "B",
                    "description": "second"
                }]}
            })]),
            None,
        )
        .unwrap()
    }

    fn log_page(cursor: bool) -> Value {
        let mut result = json!({
            "items": [{
                "id": "log-a",
                "name": "App",
                "description": MARKUP,
                "asset_id": "asset-log",
                "semantic_id": "semantic-log"
            }]
        });
        if cursor {
            result["next_cursor"] = json!("1");
        }
        json!({"operation": "list_log_sources", "result": result})
    }

    fn image(id: &str) -> Value {
        json!({
            "id": id,
            "name": id,
            "description": MARKUP,
            "asset_id": "asset-image",
            "semantic_id": "semantic-image"
        })
    }

    fn page_reply(data: Vec<Value>) -> Reply {
        let artifacts = data
            .into_iter()
            .enumerate()
            .map(|(index, data)| {
                json!({
                    "artifactId": format!("art-{index}"),
                    "parts": [{"data": data, "mediaType": "application/json"}]
                })
            })
            .collect::<Vec<_>>();
        Reply {
            status: 200,
            body: json!({
                "task": {
                    "id": "protocol-task",
                    "contextId": "protocol-context",
                    "status": {"state": "TASK_STATE_COMPLETED"},
                    "artifacts": artifacts
                }
            })
            .to_string(),
        }
    }

    fn protocol_tasks_reply() -> Reply {
        Reply {
            status: 200,
            body: json!({
                "tasks": [{
                    "id": "protocol-task",
                    "contextId": "stored-context",
                    "status": {"state": "TASK_STATE_COMPLETED"}
                }]
            })
            .to_string(),
        }
    }
}
