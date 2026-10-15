use serde_json::{Value, json};

use super::{Kind, attrs, form, form_for, schema_of};
use crate::inventory::Item;

const SECRET: &str = "page-lifetime-token";
const MARKUP: &str = r#"<img id="task-schema-xss" src=x onerror="window.__taskSchemaXss=1">"#;

#[test]
fn nested_objects_preserve_schema_structure() {
    let mut form = form(
        Some(
            r#"{
                "type": "object",
                "title": "Build",
                "properties": {
                    "name": {"type": "string", "title": "Name"},
                    "config": {
                        "type": "object",
                        "title": "Config",
                        "properties": {
                            "retries": {"type": "integer", "title": "Retries"},
                            "nested": {
                                "type": "object",
                                "properties": {
                                    "flag": {"type": "boolean"}
                                }
                            }
                        }
                    }
                }
            }"#,
        ),
        None,
    );
    assert_eq!(form.root.kind, Kind::Object);
    assert_eq!(form.field("name").unwrap().label, "Name");
    assert_eq!(form.field("name").unwrap().kind.widget(), "text");
    assert_eq!(form.field("config").unwrap().label, "Config");
    assert_eq!(form.field("config").unwrap().kind.widget(), "group");
    assert_eq!(
        form.field("config.nested.flag").unwrap().path,
        "config.nested.flag"
    );
    form.set("name", "job");
    form.set("config.retries", "2");
    form.set("config.nested.flag", "true");
    assert_eq!(
        form.submit().unwrap(),
        json!({
            "name": "job",
            "config": {
                "retries": 2,
                "nested": {"flag": true}
            }
        })
    );
}

#[test]
fn arrays_add_and_remove_item_values() {
    let mut form = form(
        Some(
            r#"{
                "type": "object",
                "properties": {
                    "tags": {
                        "type": "array",
                        "title": "Tags",
                        "items": {"type": "string"}
                    },
                    "ports": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "n": {"type": "integer"}
                            }
                        }
                    }
                }
            }"#,
        ),
        None,
    );
    assert_eq!(form.field("tags").unwrap().kind.widget(), "list");
    form.add("tags");
    form.add("tags");
    form.set("tags.0", "alpha");
    form.set("tags.1", "beta");
    form.add("ports");
    form.set("ports.0.n", "80");
    assert_eq!(
        form.submit().unwrap(),
        json!({"tags": ["alpha", "beta"], "ports": [{"n": 80}]})
    );
    form.remove("tags.0");
    assert_eq!(form.field("tags.0").unwrap().raw, "beta");
    assert_eq!(form.field("tags.0").unwrap().path, "tags.0");
    assert_eq!(form.field("tags.0").unwrap().label, "Item 1");
    assert_eq!(
        form.submit().unwrap(),
        json!({"tags": ["beta"], "ports": [{"n": 80}]})
    );
}

#[test]
fn scalar_controls_produce_json_types() {
    let mut form = form(
        Some(
            r#"{
                "type": "object",
                "properties": {
                    "note": {"type": "string"},
                    "day": {"type": "string", "format": "date"},
                    "clock": {"type": "string", "format": "time"},
                    "when": {"type": "string", "format": "date-time"},
                    "count": {"type": "integer"},
                    "ratio": {"type": "number"},
                    "on": {"type": "boolean"},
                    "mode": {"type": "string", "enum": ["fast", "slow"]}
                }
            }"#,
        ),
        None,
    );
    let widgets = [
        ("note", "text", Some("text")),
        ("day", "date", Some("date")),
        ("clock", "time", Some("time")),
        ("when", "datetime-local", Some("datetime-local")),
        ("count", "number", Some("number")),
        ("ratio", "number", Some("number")),
        ("on", "checkbox", Some("checkbox")),
        ("mode", "select", None),
    ];
    for (path, widget, input) in widgets {
        let field = form.field(path).unwrap();
        assert_eq!(field.kind.widget(), widget, "{path}");
        assert_eq!(field.kind.input_type(), input, "{path}");
    }
    form.set("note", "hello");
    form.set("day", "2024-01-15");
    form.set("clock", "13:45:00");
    form.set("when", "2024-01-15T13:45");
    form.set("count", "3");
    form.set("ratio", "1.5");
    form.set("on", "true");
    form.set("mode", "fast");
    let value = form.submit().unwrap();
    assert_eq!(value["note"], "hello");
    assert_eq!(value["day"], "2024-01-15");
    assert_eq!(value["clock"], "13:45:00");
    assert_eq!(value["when"], "2024-01-15T13:45:00Z");
    assert_eq!(value["count"], 3);
    assert_eq!(value["ratio"], 1.5);
    assert_eq!(value["on"], true);
    assert_eq!(value["mode"], "fast");
    assert!(value["count"].is_i64() || value["count"].is_u64());
    assert!(value["ratio"].is_number());
    assert!(value["on"].is_boolean());
}

