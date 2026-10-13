//! Workbench behavior for the twelve lab commands.

use serde_json::{Value, json};

use super::report::{self, history_item, interpret, transport_report};
use super::tree::{self, Node};
use super::{
    BROWSER_PREVIEW_MAX_BYTES, EditorFault, Operation, PREVIEW_MEDIA_TYPES, openings, parse_params,
    preview_note,
};
use crate::codec::Reply;
use crate::inventory::Primitive;

const SECRET: &str = "page-lifetime-token";
const MARKUP: &str = r#"<img id="workbench-xss" src=x onerror="window.__workbenchXss=1">"#;
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[test]
fn every_command_has_an_editable_object_template_and_no_context_id() {
    assert_eq!(Operation::ALL.len(), 12);
    for operation in Operation::ALL {
        let params = operation.template();
        assert!(params.is_object(), "{}", operation.name());
        let body = super::envelope("ui-1", operation, &params).unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert!(value["message"].get("contextId").is_none());
        assert_eq!(
            value["message"]["parts"][0]["data"]["operation"],
            operation.name()
        );
        assert!(value["message"]["parts"][0]["data"]["params"].is_object());
        assert_eq!(Operation::parse(operation.name()), Some(operation));
    }
    assert!(preview_note().contains("image/png"));
    assert!(preview_note().contains(&BROWSER_PREVIEW_MAX_BYTES.to_string()));
    assert!(PREVIEW_MEDIA_TYPES.contains(&"image/webp"));
    assert!(!PREVIEW_MEDIA_TYPES.contains(&"image/svg+xml"));
}

#[test]
fn local_editor_rejects_only_malformed_json_and_non_objects() {
    assert_eq!(parse_params("{"), Err(EditorFault::Malformed));
    assert_eq!(parse_params("["), Err(EditorFault::Malformed));
    assert_eq!(parse_params("[]"), Err(EditorFault::NotObject));
    assert_eq!(parse_params("\"text\""), Err(EditorFault::NotObject));
    assert_eq!(parse_params("1"), Err(EditorFault::NotObject));
    assert_eq!(parse_params("null"), Err(EditorFault::NotObject));
    let domain = [
        r#"{"page":{"limit":0}}"#,
        r#"{"page":{"cursor":"next","limit":1001}}"#,
        r#"{"source_id":"","range":{"start":"2020-01-02T00:00:00Z","end":"2020-01-01T00:00:00Z"},"page":{"limit":1}}"#,
        r#"{"task_id":"build","input":[]}"#,
        r#"{"page":{"limit":1}}"#,
        r#"{"id":"<img>"}"#,
    ];
    for text in domain {
        assert!(parse_params(text).is_ok(), "{text}");
    }
}

#[test]
fn inventory_entries_prefill_stable_identifiers() {
    let log = Operation::QueryLogs.prefill("log-a");
    assert_eq!(log["source_id"], "log-a");
    assert!(log["range"].is_object());
    assert!(log["page"].is_object());
    let metric = Operation::QueryMetric.prefill("metric-a");
    assert_eq!(metric["metric_id"], "metric-a");
    let task = Operation::StartTask.prefill("task-a");
    assert_eq!(task["task_id"], "task-a");
    assert!(task["input"].is_object());
    assert!(task.get("id").is_none());
    for operation in openings(Primitive::Images) {
        let params = operation.prefill("image-a");
        assert_eq!(params["source_id"], "image-a", "{}", operation.name());
    }
    assert_eq!(openings(Primitive::Logs), &[Operation::QueryLogs]);
    assert_eq!(openings(Primitive::Metrics), &[Operation::QueryMetric]);
    assert_eq!(openings(Primitive::Tasks), &[Operation::StartTask]);
    assert!(!openings(Primitive::Tasks).contains(&Operation::GetTaskStatus));
    assert_eq!(Operation::GetImage.prefill("frame-a")["id"], "frame-a");
    assert_eq!(Operation::GetTaskStatus.prefill("run-a")["id"], "run-a");
    assert_eq!(Operation::QueryLogs.target_label(), Some("Log source"));
    assert_eq!(Operation::QueryMetric.target_label(), Some("Metric"));
    assert_eq!(Operation::StartTask.target_label(), Some("Task"));
    assert_eq!(
        Operation::GetCurrentImage.target_label(),
        Some("Image source")
    );
    assert_eq!(Operation::GetImage.target_field(), None);
    assert_eq!(Operation::GetTaskStatus.target_field(), None);
}

