//! HTTP+JSON request and response codec for the browser client.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Exchange(Exchange),
    Tasks(Vec<TaskRecord>),
    Status(RpcStatus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exchange {
    Task(TaskRecord),
    Message(MessageRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: String,
    pub context_id: String,
    pub state: String,
    pub artifacts: Vec<ArtifactRecord>,
    pub history: Vec<MessageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub id: String,
    pub name: String,
    pub texts: Vec<String>,
    pub data: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecord {
    pub id: String,
    pub role: String,
    pub context_id: String,
    pub task_id: String,
    pub texts: Vec<String>,
    pub data: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcStatus {
    pub code: String,
    pub status: String,
    pub message: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Unauthorized(String),
    Forbidden(String),
    Malformed,
}

pub fn text_message(id: &str, text: &str, context_id: &str) -> String {
    message(id, context_id, vec![part_text(text)])
}

pub fn command_message(id: &str, command: &Value, context_id: &str) -> Result<String, ()> {
    if !command.is_object() {
        return Err(());
    }
    Ok(message(id, context_id, vec![part_data(command)]))
}

pub fn parse_command(text: &str) -> Result<Value, ()> {
    let value: Value = serde_json::from_str(text).map_err(|_| ())?;
    value.is_object().then_some(value).ok_or(())
}

pub fn tasks_path(context_id: &str) -> String {
    let mut path = String::from("/tasks?includeArtifacts=true&historyLength=10");
    if !context_id.is_empty() {
        path.push_str("&contextId=");
        path.push_str(&percent_encode(context_id));
    }
    path
}

pub fn decode(reply: &Reply, secret: Option<&str>) -> Result<Outcome, ProtocolError> {
    if reply.status == 401 {
        return Err(ProtocolError::Unauthorized(status_message(
            &reply.body,
            secret,
        )));
    }
    if reply.status == 403 {
        return Err(ProtocolError::Forbidden(status_message(
            &reply.body,
            secret,
        )));
    }
    let value: Value = serde_json::from_str(&reply.body).map_err(|_| ProtocolError::Malformed)?;
    if let Some(status) = rpc_status(&value, secret) {
        return Ok(Outcome::Status(status));
    }
    if let Some(tasks) = value.get("tasks").and_then(Value::as_array) {
        return tasks
            .iter()
            .map(|task| task_record(task, secret))
            .collect::<Result<Vec<_>, _>>()
            .map(Outcome::Tasks);
    }
    if let Some(task) = value.get("task") {
        return task_record(task, secret).map(|task| Outcome::Exchange(Exchange::Task(task)));
    }
    if let Some(message) = value.get("message") {
        return message_record(message, secret)
            .map(|message| Outcome::Exchange(Exchange::Message(message)));
    }
    if value.get("id").and(value.get("contextId")).is_some() {
        return task_record(&value, secret).map(|task| Outcome::Exchange(Exchange::Task(task)));
    }
    Err(ProtocolError::Malformed)
}

pub fn render_outcome(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Exchange(Exchange::Task(task)) => render_task(task),
        Outcome::Exchange(Exchange::Message(message)) => render_message(message),
        Outcome::Tasks(tasks) if tasks.is_empty() => {
            "No protocol tasks in this context.".to_owned()
        }
        Outcome::Tasks(tasks) => tasks
            .iter()
            .map(render_task)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Outcome::Status(status) => render_status(status),
    }
}

pub fn context_of(outcome: &Outcome) -> Option<&str> {
    let context = match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task.context_id.as_str(),
        Outcome::Exchange(Exchange::Message(message)) => message.context_id.as_str(),
        Outcome::Tasks(tasks) => tasks
            .first()
            .map(|task| task.context_id.as_str())
            .unwrap_or(""),
        Outcome::Status(_) => "",
    };
    (!context.is_empty()).then_some(context)
}

pub fn task_id_of(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task.id.as_str(),
        Outcome::Exchange(Exchange::Message(message)) => message.task_id.as_str(),
        Outcome::Tasks(tasks) => tasks.first().map(|task| task.id.as_str()).unwrap_or(""),
        Outcome::Status(_) => "",
    }
}

pub fn exchange_turn(outcome: &Outcome) -> Option<String> {
    match outcome {
        Outcome::Exchange(Exchange::Task(task)) => Some(turn_text(task)),
        Outcome::Exchange(Exchange::Message(message)) => Some(message_lines(message)),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn stored_conversation(outcome: &Outcome) -> Result<String, ()> {
    let Outcome::Tasks(tasks) = outcome else {
        return Err(());
    };
    if tasks.is_empty() {
        return Err(());
    }
    let text = conversation_text(tasks);
    if text.is_empty() { Err(()) } else { Ok(text) }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn conversation_text(tasks: &[TaskRecord]) -> String {
    let mut tasks = tasks.to_vec();
    tasks.sort_by(|left, right| left.id.cmp(&right.id));
    tasks
        .iter()
        .map(turn_text)
        .filter(|turn| !turn.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn turn_text(task: &TaskRecord) -> String {
    let mut lines = Vec::new();
    push_role(&mut lines, "You", &texts_for(&task.history, "ROLE_USER"));
    let mut agent = artifact_texts(task);
    if agent.is_empty() {
        agent = texts_for(&task.history, "ROLE_AGENT");
    }
    push_role(&mut lines, "Agent", &agent);
    if !task.id.is_empty() {
        lines.push(format!("Task {}", task.id));
    }
    lines.join("\n")
}

pub fn redact(value: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return value.to_owned();
    };
    value
        .replace(&format!("Bearer {secret}"), "[redacted]")
        .replace(secret, "[redacted]")
}

fn message(id: &str, context_id: &str, parts: Vec<Value>) -> String {
    let mut message = Map::new();
    message.insert("messageId".to_owned(), Value::String(id.to_owned()));
    message.insert("role".to_owned(), Value::String("ROLE_USER".to_owned()));
    if !context_id.is_empty() {
        message.insert("contextId".to_owned(), Value::String(context_id.to_owned()));
    }
    message.insert("parts".to_owned(), Value::Array(parts));
    serde_json::json!({ "message": Value::Object(message) }).to_string()
}

fn part_text(text: &str) -> Value {
    serde_json::json!({ "text": text, "mediaType": "text/plain" })
}

fn part_data(data: &Value) -> Value {
    serde_json::json!({ "data": data, "mediaType": "application/json" })
}

fn task_record(value: &Value, secret: Option<&str>) -> Result<TaskRecord, ProtocolError> {
    let object = value.as_object().ok_or(ProtocolError::Malformed)?;
    let id = required(object.get("id"))?;
    let context_id = required(object.get("contextId"))?;
    let state = object
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let artifacts = object
        .get("artifacts")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| artifact(item, secret))
                .collect()
        })
        .unwrap_or_default();
    Ok(TaskRecord {
        id: redact(&id, secret),
        context_id: redact(&context_id, secret),
        state: redact(&state, secret),
        artifacts,
        history: history_of(object, secret),
    })
}

fn artifact(value: &Value, secret: Option<&str>) -> Option<ArtifactRecord> {
    let object = value.as_object()?;
    let (texts, data) = parts(object.get("parts"), secret);
    Some(ArtifactRecord {
        id: redact(&optional(object.get("artifactId")), secret),
        name: redact(&optional(object.get("name")), secret),
        texts,
        data,
    })
}

fn message_record(value: &Value, secret: Option<&str>) -> Result<MessageRecord, ProtocolError> {
    let object = value.as_object().ok_or(ProtocolError::Malformed)?;
    let (texts, data) = parts(object.get("parts"), secret);
    Ok(MessageRecord {
        id: redact(&optional(object.get("messageId")), secret),
        role: redact(&optional(object.get("role")), secret),
        context_id: redact(&optional(object.get("contextId")), secret),
        task_id: redact(&optional(object.get("taskId")), secret),
        texts,
        data,
    })
}

fn parts(value: Option<&Value>, secret: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut texts = Vec::new();
    let mut data = Vec::new();
    let Some(items) = value.and_then(Value::as_array) else {
        return (texts, data);
    };
    for item in items {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            texts.push(redact(text, secret));
        }
        if let Some(payload) = item.get("data") {
            let rendered =
                serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_owned());
            data.push(redact(&rendered, secret));
        }
    }
    (texts, data)
}

