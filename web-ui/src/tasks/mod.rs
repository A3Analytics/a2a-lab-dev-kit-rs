//! Task-input form from a JSON Schema.

#![allow(dead_code)]

use serde_json::{Map, Value};

use crate::codec;
use crate::inventory::Item;

mod run;

#[cfg(test)]
mod tests;

#[cfg_attr(not(target_arch = "wasm32"), allow(unused_imports))]
pub use run::{Run, Tracked, apply, interpret, next_seq, start_command, status_command};

const UNSUPPORTED: [&str; 12] = [
    "$ref",
    "allOf",
    "anyOf",
    "const",
    "contentEncoding",
    "if",
    "not",
    "oneOf",
    "patternProperties",
    "prefixItems",
    "then",
    "else",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub root: Field,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub path: String,
    pub key: String,
    pub label: String,
    pub description: String,
    pub required: bool,
    pub kind: Kind,
    pub raw: String,
    pub error: Option<String>,
    pub options: Vec<Value>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<String>,
    pub minimum: Option<String>,
    pub maximum: Option<String>,
    pub exclusive_minimum: Option<String>,
    pub exclusive_maximum: Option<String>,
    pub error_message: Option<String>,
    pub children: Vec<Field>,
    pub item: Option<Box<Field>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Object,
    Array,
    Text,
    Date,
    Time,
    DateTime,
    Integer,
    Number,
    Boolean,
    Enum,
    Json,
}

impl Kind {
    pub const fn widget(self) -> &'static str {
        match self {
            Self::Object => "group",
            Self::Array => "list",
            Self::Text => "text",
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime-local",
            Self::Integer | Self::Number => "number",
            Self::Boolean => "checkbox",
            Self::Enum => "select",
            Self::Json => "textarea",
        }
    }

    pub const fn input_type(self) -> Option<&'static str> {
        match self {
            Self::Text => Some("text"),
            Self::Date => Some("date"),
            Self::Time => Some("time"),
            Self::DateTime => Some("datetime-local"),
            Self::Integer | Self::Number => Some("number"),
            Self::Boolean => Some("checkbox"),
            _ => None,
        }
    }
}

pub fn schema_of(item: &Item) -> Option<&str> {
    item.details.iter().find_map(|(name, value)| {
        (*name == "Input schema")
            .then_some(value.as_str())
            .filter(|value| !value.trim().is_empty())
    })
}

pub fn form_for(item: &Item, secret: Option<&str>) -> Form {
    form(schema_of(item), secret)
}

pub fn form(schema: Option<&str>, secret: Option<&str>) -> Form {
    let Some(schema) = schema.map(str::trim).filter(|schema| !schema.is_empty()) else {
        return Form {
            root: Field::new("", "", "Input", Kind::Object, true),
        };
    };
    match serde_json::from_str::<Value>(schema) {
        Ok(value) => Form {
            root: field_from(&value, "", "", "Input", true, secret),
        },
        Err(_) => Form {
            root: json_field("", "", "Input", true, &Value::Null, secret),
        },
    }
}

impl Form {
    pub fn needs_input(&self) -> bool {
        !self.root.children.is_empty() || self.root.kind == Kind::Json
    }

    pub fn field(&self, path: &str) -> Option<&Field> {
        find(&self.root, path)
    }

    pub fn submit(&mut self) -> Option<Value> {
        match read(&mut self.root) {
            Ok(Some(value)) => Some(value),
            Ok(None) => Some(Value::Object(Map::new())),
            Err(()) => None,
        }
    }

    pub fn set(&mut self, path: &str, value: &str) {
        if let Some(field) = find_mut(&mut self.root, path) {
            field.error = None;
            field.raw = raw_value(field.kind, value);
        }
    }

