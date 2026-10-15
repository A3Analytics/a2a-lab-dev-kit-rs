//! Start and status commands for lab task runs.

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::Fault;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub task_id: String,
    pub state: String,
    pub message: String,
    pub result: String,
    pub progress: String,
    pub error_kind: String,
    pub error_identifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub seq: u32,
    pub poll: u32,
    pub run: Run,
}

const STATES: [&str; 5] = ["submitted", "working", "completed", "failed", "canceled"];

impl Run {
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "failed" | "canceled")
    }
}

pub fn start_command(task_id: &str, input: &Value) -> Value {
    serde_json::json!({
        "operation": "start_task",
        "params": {
            "task_id": task_id,
            "input": input,
            "wait": false
        }
    })
}

pub fn status_command(run_id: &str) -> Value {
    serde_json::json!({
        "operation": "get_task_status",
        "params": { "id": run_id }
    })
}

pub fn interpret(reply: &Reply, secret: Option<&str>) -> Result<Run, Fault> {
    match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => Err(status_fault(&status)),
        Ok(outcome) => run_from(&outcome, secret),
    }
}

pub fn apply(runs: &mut Vec<Tracked>, run: Run, seq: u32) -> bool {
    if let Some(existing) = runs.iter_mut().find(|item| item.run.id == run.id) {
        if seq <= existing.seq {
            return false;
        }
        existing.seq = seq;
        existing.run = run;
        return true;
    }
    runs.insert(0, Tracked { seq, poll: 0, run });
    true
}

pub fn begin_poll(runs: &mut [Tracked], id: &str) -> Option<u32> {
    let tracked = runs.iter_mut().find(|item| item.run.id == id)?;
    tracked.poll = tracked.poll.wrapping_add(1);
    Some(tracked.poll)
}

pub fn poll_live(runs: &[Tracked], id: &str, generation: u32) -> bool {
    runs.iter()
        .any(|item| item.run.id == id && item.poll == generation)
}

pub fn next_seq(runs: &[Tracked], id: &str) -> u32 {
    runs.iter()
        .find(|item| item.run.id == id)
        .map(|item| item.seq.wrapping_add(1))
        .unwrap_or(1)
}

fn status_fault(status: &codec::RpcStatus) -> Fault {
    if status.status == "UNAUTHENTICATED"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "UNAUTHENTICATED")
    {
        return Fault::Unauthorized(status.message.clone());
    }
    if status.status == "PERMISSION_DENIED"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "PERMISSION_DENIED")
    {
        return Fault::Forbidden(status.message.clone());
    }
    let unavailable = status.status == "INTERNAL"
        || status.status == "UNAVAILABLE"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "INTERNAL_ERROR");
    if unavailable || !status.message.is_empty() {
        return Fault::Unavailable(status.message.clone());
    }
    Fault::Malformed
}

fn run_from(outcome: &Outcome, secret: Option<&str>) -> Result<Run, Fault> {
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
    let mut last = None;
    for payload in payloads {
        last = Some(parse_payload(&payload, secret)?);
    }
    last.ok_or(Fault::Malformed)
}

fn parse_payload(payload: &str, secret: Option<&str>) -> Result<Run, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or(Fault::Malformed)?;
    if operation != "start_task" && operation != "get_task_status" {
        return Err(Fault::Malformed);
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    parse_run(result, secret)
}

fn parse_run(object: &Map<String, Value>, secret: Option<&str>) -> Result<Run, Fault> {
    let state = required_field(object, "state")?;
    if !STATES.contains(&state.as_str()) {
        return Err(Fault::Malformed);
    }
    Ok(Run {
        id: text_of(&required_field(object, "id")?, secret),
        task_id: text_of(&required_field(object, "task_id")?, secret),
        state: text_of(&state, secret),
        message: optional_text(object.get("message"), secret)?,
        result: optional_json(object.get("result"), secret)?,
        progress: optional_progress(object.get("progress"), secret)?,
        error_kind: optional_text(object.get("error_kind"), secret)?,
        error_identifier: optional_text(object.get("error_identifier"), secret)?,
    })
}