fn rpc_status(value: &Value, secret: Option<&str>) -> Option<RpcStatus> {
    let error = value.get("error")?.as_object()?;
    if error.get("message").is_none() && error.get("status").is_none() {
        return None;
    }
    let reasons = error
        .get("details")
        .and_then(Value::as_array)
        .map(|details| {
            details
                .iter()
                .filter_map(|detail| detail.get("reason").and_then(Value::as_str))
                .map(|reason| redact(reason, secret))
                .collect()
        })
        .unwrap_or_default();
    Some(RpcStatus {
        code: redact(&json_scalar(error.get("code")), secret),
        status: redact(&optional(error.get("status")), secret),
        message: redact(&optional(error.get("message")), secret),
        reasons,
    })
}

fn status_message(body: &str, secret: Option<&str>) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .as_ref()
        .and_then(|value| rpc_status(value, secret))
        .map(|status| status.message)
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "The agent rejected the call.".to_owned())
}

fn render_task(task: &TaskRecord) -> String {
    let mut lines = vec![
        format!("Task {}", task.id),
        format!("Context {}", task.context_id),
        format!("State {}", task.state),
    ];
    for artifact in &task.artifacts {
        lines.push(format!("Artifact {}", artifact.id));
        if !artifact.name.is_empty() {
            lines.push(format!("Name {}", artifact.name));
        }
        lines.extend(artifact.texts.iter().map(|text| format!("text: {text}")));
        lines.extend(artifact.data.iter().map(|data| format!("data: {data}")));
    }
    lines.join("\n")
}