    pub fn add(&mut self, path: &str) {
        let Some(field) = find_mut(&mut self.root, path) else {
            return;
        };
        if field.kind != Kind::Array {
            return;
        }
        let Some(template) = field.item.clone() else {
            return;
        };
        let index = field.children.len();
        let mut child = *template;
        retarget(
            &mut child,
            &join(&field.path, &index.to_string()),
            &index.to_string(),
        );
        child.label = format!("Item {}", index + 1);
        field.children.push(child);
    }

    pub fn remove(&mut self, path: &str) {
        let Some((parent, index)) = split_index(path) else {
            return;
        };
        let Some(field) = find_mut(&mut self.root, parent) else {
            return;
        };
        let Ok(index) = index.parse::<usize>() else {
            return;
        };
        if index >= field.children.len() {
            return;
        }
        field.children.remove(index);
        let base = field.path.clone();
        for (index, child) in field.children.iter_mut().enumerate() {
            retarget(child, &join(&base, &index.to_string()), &index.to_string());
            child.label = format!("Item {}", index + 1);
        }
    }
}

impl Field {
    fn new(path: &str, key: &str, label: &str, kind: Kind, required: bool) -> Self {
        Self {
            path: path.to_owned(),
            key: key.to_owned(),
            label: label.to_owned(),
            description: String::new(),
            required,
            kind,
            raw: if kind == Kind::Boolean {
                "false".to_owned()
            } else {
                String::new()
            },
            error: None,
            options: Vec::new(),
            min_length: None,
            max_length: None,
            pattern: None,
            minimum: None,
            maximum: None,
            exclusive_minimum: None,
            exclusive_maximum: None,
            error_message: None,
            children: Vec::new(),
            item: None,
        }
    }

    pub fn id(&self) -> String {
        if self.path.is_empty() {
            "task-input".to_owned()
        } else {
            format!("task-input-{}", self.path.replace('.', "-"))
        }
    }

    pub fn help_id(&self) -> String {
        format!("{}-help", self.id())
    }

    pub fn error_id(&self) -> String {
        format!("{}-error", self.id())
    }
}

pub fn attrs(field: &Field) -> Vec<(&'static str, String)> {
    let mut attrs = Vec::new();
    if let Some(input) = field.kind.input_type() {
        attrs.push(("type", input.to_owned()));
    }
    match field.kind {
        Kind::Integer => attrs.push(("step", "1".to_owned())),
        Kind::Number => attrs.push(("step", "any".to_owned())),
        Kind::DateTime | Kind::Time => attrs.push(("step", "1".to_owned())),
        _ => {}
    }
    if field.required
        && matches!(
            field.kind,
            Kind::Text
                | Kind::Date
                | Kind::Time
                | Kind::DateTime
                | Kind::Integer
                | Kind::Number
                | Kind::Enum
                | Kind::Json
        )
    {
        attrs.push(("required", "required".to_owned()));
        attrs.push(("aria-required", "true".to_owned()));
    }
    push_attr(&mut attrs, "min", field.minimum.as_deref());
    push_attr(&mut attrs, "max", field.maximum.as_deref());
    if let Some(min) = field.min_length {
        attrs.push(("minlength", min.to_string()));
    }
    if let Some(max) = field.max_length {
        attrs.push(("maxlength", max.to_string()));
    }
    push_attr(&mut attrs, "pattern", field.pattern.as_deref());
    push_attr(
        &mut attrs,
        "data-exclusive-minimum",
        field.exclusive_minimum.as_deref(),
    );
    push_attr(
        &mut attrs,
        "data-exclusive-maximum",
        field.exclusive_maximum.as_deref(),
    );
    if field.error.is_some() {
        attrs.push(("aria-invalid", "true".to_owned()));
    }
    attrs
}