#[test]
fn required_defaults_and_constraints_are_on_controls() {
    let mut form = form(
        Some(
            r#"{
                "type": "object",
                "required": ["name", "count"],
                "properties": {
                    "name": {
                        "type": "string",
                        "title": "Name",
                        "minLength": 2,
                        "maxLength": 8,
                        "pattern": "^lab$",
                        "default": "lab"
                    },
                    "count": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 9,
                        "exclusiveMinimum": 1,
                        "exclusiveMaximum": 9,
                        "errorMessage": "Count is out of range."
                    },
                    "ratio": {
                        "type": "number",
                        "minimum": 0.5,
                        "maximum": 2.5
                    }
                }
            }"#,
        ),
        None,
    );
    let name_attrs = {
        let name = form.field("name").unwrap();
        assert!(name.required);
        assert_eq!(name.raw, "lab");
        attrs(name)
    };
    assert!(name_attrs.contains(&("required", "required".to_owned())));
    assert!(name_attrs.contains(&("aria-required", "true".to_owned())));
    assert!(name_attrs.contains(&("minlength", "2".to_owned())));
    assert!(name_attrs.contains(&("maxlength", "8".to_owned())));
    assert!(name_attrs.contains(&("pattern", "^lab$".to_owned())));
    let count_attrs = attrs(form.field("count").unwrap());
    assert!(count_attrs.contains(&("min", "1".to_owned())));
    assert!(count_attrs.contains(&("max", "9".to_owned())));
    assert!(count_attrs.contains(&("data-exclusive-minimum", "1".to_owned())));
    assert!(count_attrs.contains(&("data-exclusive-maximum", "9".to_owned())));
    form.set("count", "5");
    assert_eq!(form.submit().unwrap()["name"], "lab");

    form.set("name", "no");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("name").unwrap().error.as_deref(),
        Some("Must match the required pattern.")
    );
    form.set("name", "lab");
    form.set("count", "1");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("count").unwrap().error.as_deref(),
        Some("Count is out of range.")
    );
    form.set("count", "2");
    form.set("ratio", "0.1");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("ratio").unwrap().error.as_deref(),
        Some("Must be at least 0.5.")
    );
    form.set("ratio", "1");
    form.set("name", "");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("name").unwrap().error.as_deref(),
        Some("This field is required.")
    );
}

#[test]
fn unsupported_schema_uses_json_fallback() {
    let mut form = form(
        Some(
            r#"{
                "type": "object",
                "properties": {
                    "plain": {"type": "string", "title": "Plain"},
                    "choice": {
                        "title": "Choice",
                        "oneOf": [
                            {"type": "string"},
                            {"type": "number"}
                        ]
                    }
                }
            }"#,
        ),
        None,
    );
    assert_eq!(form.field("plain").unwrap().kind, Kind::Text);
    {
        let fallback = form.field("choice").unwrap();
        assert_eq!(fallback.kind, Kind::Json);
        assert_eq!(fallback.kind.widget(), "textarea");
        assert_eq!(fallback.label, "Choice");
    }
    form.set("plain", "ok");
    form.set("choice", "not-json");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("choice").unwrap().error.as_deref(),
        Some("Enter valid JSON.")
    );
    form.set("choice", r#"{"picked":1}"#);
    assert_eq!(
        form.submit().unwrap(),
        json!({"plain": "ok", "choice": {"picked": 1}})
    );
}

#[test]
fn missing_schema_submits_empty_object() {
    let mut missing = form(None, None);
    assert!(!missing.needs_input());
    assert_eq!(missing.root.children.len(), 0);
    assert_eq!(missing.submit().unwrap(), json!({}));

    let mut blank = form(Some("  "), None);
    assert_eq!(blank.submit().unwrap(), json!({}));

    let item = Item {
        id: "task-a".to_owned(),
        name: "Build".to_owned(),
        description: "Build".to_owned(),
        details: Vec::new(),
    };
    assert!(schema_of(&item).is_none());
    let mut from_item = form_for(&item, None);
    assert_eq!(from_item.submit().unwrap(), json!({}));

    let mut empty_object = form(Some(r#"{"type":"object","properties":{}}"#), None);
    assert!(!empty_object.needs_input());
    assert_eq!(empty_object.submit().unwrap(), json!({}));
}

#[test]
fn provider_text_is_inert_and_bearer_is_redacted() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "title": MARKUP,
                "description": format!("Use {SECRET}"),
                "default": format!("Bearer {SECRET}"),
                "errorMessage": format!("bad {SECRET}"),
                "minLength": 8
            },
            "mode": {
                "type": "string",
                "enum": [MARKUP, format!("Bearer {SECRET}")]
            }
        }
    })
    .to_string();
    let item = Item {
        id: "task-a".to_owned(),
        name: MARKUP.to_owned(),
        description: "Build".to_owned(),
        details: vec![("Input schema", schema)],
    };
    let mut form = form_for(&item, Some(SECRET));
    {
        let name = form.field("name").unwrap();
        assert_eq!(name.label, MARKUP);
        assert!(name.description.contains("[redacted]"));
        assert!(!name.description.contains(SECRET));
        assert_eq!(name.raw, "[redacted]");
        assert!(!name.raw.contains(SECRET));
        assert_eq!(name.error_message.as_deref(), Some("bad [redacted]"));
        let mode = form.field("mode").unwrap();
        assert_eq!(mode.options[0], Value::String(MARKUP.to_owned()));
        assert_eq!(mode.options[1], Value::String("[redacted]".to_owned()));
    }
    form.set("name", "short");
    assert!(form.submit().is_none());
    assert_eq!(
        form.field("name").unwrap().error.as_deref(),
        Some("bad [redacted]")
    );
    assert!(
        !form
            .field("name")
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains(SECRET)
    );
}

#[test]
fn widgets_expose_accessible_ids_and_invalid_state() {
    let mut form = form(
        Some(
            r#"{"type":"object","required":["n"],"properties":{"n":{"type":"integer","title":"N"}}}"#,
        ),
        None,
    );
    {
        let field = form.field("n").unwrap();
        assert_eq!(field.id(), "task-input-n");
        assert_eq!(field.help_id(), "task-input-n-help");
        assert_eq!(field.error_id(), "task-input-n-error");
        assert!(attrs(field).iter().any(|(name, _)| *name == "type"));
    }
    assert!(form.submit().is_none());
    let attrs = attrs(form.field("n").unwrap());
    assert!(attrs.contains(&("aria-invalid", "true".to_owned())));
}