fn render_message(message: &MessageRecord) -> String {
    let mut lines = vec![
        format!("Message {}", message.id),
        format!("Role {}", message.role),
        format!("Context {}", message.context_id),
    ];
    lines.extend(message.texts.iter().map(|text| format!("text: {text}")));
    lines.extend(message.data.iter().map(|data| format!("data: {data}")));
    lines.join("\n")
}

fn render_status(status: &RpcStatus) -> String {
    let mut lines = vec![
        format!("Status {}", status.status),
        format!("Code {}", status.code),
        format!("Message {}", status.message),
    ];
    lines.extend(
        status
            .reasons
            .iter()
            .map(|reason| format!("Reason {reason}")),
    );
    lines.join("\n")
}

fn history_of(object: &Map<String, Value>, secret: Option<&str>) -> Vec<MessageRecord> {
    object
        .get("history")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| message_record(item, secret).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn message_lines(message: &MessageRecord) -> String {
    let mut lines = Vec::new();
    if message.role == "ROLE_AGENT" {
        push_role(&mut lines, "Agent", &message.texts);
    } else {
        push_role(&mut lines, "You", &message.texts);
    }
    if !message.task_id.is_empty() {
        lines.push(format!("Task {}", message.task_id));
    }
    lines.join("\n")
}

fn push_role(lines: &mut Vec<String>, role: &str, texts: &[String]) {
    if texts.is_empty() {
        return;
    }
    lines.push(role.to_owned());
    lines.extend(texts.iter().cloned());
}

fn texts_for(messages: &[MessageRecord], role: &str) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == role)
        .flat_map(|message| message.texts.clone())
        .collect()
}

fn artifact_texts(task: &TaskRecord) -> Vec<String> {
    task.artifacts
        .iter()
        .flat_map(|artifact| artifact.texts.clone())
        .collect()
}

fn required(value: Option<&Value>) -> Result<String, ProtocolError> {
    value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ProtocolError::Malformed)
}

fn optional(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_owned()
}