fn field_from(
    schema: &Value,
    path: &str,
    key: &str,
    fallback: &str,
    required: bool,
    secret: Option<&str>,
) -> Field {
    let label = text(
        schema
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(fallback),
        secret,
    );
    if unsupported(schema) {
        return json_field(path, key, &label, required, schema, secret);
    }
    let Some(kind) = kind_of(schema) else {
        return json_field(path, key, &label, required, schema, secret);
    };
    let mut field = match kind {
        Kind::Object => object_field(schema, path, key, &label, required, secret),
        Kind::Array => array_field(schema, path, key, &label, required, secret),
        _ => scalar_field(schema, path, key, &label, kind, required, secret),
    };
    field.description = text(
        schema
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(""),
        secret,
    );
    field.error_message = schema
        .get("errorMessage")
        .and_then(Value::as_str)
        .map(|message| text(message, secret));
    if let Some(default) = schema.get("default") {
        apply_value(&mut field, default, secret);
    }
    field
}

fn kind_of(schema: &Value) -> Option<Kind> {
    if schema.get("enum").and_then(Value::as_array).is_some() {
        return Some(Kind::Enum);
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => Some(Kind::Object),
        Some("array") if schema.get("items").and_then(Value::as_object).is_some() => {
            Some(Kind::Array)
        }
        Some("string") => Some(string_kind(schema)),
        Some("integer") => Some(Kind::Integer),
        Some("number") => Some(Kind::Number),
        Some("boolean") => Some(Kind::Boolean),
        None if schema.get("properties").is_some() || is_blank(schema) => Some(Kind::Object),
        _ => None,
    }
}

fn string_kind(schema: &Value) -> Kind {
    match schema.get("format").and_then(Value::as_str) {
        Some("date") => Kind::Date,
        Some("time") => Kind::Time,
        Some("date-time") | Some("datetime") => Kind::DateTime,
        _ => Kind::Text,
    }
}

fn object_field(
    schema: &Value,
    path: &str,
    key: &str,
    label: &str,
    required: bool,
    secret: Option<&str>,
) -> Field {
    let mut field = Field::new(path, key, label, Kind::Object, required);
    let required_keys = required_keys(schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return field;
    };
    for (name, child) in properties {
        field.children.push(field_from(
            child,
            &join(path, name),
            name,
            name,
            required_keys.iter().any(|item| item == name),
            secret,
        ));
    }
    field
}

fn array_field(
    schema: &Value,
    path: &str,
    key: &str,
    label: &str,
    required: bool,
    secret: Option<&str>,
) -> Field {
    let mut field = Field::new(path, key, label, Kind::Array, required);
    if let Some(items) = schema.get("items") {
        field.item = Some(Box::new(field_from(
            items,
            &join(path, "_"),
            "_",
            "Item",
            false,
            secret,
        )));
    }
    field
}

fn scalar_field(
    schema: &Value,
    path: &str,
    key: &str,
    label: &str,
    kind: Kind,
    required: bool,
    secret: Option<&str>,
) -> Field {
    let mut field = Field::new(path, key, label, kind, required);
    field.min_length = schema.get("minLength").and_then(Value::as_u64);
    field.max_length = schema.get("maxLength").and_then(Value::as_u64);
    field.pattern = schema
        .get("pattern")
        .and_then(Value::as_str)
        .map(|pattern| text(pattern, secret));
    field.minimum = number_string(schema.get("minimum"));
    field.maximum = number_string(schema.get("maximum"));
    exclusive_bound(
        schema.get("exclusiveMinimum"),
        &mut field.minimum,
        &mut field.exclusive_minimum,
    );
    exclusive_bound(
        schema.get("exclusiveMaximum"),
        &mut field.maximum,
        &mut field.exclusive_maximum,
    );
    if kind == Kind::Enum {
        field.options = schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| redact_value(item, secret))
                    .collect()
            })
            .unwrap_or_default();
    }
    field
}

fn json_field(
    path: &str,
    key: &str,
    label: &str,
    required: bool,
    schema: &Value,
    secret: Option<&str>,
) -> Field {
    let mut field = Field::new(path, key, label, Kind::Json, required);
    field.description = text(
        schema
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(""),
        secret,
    );
    field.error_message = schema
        .get("errorMessage")
        .and_then(Value::as_str)
        .map(|message| text(message, secret));
    if let Some(default) = schema.get("default") {
        field.raw = text(&pretty(default), secret);
    }
    field
}