#[test]
fn successful_results_label_protocol_ids_separately_from_lab_runs() {
    for operation in Operation::ALL {
        let report = interpret(operation, &success_reply(operation), None);
        assert_eq!(report.kind, report::Kind::Success, "{}", operation.name());
        assert_eq!(report.operation, operation.name());
        assert_eq!(report.protocol_task_id, "protocol-task");
        assert_eq!(report.protocol_context_id, "protocol-context");
        assert_eq!(report.protocol_state, "TASK_STATE_COMPLETED");
        if operation.is_task() {
            assert_eq!(report.lab_run_id, "run-1");
            assert_eq!(report.lab_run_state, "working");
            assert_ne!(report.lab_run_id, report.protocol_task_id);
        } else {
            assert!(report.lab_run_id.is_empty(), "{}", operation.name());
            assert!(report.lab_run_state.is_empty(), "{}", operation.name());
        }
    }
}

#[test]
fn chunked_lists_keep_items_in_order_and_the_last_cursor() {
    for operation in [
        Operation::QueryLogs,
        Operation::QueryMetric,
        Operation::ListImageSources,
        Operation::ListImages,
        Operation::SearchImages,
    ] {
        let reply = Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_COMPLETED",
                "",
                vec![
                    chunk(
                        operation,
                        json!({"items": [{"id": "a"}], "next_cursor": "stale"}),
                    ),
                    chunk(
                        operation,
                        json!({"items": [{"id": "b"}], "next_cursor": "2"}),
                    ),
                ],
            ),
        };
        let report = interpret(operation, &reply, None);
        assert_eq!(report.kind, report::Kind::Success);
        let rendered = serde_json::to_string(&tree_json(&report.tree)).unwrap();
        assert!(rendered.contains("\"a\""));
        assert!(rendered.contains("\"b\""));
        assert!(rendered.contains("\"2\""));
        assert!(!rendered.contains("stale"));
    }
    let dropped = interpret(
        Operation::ListImages,
        &Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_COMPLETED",
                "",
                vec![
                    chunk(
                        Operation::ListImages,
                        json!({"items": [{"id": "a"}], "next_cursor": "keep-earlier"}),
                    ),
                    chunk(Operation::ListImages, json!({"items": [{"id": "b"}]})),
                ],
            ),
        },
        None,
    );
    let rendered = serde_json::to_string(&tree_json(&dropped.tree)).unwrap();
    assert!(rendered.contains("\"b\""));
    assert!(!rendered.contains("keep-earlier"));
}