fn json_scalar(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "page-lifetime-token";

    #[test]
    fn text_and_command_messages_use_the_http_json_envelope() {
        let text = text_message("ui-1", "hello <b>", "ctx 1");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["message"]["role"], "ROLE_USER");
        assert_eq!(value["message"]["parts"][0]["text"], "hello <b>");
        assert_eq!(value["message"]["parts"][0]["mediaType"], "text/plain");
        assert_eq!(value["message"]["contextId"], "ctx 1");
        let command = command_message(
            "ui-2",
            &json!({"operation": "list_tasks", "params": {"page": {"limit": 10}}}),
            "",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&command).unwrap();
        assert_eq!(
            value["message"]["parts"][0]["mediaType"],
            "application/json"
        );
        assert_eq!(
            value["message"]["parts"][0]["data"]["operation"],
            "list_tasks"
        );
        assert!(value["message"].get("contextId").is_none());
        assert!(parse_command("[]").is_err());
    }

    #[test]
    fn tasks_path_keeps_the_context_in_the_query_and_out_of_secrets() {
        let path = tasks_path("ctx/1");
        assert_eq!(
            path,
            "/tasks?includeArtifacts=true&historyLength=10&contextId=ctx%2F1"
        );
        assert!(!path.contains(SECRET));
    }

    #[test]
    fn decodes_task_artifact_message_and_status() {
        let task = decode(
            &Reply {
                status: 200,
                body: json!({
                    "task": {
                        "id": "task-1",
                        "contextId": "ctx-1",
                        "status": {"state": "TASK_STATE_COMPLETED"},
                        "artifacts": [{
                            "artifactId": "art-1",
                            "name": "lab-profile",
                            "parts": [
                                {"text": "<img id=\"xss\">", "mediaType": "text/plain"},
                                {"data": {"operation": "list_tasks", "result": {"items": ["<i>"]}}}
                            ]
                        }]
                    }
                })
                .to_string(),
            },
            Some(SECRET),
        )
        .unwrap();
        let rendered = render_outcome(&task);
        assert!(rendered.contains("Task task-1"));
        assert!(rendered.contains("Context ctx-1"));
        assert!(rendered.contains("TASK_STATE_COMPLETED"));
        assert!(rendered.contains("text: <img id=\"xss\">"));
        assert!(rendered.contains("list_tasks"));
        assert_eq!(context_of(&task), Some("ctx-1"));

        let tasks = decode(
            &Reply {
                status: 200,
                body: json!({"tasks": [{
                    "id": "task-1",
                    "contextId": "ctx-1",
                    "status": {"state": "TASK_STATE_WORKING"}
                }]})
                .to_string(),
            },
            None,
        )
        .unwrap();
        assert!(render_outcome(&tasks).contains("TASK_STATE_WORKING"));

        let message = decode(
            &Reply {
                status: 200,
                body: json!({"message": {
                    "messageId": "m1",
                    "role": "ROLE_AGENT",
                    "contextId": "ctx-9",
                    "parts": [{"text": "hi"}]
                }})
                .to_string(),
            },
            None,
        )
        .unwrap();
        assert!(render_outcome(&message).contains("Role ROLE_AGENT"));
    }

    #[test]
    fn auth_errors_are_distinct_and_redacted() {
        let body = json!({
            "error": {
                "code": 401,
                "status": "UNAUTHENTICATED",
                "message": format!("rejected {SECRET}"),
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "UNAUTHENTICATED",
                    "domain": "a2a-protocol.org"
                }]
            }
        })
        .to_string();
        let error = decode(
            &Reply {
                status: 401,
                body: body.clone(),
            },
            Some(SECRET),
        )
        .unwrap_err();
        let ProtocolError::Unauthorized(message) = error else {
            panic!("401");
        };
        assert!(!message.contains(SECRET));
        assert!(message.contains("[redacted]"));
        let forbidden = decode(
            &Reply {
                status: 403,
                body: body.replace("401", "403"),
            },
            Some(SECRET),
        )
        .unwrap_err();
        assert!(matches!(forbidden, ProtocolError::Forbidden(_)));
        assert!(
            decode(
                &Reply {
                    status: 200,
                    body: "{".to_owned()
                },
                None
            )
            .is_err()
        );
        assert!(matches!(
            decode(
                &Reply {
                    status: 200,
                    body: "{\"nope\":1}".to_owned()
                },
                None
            ),
            Err(ProtocolError::Malformed)
        ));
    }

    #[test]
    fn conversation_uses_history_and_artifact_text_in_task_order() {
        let outcome = decode(
            &Reply {
                status: 200,
                body: json!({"tasks": [
                    {
                        "id": "task-2",
                        "contextId": "ctx-1",
                        "status": {"state": "TASK_STATE_COMPLETED"},
                        "history": [{
                            "messageId": "m2",
                            "role": "ROLE_USER",
                            "parts": [{"text": "second"}]
                        }],
                        "artifacts": [{
                            "artifactId": "a2",
                            "name": "agent-message",
                            "parts": [{"text": "echo second"}]
                        }]
                    },
                    {
                        "id": "task-1",
                        "contextId": "ctx-1",
                        "status": {"state": "TASK_STATE_COMPLETED"},
                        "history": [{
                            "messageId": "m1",
                            "role": "ROLE_USER",
                            "parts": [{"text": "first"}]
                        }],
                        "artifacts": [{
                            "artifactId": "a1",
                            "parts": [{"text": "echo first"}]
                        }]
                    }
                ]})
                .to_string(),
            },
            None,
        )
        .unwrap();
        let text = stored_conversation(&outcome).unwrap();
        let first = text.find("first").unwrap();
        let second = text.find("second").unwrap();
        assert!(first < second);
        assert!(text.contains("echo first"));
        assert!(text.contains("echo second"));
        assert!(text.contains("Task task-1"));
        assert!(text.contains("Task task-2"));
        assert!(stored_conversation(&Outcome::Tasks(Vec::new())).is_err());
        assert!(
            stored_conversation(&Outcome::Status(RpcStatus {
                code: "1".to_owned(),
                status: "FAILED".to_owned(),
                message: "nope".to_owned(),
                reasons: Vec::new(),
            }))
            .is_err()
        );
    }
}