fn read(field: &mut Field) -> Result<Option<Value>, ()> {
    field.error = None;
    match field.kind {
        Kind::Object => read_object(field),
        Kind::Array => read_array(field),
        Kind::Json => read_json(field),
        Kind::Boolean => Ok(Some(Value::Bool(field.raw == "true"))),
        Kind::Enum => read_enum(field),
        Kind::Integer => read_integer(field),
        Kind::Number => read_number(field),
        Kind::Text | Kind::Date | Kind::Time | Kind::DateTime => read_string(field),
    }
}

fn read_object(field: &mut Field) -> Result<Option<Value>, ()> {
    let mut map = Map::new();
    let mut ok = true;
    for child in &mut field.children {
        match read(child) {
            Ok(Some(value)) => {
                map.insert(child.key.clone(), value);
            }
            Ok(None) if child.required => {
                child.error = Some(fail(child, "This field is required."));
                ok = false;
            }
            Ok(None) => {}
            Err(()) => ok = false,
        }
    }
    if ok {
        Ok(Some(Value::Object(map)))
    } else {
        Err(())
    }
}

fn read_array(field: &mut Field) -> Result<Option<Value>, ()> {
    let mut items = Vec::new();
    let mut ok = true;
    for child in &mut field.children {
        match read(child) {
            Ok(Some(value)) => items.push(value),
            Ok(None) => {
                child.error = Some(fail(child, "This field is required."));
                ok = false;
            }
            Err(()) => ok = false,
        }
    }
    if ok {
        Ok(Some(Value::Array(items)))
    } else {
        Err(())
    }
}

fn read_json(field: &mut Field) -> Result<Option<Value>, ()> {
    let raw = field.raw.trim();
    if raw.is_empty() {
        if field.required {
            field.error = Some(fail(field, "Enter valid JSON."));
            return Err(());
        }
        return Ok(None);
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => Ok(Some(value)),
        Err(_) => {
            field.error = Some(fail(field, "Enter valid JSON."));
            Err(())
        }
    }
}

fn read_enum(field: &mut Field) -> Result<Option<Value>, ()> {
    let raw = field.raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if let Some(value) = field
        .options
        .iter()
        .find(|option| json_scalar(option) == raw)
    {
        return Ok(Some(value.clone()));
    }
    field.error = Some(fail(field, "Choose one of the available values."));
    Err(())
}

fn read_integer(field: &mut Field) -> Result<Option<Value>, ()> {
    let raw = field.raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let Ok(value) = raw.parse::<i64>() else {
        field.error = Some(fail(field, "Enter an integer."));
        return Err(());
    };
    if let Some(error) = range_ok(field, value as f64) {
        field.error = Some(error);
        return Err(());
    }
    Ok(Some(Value::from(value)))
}

fn read_number(field: &mut Field) -> Result<Option<Value>, ()> {
    let raw = field.raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let Ok(value) = raw.parse::<f64>() else {
        field.error = Some(fail(field, "Enter a number."));
        return Err(());
    };
    let Some(number) = serde_json::Number::from_f64(value) else {
        field.error = Some(fail(field, "Enter a number."));
        return Err(());
    };
    if let Some(error) = range_ok(field, value) {
        field.error = Some(error);
        return Err(());
    }
    Ok(Some(Value::Number(number)))
}

fn read_string(field: &mut Field) -> Result<Option<Value>, ()> {
    let raw = field.raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if let Some(error) = string_ok(field, raw) {
        field.error = Some(error);
        return Err(());
    }
    let value = if field.kind == Kind::DateTime {
        datetime_value(raw)
    } else {
        raw.to_owned()
    };
    Ok(Some(Value::String(value)))
}