#[test]
fn failures_keep_status_reason_domain_fields_and_correlation_ids() {
    let invalid = interpret(
        Operation::QueryLogs,
        &Reply {
            status: 400,
            body: json!({
                "error": {
                    "code": 400,
                    "status": "INVALID_ARGUMENT",
                    "message": format!("bad range {SECRET}"),
                    "details": [
                        {
                            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                            "reason": "INVALID_PARAMS",
                            "domain": "a2a-protocol.org",
                            "metadata": {"correlationId": "corr-1"}
                        },
                        {
                            "@type": "type.googleapis.com/google.rpc.BadRequest",
                            "fieldViolations": [{
                                "field": "range",
                                "description": "start must be before end"
                            }]
                        },
                        {
                            "@type": "type.googleapis.com/google.rpc.RequestInfo",
                            "requestId": "req-1"
                        }
                    ]
                }
            })
            .to_string(),
        },
        Some(SECRET),
    );
    assert_eq!(invalid.kind, report::Kind::Rpc);
    assert_eq!(invalid.status, "INVALID_ARGUMENT");
    assert_eq!(invalid.code, "400");
    assert!(!invalid.message.contains(SECRET));
    assert_eq!(invalid.reasons[0].reason, "INVALID_PARAMS");
    assert_eq!(invalid.reasons[0].domain, "a2a-protocol.org");
    assert_eq!(invalid.fields[0].field, "range");
    assert!(invalid.fields[0].description.contains("before end"));
    assert!(
        invalid
            .correlations
            .iter()
            .any(|item| item.value == "req-1")
    );
    assert!(
        invalid
            .correlations
            .iter()
            .any(|item| item.value == "corr-1")
    );
    assert_ne!(invalid.message, invalid.fields[0].description);

    let denied = interpret(
        Operation::ListTasks,
        &Reply {
            status: 401,
            body: json!({
                "error": {
                    "code": 401,
                    "status": "UNAUTHENTICATED",
                    "message": SECRET,
                    "details": [{"reason": "INVALID_REQUEST", "domain": "a2a-protocol.org"}]
                }
            })
            .to_string(),
        },
        Some(SECRET),
    );
    assert_eq!(denied.kind, report::Kind::Unauthorized);
    assert!(!denied.message.contains(SECRET));
    assert_eq!(denied.reasons[0].domain, "a2a-protocol.org");

    let forbidden = interpret(
        Operation::ListMetrics,
        &Reply {
            status: 403,
            body: json!({"error": {"code": 403, "status": "PERMISSION_DENIED", "message": "no"}})
                .to_string(),
        },
        None,
    );
    assert_eq!(forbidden.kind, report::Kind::Forbidden);

    let unavailable = interpret(
        Operation::GetImage,
        &Reply {
            status: 500,
            body: json!({
                "error": {
                    "code": 500,
                    "status": "INTERNAL",
                    "message": "image provider is not configured",
                    "details": [{"reason": "INTERNAL_ERROR", "domain": "a2a-protocol.org"}]
                }
            })
            .to_string(),
        },
        None,
    );
    assert_eq!(unavailable.kind, report::Kind::Unavailable);
    assert_eq!(unavailable.reasons[0].reason, "INTERNAL_ERROR");

    let unsupported = interpret(
        Operation::GetCurrentImage,
        &Reply {
            status: 415,
            body: json!({
                "error": {
                    "code": 415,
                    "status": "INVALID_ARGUMENT",
                    "message": "content type not supported",
                    "details": [{
                        "reason": "CONTENT_TYPE_NOT_SUPPORTED",
                        "domain": "a2a-protocol.org"
                    }]
                }
            })
            .to_string(),
        },
        None,
    );
    assert_eq!(unsupported.kind, report::Kind::Unsupported);

    let failed = interpret(
        Operation::StartTask,
        &Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_FAILED",
                "run exploded",
                vec![chunk(
                    Operation::StartTask,
                    json!({"id": "run-9", "task_id": "build", "state": "failed"}),
                )],
            ),
        },
        None,
    );
    assert_eq!(failed.kind, report::Kind::TaskFailed);
    assert_eq!(failed.protocol_task_id, "protocol-task");
    assert_eq!(failed.protocol_state, "TASK_STATE_FAILED");
    assert_eq!(failed.message, "run exploded");
    assert_eq!(failed.lab_run_id, "run-9");
    assert_eq!(failed.lab_run_state, "failed");

    let unexpected = interpret(
        Operation::QueryLogs,
        &Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_COMPLETED",
                "",
                vec![chunk(Operation::ListTasks, json!({"items": []}))],
            ),
        },
        None,
    );
    assert_eq!(unexpected.kind, report::Kind::Unexpected);
    assert_eq!(unexpected.protocol_task_id, "protocol-task");

    let malformed = interpret(
        Operation::ListLogSources,
        &Reply {
            status: 200,
            body: "not-json".to_owned(),
        },
        None,
    );
    assert_eq!(malformed.kind, report::Kind::Unexpected);
    assert_eq!(
        transport_report(Operation::ListMetrics).kind,
        report::Kind::Transport
    );
}