fn required_field(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(Fault::Malformed)
}

fn optional_text(value: Option<&Value>, secret: Option<&str>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text_of(text, secret)),
        Some(_) => Err(Fault::Malformed),
    }
}

fn optional_json(value: Option<&Value>, secret: Option<&str>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::Object(fields)) if fields.is_empty() => Ok(String::new()),
        Some(object @ Value::Object(_)) => {
            let rendered = serde_json::to_string(object).map_err(|_| Fault::Malformed)?;
            Ok(text_of(&rendered, secret))
        }
        Some(_) => Err(Fault::Malformed),
    }
}

fn optional_progress(value: Option<&Value>, secret: Option<&str>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::Number(number)) => Ok(text_of(&number.to_string(), secret)),
        Some(_) => Err(Fault::Malformed),
    }
}

fn text_of(value: &str, secret: Option<&str>) -> String {
    codec::redact(value, secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "page-lifetime-token";

    #[test]
    fn start_command_sets_wait_false_and_typed_input() {
        let empty = start_command("home", &json!({}));
        assert_eq!(empty["operation"], "start_task");
        assert_eq!(empty["params"]["task_id"], "home");
        assert_eq!(empty["params"]["wait"], false);
        assert_eq!(empty["params"]["input"], json!({}));
        let body = codec::command_message("ui-1", &empty, "").unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert!(value["message"].get("contextId").is_none());

        let structured = start_command("build", &json!({"name": "job", "count": 2}));
        assert_eq!(structured["params"]["input"]["name"], "job");
        assert_eq!(structured["params"]["input"]["count"], 2);
        assert!(
            structured["params"]["input"]["count"].is_i64()
                || structured["params"]["input"]["count"].is_u64()
        );
    }

    #[test]
    fn status_command_reads_lab_run_id() {
        let command = status_command("run-1");
        assert_eq!(command["operation"], "get_task_status");
        assert_eq!(command["params"]["id"], "run-1");
        let body = codec::command_message("ui-2", &command, "").unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert!(value["message"].get("contextId").is_none());
    }

    #[test]
    fn interpret_reads_progress_result_and_terminal_states() {
        let working = interpret(
            &run_reply(
                "get_task_status",
                json!({
                    "id": "run-1",
                    "task_id": "build",
                    "state": "working",
                    "progress": 0.5,
                    "message": "halfway"
                }),
            ),
            None,
        )
        .unwrap();
        assert_eq!(working.state, "working");
        assert_eq!(working.progress, "0.5");
        assert_eq!(working.message, "halfway");
        assert!(!working.terminal());

        for (state, extra) in [
            (
                "completed",
                json!({"result": {"ok": true}, "message": "done"}),
            ),
            (
                "failed",
                json!({
                    "error_kind": "execution",
                    "error_identifier": "boom",
                    "message": "nope"
                }),
            ),
            ("canceled", json!({"message": "stopped"})),
        ] {
            let mut result = json!({
                "id": "run-1",
                "task_id": "build",
                "state": state
            });
            if let Some(object) = extra.as_object() {
                for (key, value) in object {
                    result[key] = value.clone();
                }
            }
            let run = interpret(&run_reply("get_task_status", result), None).unwrap();
            assert_eq!(run.state, state);
            assert!(run.terminal());
            if state == "completed" {
                assert!(run.result.contains("\"ok\":true") || run.result.contains("\"ok\": true"));
            }
            if state == "failed" {
                assert_eq!(run.error_kind, "execution");
                assert_eq!(run.error_identifier, "boom");
            }
        }
    }

    #[test]
    fn interpret_redacts_bearer_and_maps_faults() {
        let run = interpret(
            &run_reply(
                "start_task",
                json!({
                    "id": "run-1",
                    "task_id": "build",
                    "state": "submitted",
                    "message": format!("Use {SECRET}"),
                    "result": {"token": format!("Bearer {SECRET}")},
                    "error_kind": format!("bad {SECRET}")
                }),
            ),
            Some(SECRET),
        )
        .unwrap();
        assert!(run.message.contains("[redacted]"));
        assert!(!run.message.contains(SECRET));
        assert!(run.result.contains("[redacted]"));
        assert!(!run.result.contains(SECRET));
        assert!(run.error_kind.contains("[redacted]"));

        assert!(matches!(
            interpret(
                &Reply {
                    status: 401,
                    body: "{}".into()
                },
                None
            ),
            Err(Fault::Unauthorized(_))
        ));
        assert!(matches!(
            interpret(
                &Reply {
                    status: 403,
                    body: "{}".into()
                },
                None
            ),
            Err(Fault::Forbidden(_))
        ));
        assert_eq!(
            interpret(
                &Reply {
                    status: 200,
                    body: "not-json".into()
                },
                None
            ),
            Err(Fault::Malformed)
        );
        let rejected = interpret(
            &Reply {
                status: 200,
                body: json!({
                    "error": {
                        "code": 5,
                        "status": "NOT_FOUND",
                        "message": "task `missing` was not found",
                        "details": [{"reason": "NOT_FOUND"}]
                    }
                })
                .to_string(),
            },
            None,
        );
        assert_eq!(
            rejected,
            Err(Fault::Unavailable("task `missing` was not found".into()))
        );
        let unavailable = interpret(
            &Reply {
                status: 200,
                body: json!({
                    "error": {
                        "code": 13,
                        "status": "INTERNAL",
                        "message": "task provider is not configured",
                        "details": [{"reason": "INTERNAL_ERROR"}]
                    }
                })
                .to_string(),
            },
            None,
        );
        assert_eq!(
            unavailable,
            Err(Fault::Unavailable("task provider is not configured".into()))
        );
    }

    #[test]
    fn apply_ignores_stale_duplicates_and_keeps_cards_apart() {
        let mut runs = Vec::new();
        assert!(apply(&mut runs, sample("run-1", "submitted"), 1));
        assert!(!apply(&mut runs, sample("run-1", "working"), 1));
        assert_eq!(runs[0].run.state, "submitted");
        assert!(apply(&mut runs, sample("run-1", "working"), 2));
        assert_eq!(runs[0].run.state, "working");
        assert!(!apply(&mut runs, sample("run-1", "submitted"), 1));
        assert_eq!(runs[0].run.state, "working");
        assert!(apply(&mut runs, sample("run-2", "submitted"), 1));
        assert_eq!(runs[0].run.id, "run-2");
        assert_eq!(runs[1].run.id, "run-1");
        assert!(!apply(&mut runs, sample("run-1", "completed"), 2));
        assert_eq!(runs[1].run.state, "working");
        assert_eq!(runs[0].run.state, "submitted");

        let generation = begin_poll(&mut runs, "run-1").unwrap();
        assert!(poll_live(&runs, "run-1", generation));
        assert!(!poll_live(&runs, "run-2", generation));
        let next = begin_poll(&mut runs, "run-1").unwrap();
        assert_ne!(next, generation);
        assert!(!poll_live(&runs, "run-1", generation));
        assert!(poll_live(&runs, "run-1", next));
        assert_eq!(next_seq(&runs, "run-1"), 3);
        assert_eq!(next_seq(&runs, "missing"), 1);
    }

    fn sample(id: &str, state: &str) -> Run {
        Run {
            id: id.to_owned(),
            task_id: "build".to_owned(),
            state: state.to_owned(),
            message: String::new(),
            result: String::new(),
            progress: String::new(),
            error_kind: String::new(),
            error_identifier: String::new(),
        }
    }

    fn run_reply(operation: &str, result: Value) -> Reply {
        Reply {
            status: 200,
            body: json!({
                "task": {
                    "id": "protocol-task",
                    "contextId": "protocol-context",
                    "status": {"state": "TASK_STATE_COMPLETED"},
                    "artifacts": [{
                        "artifactId": "art-0",
                        "parts": [{
                            "data": {"operation": operation, "result": result},
                            "mediaType": "application/json"
                        }]
                    }]
                }
            })
            .to_string(),
        }
    }
}