fn string_ok(field: &Field, value: &str) -> Option<String> {
    let chars = value.chars().count() as u64;
    if let Some(min) = field.min_length
        && chars < min
    {
        return Some(fail(field, &format!("Must be at least {min} characters.")));
    }
    if let Some(max) = field.max_length
        && chars > max
    {
        return Some(fail(field, &format!("Must be at most {max} characters.")));
    }
    if let Some(pattern) = &field.pattern
        && !pattern_ok(value, pattern)
    {
        return Some(fail(field, "Must match the required pattern."));
    }
    None
}

fn range_ok(field: &Field, value: f64) -> Option<String> {
    if let Some(min) = bound(&field.minimum)
        && value < min
    {
        return Some(fail(
            field,
            &format!(
                "Must be at least {}.",
                field.minimum.as_deref().unwrap_or("")
            ),
        ));
    }
    if let Some(max) = bound(&field.maximum)
        && value > max
    {
        return Some(fail(
            field,
            &format!(
                "Must be at most {}.",
                field.maximum.as_deref().unwrap_or("")
            ),
        ));
    }
    if let Some(min) = bound(&field.exclusive_minimum)
        && value <= min
    {
        return Some(fail(
            field,
            &format!(
                "Must be greater than {}.",
                field.exclusive_minimum.as_deref().unwrap_or("")
            ),
        ));
    }
    if let Some(max) = bound(&field.exclusive_maximum)
        && value >= max
    {
        return Some(fail(
            field,
            &format!(
                "Must be less than {}.",
                field.exclusive_maximum.as_deref().unwrap_or("")
            ),
        ));
    }
    None
}

fn apply_value(field: &mut Field, value: &Value, secret: Option<&str>) {
    match field.kind {
        Kind::Object => {
            if let Some(object) = value.as_object() {
                for child in &mut field.children {
                    if let Some(item) = object.get(&child.key) {
                        apply_value(child, item, secret);
                    }
                }
            }
        }
        Kind::Array => fill_array(field, value, secret),
        Kind::Json => field.raw = text(&pretty(value), secret),
        Kind::Boolean => {
            field.raw = if value.as_bool() == Some(true) {
                "true".to_owned()
            } else {
                "false".to_owned()
            };
        }
        Kind::DateTime => field.raw = text(&strip_zulu(&json_scalar(value)), secret),
        _ => field.raw = text(&json_scalar(value), secret),
    }
}

fn fill_array(field: &mut Field, value: &Value, secret: Option<&str>) {
    let Some(items) = value.as_array() else {
        return;
    };
    let Some(template) = field.item.clone() else {
        return;
    };
    field.children.clear();
    for (index, item) in items.iter().enumerate() {
        let mut child = *template.clone();
        retarget(
            &mut child,
            &join(&field.path, &index.to_string()),
            &index.to_string(),
        );
        child.label = format!("Item {}", index + 1);
        apply_value(&mut child, item, secret);
        field.children.push(child);
    }
}

fn retarget(field: &mut Field, path: &str, key: &str) {
    field.path = path.to_owned();
    field.key = key.to_owned();
    match field.kind {
        Kind::Object => {
            for child in &mut field.children {
                let child_key = child.key.clone();
                retarget(child, &join(path, &child_key), &child_key);
            }
        }
        Kind::Array => {
            for (index, child) in field.children.iter_mut().enumerate() {
                retarget(child, &join(path, &index.to_string()), &index.to_string());
            }
            if let Some(item) = field.item.as_mut() {
                retarget(item, &join(path, "_"), "_");
            }
        }
        _ => {}
    }
}

fn find<'a>(field: &'a Field, path: &str) -> Option<&'a Field> {
    if field.path == path {
        return Some(field);
    }
    field.children.iter().find_map(|child| find(child, path))
}

fn find_mut<'a>(field: &'a mut Field, path: &str) -> Option<&'a mut Field> {
    if field.path == path {
        return Some(field);
    }
    field
        .children
        .iter_mut()
        .find_map(|child| find_mut(child, path))
}