#[test]
fn markup_large_values_and_images_stay_bounded_and_unexecuted() {
    let markup = interpret(
        Operation::ListLogSources,
        &Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_COMPLETED",
                "",
                vec![chunk(
                    Operation::ListLogSources,
                    json!({"items": [{"id": "log-a", "name": MARKUP}]}),
                )],
            ),
        },
        None,
    );
    assert!(tree::contains_text(&markup.tree, MARKUP));
    assert!(tree::contains_text(&markup.tree, "onerror"));

    let mut deep = json!("leaf");
    for _ in 0..8 {
        deep = json!({"child": deep});
    }
    let nested = tree::from_value(&deep);
    assert!(has_collapsed(&nested));
    assert!(tree::contains_text(&nested, "leaf"));

    let long = "x".repeat(500);
    let wide = json!({"note": long});
    let bounded = tree::from_value(&wide);
    assert!(has_collapsed(&bounded));
    assert!(tree::contains_text(&bounded, &long));

    let png = image_report(Operation::GetImage, "image/png", PNG);
    assert_eq!(png.kind, report::Kind::Success);
    let image = png.image.unwrap();
    assert!(
        image
            .preview_url
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert!(!tree::contains_text(&png.tree, PNG));
    assert!(tree::contains_text(&png.tree, "base64"));

    let svg = image_report(Operation::GetCurrentImage, "image/svg+xml", PNG);
    let image = svg.image.unwrap();
    assert!(image.preview_url.is_none());
    assert!(image.withheld.contains("allowlist"));
    assert!(!tree::contains_text(&svg.tree, PNG));

    let huge = "A".repeat((BROWSER_PREVIEW_MAX_BYTES + 3) / 3 * 4);
    let oversized = image_report(Operation::GetImage, "image/gif", &huge);
    let image = oversized.image.unwrap();
    assert!(image.decoded_bytes.unwrap() > BROWSER_PREVIEW_MAX_BYTES);
    assert!(image.preview_url.is_none());
    assert!(image.withheld.contains("ceiling"));
    assert!(!tree::contains_text(&oversized.tree, &huge));
}

#[test]
fn history_summaries_are_sanitized_and_retryable_without_credentials() {
    let report = interpret(
        Operation::GetImage,
        &success_reply(Operation::GetImage),
        Some(SECRET),
    );
    let params = json!({"id": SECRET, "note": format!("Bearer {SECRET}")});
    let item = history_item(Operation::GetImage, &params, &report, Some(SECRET));
    assert!(!item.summary.contains(SECRET));
    assert!(!item.params.to_string().contains(SECRET));
    assert!(!item.summary.contains(PNG));
    assert_eq!(item.operation, Operation::GetImage);
    assert!(item.params["id"].is_string());
    let again = super::envelope("ui-2", item.operation, &item.params).unwrap();
    assert!(!again.contains(SECRET));
    assert!(again.contains("[redacted]"));
}

fn success_reply(operation: Operation) -> Reply {
    let result = if operation.is_task() {
        json!({"id": "run-1", "task_id": "build", "state": "working"})
    } else if matches!(operation, Operation::GetImage | Operation::GetCurrentImage) {
        json!({
            "descriptor": {"media_type": "image/png", "id": "frame-1"},
            "data": PNG
        })
    } else {
        json!({"items": [{"id": "item-1", "name": MARKUP}]})
    };
    Reply {
        status: 200,
        body: task_body("TASK_STATE_COMPLETED", "", vec![chunk(operation, result)]),
    }
}

fn image_report(operation: Operation, media_type: &str, data: &str) -> report::Report {
    interpret(
        operation,
        &Reply {
            status: 200,
            body: task_body(
                "TASK_STATE_COMPLETED",
                "",
                vec![chunk(
                    operation,
                    json!({
                        "descriptor": {"media_type": media_type, "id": "frame"},
                        "data": data
                    }),
                )],
            ),
        },
        None,
    )
}

fn chunk(operation: Operation, result: Value) -> Value {
    json!({"operation": operation.name(), "result": result})
}

fn task_body(state: &str, message: &str, chunks: Vec<Value>) -> String {
    let artifacts: Vec<Value> = chunks
        .into_iter()
        .enumerate()
        .map(|(index, data)| {
            json!({
                "artifactId": format!("art-{index}"),
                "parts": [{"data": data}]
            })
        })
        .collect();
    let mut status = json!({"state": state});
    if !message.is_empty() {
        status["message"] = json!(message);
    }
    json!({
        "task": {
            "id": "protocol-task",
            "contextId": "protocol-context",
            "status": status,
            "artifacts": artifacts
        }
    })
    .to_string()
}

fn has_collapsed(node: &Node) -> bool {
    match node {
        Node::Text(_) => false,
        Node::Group {
            collapsed,
            children,
            ..
        } => *collapsed || children.iter().any(has_collapsed),
    }
}

fn tree_json(node: &Node) -> Value {
    match node {
        Node::Text(text) => Value::String(text.clone()),
        Node::Group { name, children, .. } => json!({
            "name": name,
            "children": children.iter().map(tree_json).collect::<Vec<_>>()
        }),
    }
}