fn unsupported(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return true;
    };
    if UNSUPPORTED.iter().any(|key| object.contains_key(*key)) {
        return true;
    }
    if matches!(object.get("items"), Some(Value::Array(_))) {
        return true;
    }
    match object.get("type") {
        Some(Value::Array(_)) => true,
        Some(Value::String(ty)) => !matches!(
            ty.as_str(),
            "object" | "array" | "string" | "integer" | "number" | "boolean"
        ),
        Some(_) => true,
        None => false,
    }
}

fn is_blank(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let ignorable = [
        "$schema",
        "$id",
        "title",
        "description",
        "type",
        "properties",
        "required",
        "additionalProperties",
    ];
    if object.keys().any(|key| !ignorable.contains(&key.as_str())) {
        return false;
    }
    let ty = object.get("type").and_then(Value::as_str);
    if ty.is_some() && ty != Some("object") {
        return false;
    }
    object
        .get("properties")
        .and_then(Value::as_object)
        .is_none_or(Map::is_empty)
}

fn required_keys(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn exclusive_bound(
    value: Option<&Value>,
    inclusive: &mut Option<String>,
    exclusive: &mut Option<String>,
) {
    match value {
        Some(Value::Bool(true)) => *exclusive = inclusive.take(),
        Some(Value::Number(number)) => *exclusive = Some(number.to_string()),
        _ => {}
    }
}

fn pattern_ok(value: &str, pattern: &str) -> bool {
    let (pattern, start) = match pattern.strip_prefix('^') {
        Some(rest) => (rest, true),
        None => (pattern, false),
    };
    let (pattern, end) = if pattern.ends_with('$') && !pattern.ends_with("\\$") {
        (&pattern[..pattern.len() - 1], true)
    } else {
        (pattern, false)
    };
    if pattern.chars().any(|ch| {
        matches!(
            ch,
            '.' | '*' | '+' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '\\'
        )
    }) {
        return true;
    }
    match (start, end) {
        (true, true) => value == pattern,
        (true, false) => value.starts_with(pattern),
        (false, true) => value.ends_with(pattern),
        (false, false) => value.contains(pattern),
    }
}

fn datetime_value(raw: &str) -> String {
    if raw.ends_with('Z') || raw.contains('+') || raw.rfind('-').is_some_and(|index| index > 10) {
        return raw.to_owned();
    }
    if raw.len() == 16 {
        format!("{raw}:00Z")
    } else {
        format!("{raw}Z")
    }
}

fn strip_zulu(value: &str) -> String {
    value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .unwrap_or(value)
        .to_owned()
}

fn fail(field: &Field, generated: &str) -> String {
    field
        .error_message
        .clone()
        .unwrap_or_else(|| generated.to_owned())
}

fn raw_value(kind: Kind, value: &str) -> String {
    if kind == Kind::Boolean {
        if matches!(value, "true" | "on" | "1") {
            "true".to_owned()
        } else {
            "false".to_owned()
        }
    } else {
        value.to_owned()
    }
}

fn split_index(path: &str) -> Option<(&str, &str)> {
    match path.rsplit_once('.') {
        Some((parent, index)) if index.bytes().all(|byte| byte.is_ascii_digit()) => {
            Some((parent, index))
        }
        None if path.bytes().all(|byte| byte.is_ascii_digit()) => Some(("", path)),
        _ => None,
    }
}

fn join(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}

fn push_attr(attrs: &mut Vec<(&'static str, String)>, name: &'static str, value: Option<&str>) {
    if let Some(value) = value {
        attrs.push((name, value.to_owned()));
    }
}

fn number_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn bound(value: &Option<String>) -> Option<f64> {
    value.as_deref()?.parse().ok()
}

fn json_scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn text(value: &str, secret: Option<&str>) -> String {
    codec::redact(value, secret)
}

fn redact_value(value: &Value, secret: Option<&str>) -> Value {
    match value {
        Value::String(text) => Value::String(codec::redact(text, secret)),
        other => other.clone(),
    }
}
