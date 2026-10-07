//! JSON Schema and JSON/protobuf conversion for SiLA values.

use std::collections::HashMap;

use base64::Engine;
use prost::Message;
use prost_reflect::bytes::Bytes;
use prost_reflect::{DynamicMessage, ReflectMessage, Value as Wire};
use serde_json::{Map, Value, json};

use crate::error::A2aLabError;
use crate::sila::consumer::model::{Basic, Constraints, Element, FeatureModel, SilaType};

pub(crate) const BINARY_LIMIT: usize = 2 * 1024 * 1024;
const TRANSFER: &str = "$silaTransfer";

/// Uploads already performed, keyed by payload bytes.
#[derive(Default)]
pub(crate) struct Uploads {
    pub ready: Vec<(Vec<u8>, String)>,
}

pub(crate) fn input_schema(feature: &FeatureModel, elements: &[Element]) -> String {
    schema_document(feature, elements).to_string()
}

pub(crate) fn output_schema(feature: &FeatureModel, elements: &[Element]) -> String {
    schema_document(feature, elements).to_string()
}

pub(crate) fn property_input_schema() -> String {
    json!({
        "type": "object",
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
    .to_string()
}

fn schema_document(feature: &FeatureModel, elements: &[Element]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for element in elements {
        required.push(Value::String(element.identifier.clone()));
        properties.insert(
            element.identifier.clone(),
            type_schema(feature, &element.data_type),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn type_schema(feature: &FeatureModel, data_type: &SilaType) -> Value {
    match data_type {
        SilaType::Basic(basic) => basic_schema(*basic),
        SilaType::Reference(name) => match feature.data_type(name) {
            Some(inner) => type_schema(feature, inner),
            None => json!({"type": "object"}),
        },
        SilaType::List(item) => json!({
            "type": "array",
            "items": type_schema(feature, item)
        }),
        SilaType::Structure(elements) => schema_document(feature, elements),
        SilaType::Constrained { inner, constraints } => {
            constrained_schema(feature, inner, constraints)
        }
    }
}

fn constrained_schema(
    feature: &FeatureModel,
    inner: &SilaType,
    constraints: &Constraints,
) -> Value {
    match inner {
        SilaType::List(item) => {
            let mut schema = match type_schema(feature, &SilaType::List(item.clone())) {
                Value::Object(object) => object,
                other => return other,
            };
            if let Some(count) = constraints.element_count {
                schema.insert("minItems".to_owned(), json!(count));
                schema.insert("maxItems".to_owned(), json!(count));
            }
            if let Some(min) = constraints.min_elements {
                schema.insert("minItems".to_owned(), json!(min));
            }
            if let Some(max) = constraints.max_elements {
                schema.insert("maxItems".to_owned(), json!(max));
            }
            Value::Object(schema)
        }
        SilaType::Constrained {
            inner,
            constraints: nested,
        } => {
            let mut schema = match constrained_schema(feature, inner, nested) {
                Value::Object(object) => object,
                other => return other,
            };
            apply_constraints(&mut schema, constraints);
            Value::Object(schema)
        }
        other => {
            let mut schema = match type_schema(feature, other) {
                Value::Object(object) => object,
                other => return other,
            };
            apply_constraints(&mut schema, constraints);
            Value::Object(schema)
        }
    }
}

fn apply_constraints(schema: &mut Map<String, Value>, constraints: &Constraints) {
    if let Some(length) = constraints.length {
        schema.insert("minLength".to_owned(), json!(length));
        schema.insert("maxLength".to_owned(), json!(length));
    }
    if let Some(min) = constraints.min_length {
        schema.insert("minLength".to_owned(), json!(min));
    }
    if let Some(max) = constraints.max_length {
        schema.insert("maxLength".to_owned(), json!(max));
    }
    if let Some(pattern) = &constraints.pattern {
        schema.insert("pattern".to_owned(), json!(pattern));
    }
    if constraints.fully_qualified_identifier {
        schema.insert(
            "x-sila-fully-qualified-identifier".to_owned(),
            Value::Bool(true),
        );
    }
    if let Some(value) = constraints.minimum {
        schema.insert("minimum".to_owned(), json!(value));
    }
    if let Some(value) = constraints.exclusive_minimum {
        schema.insert("exclusiveMinimum".to_owned(), json!(value));
    }
    if let Some(value) = constraints.maximum {
        schema.insert("maximum".to_owned(), json!(value));
    }
    if let Some(value) = constraints.exclusive_maximum {
        schema.insert("exclusiveMaximum".to_owned(), json!(value));
    }
    if let Some(unit) = &constraints.unit {
        schema.insert("x-sila-unit".to_owned(), json!(unit));
    }
    if !constraints.enumeration.is_empty() {
        schema.insert(
            "enum".to_owned(),
            Value::Array(
                constraints
                    .enumeration
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
}

fn basic_schema(basic: Basic) -> Value {
    match basic {
        Basic::String => json!({"type": "string"}),
        Basic::Integer => json!({"type": "integer"}),
        Basic::Real => json!({"type": "number"}),
        Basic::Boolean => json!({"type": "boolean"}),
        Basic::Binary => json!({"type": "string", "contentEncoding": "base64"}),
        Basic::Date => json!({"type": "string", "format": "date"}),
        Basic::Time => json!({"type": "string", "format": "time"}),
        Basic::Timestamp => json!({"type": "string", "format": "date-time"}),
        Basic::Any => json!({}),
    }
}

pub(crate) fn parameters_without_metadata(
    input: &crate::json_object::JsonObject,
) -> HashMap<String, Value> {
    input
        .as_map()
        .iter()
        .filter(|(key, _)| key.as_str() != "metadata")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Encodes command or metadata fields. Large binaries return [`EncodeStop::Upload`].
pub(crate) fn encode_fields(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    message_name: &str,
    elements: &[Element],
    values: &HashMap<String, Value>,
    uploads: &Uploads,
    parameter: &str,
) -> Result<DynamicMessage, EncodeStop> {
    let descriptor = pool
        .get_message_by_name(message_name)
        .ok_or_else(|| EncodeStop::Protocol(format!("missing message {message_name}")))?;
    let mut message = DynamicMessage::new(descriptor);
    fill_message(
        pool,
        feature,
        &mut message,
        elements,
        values,
        uploads,
        parameter,
    )?;
    Ok(message)
}

fn fill_message(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    message: &mut DynamicMessage,
    elements: &[Element],
    values: &HashMap<String, Value>,
    uploads: &Uploads,
    parameter: &str,
) -> Result<(), EncodeStop> {
    for element in elements {
        let value = values.get(&element.identifier).ok_or_else(|| {
            EncodeStop::Invalid(format!("missing parameter {}", element.identifier))
        })?;
        let parameter = if parameter.is_empty() {
            element.identifier.clone()
        } else {
            parameter.to_owned()
        };
        let field = message
            .descriptor()
            .get_field_by_name(&element.identifier)
            .ok_or_else(|| {
                EncodeStop::Protocol(format!(
                    "{} has no {}",
                    message.descriptor().full_name(),
                    element.identifier
                ))
            })?
            .clone();
        let encoded = encode_field(
            pool,
            feature,
            &field,
            &element.data_type,
            value,
            uploads,
            &parameter,
        )?;
        message.set_field(&field, encoded);
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum EncodeStop {
    Invalid(String),
    Protocol(String),
    Upload { parameter: String, bytes: Vec<u8> },
}

fn encode_field(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    field: &prost_reflect::FieldDescriptor,
    data_type: &SilaType,
    value: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<Wire, EncodeStop> {
    match data_type {
        SilaType::Constrained { inner, .. } => {
            encode_field(pool, feature, field, inner, value, uploads, parameter)
        }
        SilaType::Reference(name) => {
            let inner = feature
                .data_type(name)
                .ok_or_else(|| EncodeStop::Protocol(format!("unknown SiLA type {name}")))?;
            let wrapper = field_message(field)?;
            let mut message = DynamicMessage::new(wrapper);
            let inner_field = message
                .descriptor()
                .get_field_by_name(name)
                .ok_or_else(|| EncodeStop::Protocol(format!("custom type {name} has no field")))?
                .clone();
            let encoded = encode_field(
                pool,
                feature,
                &inner_field,
                inner,
                value,
                uploads,
                parameter,
            )?;
            message.set_field(&inner_field, encoded);
            Ok(Wire::Message(message))
        }
        SilaType::Structure(elements) => {
            let Value::Object(object) = value else {
                return Err(EncodeStop::Invalid(
                    "structure must be an object".to_owned(),
                ));
            };
            let mut values = HashMap::new();
            for (key, item) in object {
                values.insert(key.clone(), item.clone());
            }
            let mut message = DynamicMessage::new(field_message(field)?);
            fill_message(
                pool,
                feature,
                &mut message,
                elements,
                &values,
                uploads,
                parameter,
            )?;
            Ok(Wire::Message(message))
        }
        SilaType::List(item) => {
            let Value::Array(items) = value else {
                return Err(EncodeStop::Invalid("list must be an array".to_owned()));
            };
            let mut encoded = Vec::new();
            for item_value in items {
                encoded.push(encode_field(
                    pool, feature, field, item, item_value, uploads, parameter,
                )?);
            }
            Ok(Wire::List(encoded))
        }
        SilaType::Basic(basic) => encode_basic(pool, feature, *basic, value, uploads, parameter),
    }
}

fn field_message(
    field: &prost_reflect::FieldDescriptor,
) -> Result<prost_reflect::MessageDescriptor, EncodeStop> {
    match field.kind() {
        prost_reflect::Kind::Message(descriptor) => Ok(descriptor),
        _ => Err(EncodeStop::Protocol(format!(
            "{} is not a message field",
            field.name()
        ))),
    }
}

fn encode_basic(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    basic: Basic,
    value: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<Wire, EncodeStop> {
    let name = match basic {
        Basic::String => "sila2.org.silastandard.String",
        Basic::Integer => "sila2.org.silastandard.Integer",
        Basic::Real => "sila2.org.silastandard.Real",
        Basic::Boolean => "sila2.org.silastandard.Boolean",
        Basic::Binary => "sila2.org.silastandard.Binary",
        Basic::Date => "sila2.org.silastandard.Date",
        Basic::Time => "sila2.org.silastandard.Time",
        Basic::Timestamp => "sila2.org.silastandard.Timestamp",
        Basic::Any => "sila2.org.silastandard.Any",
    };
    let descriptor = pool
        .get_message_by_name(name)
        .ok_or_else(|| EncodeStop::Protocol(format!("missing framework message {name}")))?;
    let mut message = DynamicMessage::new(descriptor);
    match basic {
        Basic::String => set_field(&mut message, "value", Wire::String(require_string(value)?))?,
        Basic::Integer => set_field(&mut message, "value", Wire::I64(require_i64(value)?))?,
        Basic::Real => set_field(&mut message, "value", Wire::F64(require_f64(value)?))?,
        Basic::Boolean => {
            let Value::Bool(flag) = value else {
                return Err(EncodeStop::Invalid("expected a boolean".to_owned()));
            };
            set_field(&mut message, "value", Wire::Bool(*flag))?;
        }
        Basic::Binary => encode_binary(&mut message, value, uploads, parameter)?,
        Basic::Date => encode_date(pool, &mut message, &require_string(value)?)?,
        Basic::Time => encode_time(pool, &mut message, &require_string(value)?)?,
        Basic::Timestamp => encode_timestamp(pool, &mut message, &require_string(value)?)?,
        Basic::Any => encode_any(pool, feature, &mut message, value, uploads, parameter)?,
    }
    Ok(Wire::Message(message))
}

fn encode_binary(
    message: &mut DynamicMessage,
    value: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<(), EncodeStop> {
    let bytes = decode_base64(&require_string(value)?)?;
    if bytes.len() > BINARY_LIMIT {
        if let Some((_, uuid)) = uploads.ready.iter().find(|(stored, _)| stored == &bytes) {
            set_field(message, "binaryTransferUUID", Wire::String(uuid.clone()))?;
            return Ok(());
        }
        return Err(EncodeStop::Upload {
            parameter: parameter.to_owned(),
            bytes,
        });
    }
    set_field(message, "value", Wire::Bytes(Bytes::from(bytes)))?;
    Ok(())
}

fn encode_date(
    pool: &prost_reflect::DescriptorPool,
    message: &mut DynamicMessage,
    text: &str,
) -> Result<(), EncodeStop> {
    let (body, hours, minutes) = split_zone(text)?;
    let mut parts = body.split('-');
    set_field(message, "year", Wire::U32(number(parts.next(), "year")?))?;
    set_field(message, "month", Wire::U32(number(parts.next(), "month")?))?;
    set_field(message, "day", Wire::U32(number(parts.next(), "day")?))?;
    set_field(message, "timezone", timezone(pool, hours, minutes)?)?;
    Ok(())
}

fn encode_time(
    pool: &prost_reflect::DescriptorPool,
    message: &mut DynamicMessage,
    text: &str,
) -> Result<(), EncodeStop> {
    let (body, hours, minutes) = split_zone(text)?;
    let (hour, minute, second, millisecond) = clock(body)?;
    set_field(message, "hour", Wire::U32(hour))?;
    set_field(message, "minute", Wire::U32(minute))?;
    set_field(message, "second", Wire::U32(second))?;
    set_field(message, "millisecond", Wire::U32(millisecond))?;
    set_field(message, "timezone", timezone(pool, hours, minutes)?)?;
    Ok(())
}

fn encode_timestamp(
    pool: &prost_reflect::DescriptorPool,
    message: &mut DynamicMessage,
    text: &str,
) -> Result<(), EncodeStop> {
    let (body, hours, minutes) = split_zone(text)?;
    let (date, time) = body.split_once(' ').ok_or_else(|| {
        EncodeStop::Invalid("timestamp must be YYYY-MM-DD HH:MM:SS.mmm±HH:MM".to_owned())
    })?;
    let mut parts = date.split('-');
    set_field(message, "year", Wire::U32(number(parts.next(), "year")?))?;
    set_field(message, "month", Wire::U32(number(parts.next(), "month")?))?;
    set_field(message, "day", Wire::U32(number(parts.next(), "day")?))?;
    let (hour, minute, second, millisecond) = clock(time)?;
    set_field(message, "hour", Wire::U32(hour))?;
    set_field(message, "minute", Wire::U32(minute))?;
    set_field(message, "second", Wire::U32(second))?;
    set_field(message, "millisecond", Wire::U32(millisecond))?;
    set_field(message, "timezone", timezone(pool, hours, minutes)?)?;
    Ok(())
}

fn timezone(
    pool: &prost_reflect::DescriptorPool,
    hours: i32,
    minutes: u32,
) -> Result<Wire, EncodeStop> {
    let descriptor = pool
        .get_message_by_name("sila2.org.silastandard.Timezone")
        .ok_or_else(|| EncodeStop::Protocol("missing Timezone".to_owned()))?;
    let mut message = DynamicMessage::new(descriptor);
    set_field(&mut message, "hours", Wire::I32(hours))?;
    set_field(&mut message, "minutes", Wire::U32(minutes))?;
    Ok(Wire::Message(message))
}

fn encode_any(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    message: &mut DynamicMessage,
    value: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<(), EncodeStop> {
    let (xml, payload_type, payload) = any_parts(value)?;
    let bytes = any_payload(pool, feature, &payload_type, &payload, uploads, parameter)?;
    set_field(message, "type", Wire::String(xml))?;
    set_field(message, "payload", Wire::Bytes(Bytes::from(bytes)))?;
    Ok(())
}

fn any_parts(value: &Value) -> Result<(String, SilaType, Value), EncodeStop> {
    if let Value::Object(object) = value
        && object.get("type").and_then(Value::as_str) == Some("Void")
    {
        let xml = "<DataType xmlns=\"http://www.sila-standard.org\"><Constrained><DataType><Basic>String</Basic></DataType><Constraints><Length>0</Length></Constraints></Constrained></DataType>";
        return Ok((
            xml.to_owned(),
            SilaType::Basic(Basic::String),
            Value::String(String::new()),
        ));
    }
    let Value::Object(object) = value else {
        return Ok((
            basic_xml("String"),
            SilaType::Basic(Basic::String),
            Value::String(require_string(value)?),
        ));
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| EncodeStop::Invalid("Any type must be a string".to_owned()))?;
    let payload = object
        .get("value")
        .cloned()
        .ok_or_else(|| EncodeStop::Invalid("Any value is missing".to_owned()))?;
    let data_type = match kind {
        "String" => SilaType::Basic(Basic::String),
        "Integer" => SilaType::Basic(Basic::Integer),
        "Real" => SilaType::Basic(Basic::Real),
        "Boolean" => SilaType::Basic(Basic::Boolean),
        "Binary" => SilaType::Basic(Basic::Binary),
        "Date" => SilaType::Basic(Basic::Date),
        "Time" => SilaType::Basic(Basic::Time),
        "Timestamp" => SilaType::Basic(Basic::Timestamp),
        "List" => SilaType::List(Box::new(SilaType::Basic(Basic::String))),
        "Structure" => return any_structure(&payload),
        _ => return Err(EncodeStop::Invalid("unsupported Any type".to_owned())),
    };
    Ok((type_xml(&data_type)?, data_type, payload))
}

fn any_structure(payload: &Value) -> Result<(String, SilaType, Value), EncodeStop> {
    let Value::Object(fields) = payload else {
        return Err(EncodeStop::Invalid(
            "Any structure must be an object".to_owned(),
        ));
    };
    let mut elements = Vec::new();
    for (name, value) in fields {
        let (inner_xml_ignored, data_type, _) = any_parts(value)?;
        let _ = inner_xml_ignored;
        elements.push(Element {
            identifier: name.clone(),
            data_type,
        });
    }
    let data_type = SilaType::Structure(elements);
    Ok((type_xml(&data_type)?, data_type, payload.clone()))
}

fn any_payload(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    data_type: &SilaType,
    payload: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<Vec<u8>, EncodeStop> {
    let encoded = match data_type {
        SilaType::Basic(basic) => encode_basic(pool, feature, *basic, payload, uploads, parameter)?,
        SilaType::List(item) => {
            let Value::Array(items) = payload else {
                return Err(EncodeStop::Invalid("Any list must be an array".to_owned()));
            };
            let mut encoded = Vec::new();
            for item_value in items {
                let SilaType::Basic(basic) = item.as_ref() else {
                    return Err(EncodeStop::Invalid(
                        "only lists of basic Any values are encoded".to_owned(),
                    ));
                };
                encoded.push(encode_basic(
                    pool, feature, *basic, item_value, uploads, parameter,
                )?);
            }
            Wire::List(encoded)
        }
        SilaType::Structure(elements) => {
            return any_structure_payload(pool, feature, elements, payload, uploads, parameter);
        }
        other => {
            return Err(EncodeStop::Invalid(format!(
                "unsupported Any payload {other:?}"
            )));
        }
    };
    wrap_any_payload(pool, data_type, encoded)
}

fn wrap_any_payload(
    pool: &prost_reflect::DescriptorPool,
    data_type: &SilaType,
    encoded: Wire,
) -> Result<Vec<u8>, EncodeStop> {
    let mut file_pool = pool.clone();
    let file = prost_types::FileDescriptorProto {
        name: Some("sila/any/payload.proto".to_owned()),
        package: Some("sila.any.payload".to_owned()),
        syntax: Some("proto3".to_owned()),
        dependency: vec!["SiLAFramework.proto".to_owned()],
        message_type: vec![prost_types::DescriptorProto {
            name: Some("AnyPayload".to_owned()),
            field: vec![prost_types::FieldDescriptorProto {
                name: Some("Value".to_owned()),
                number: Some(1),
                label: Some(if matches!(data_type, SilaType::List(_)) {
                    prost_types::field_descriptor_proto::Label::Repeated as i32
                } else {
                    prost_types::field_descriptor_proto::Label::Optional as i32
                }),
                r#type: Some(prost_types::field_descriptor_proto::Type::Message as i32),
                type_name: Some(any_field_type(data_type)),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    file_pool
        .add_file_descriptor_proto(file)
        .map_err(|error| EncodeStop::Protocol(error.to_string()))?;
    let descriptor = file_pool
        .get_message_by_name("sila.any.payload.AnyPayload")
        .ok_or_else(|| EncodeStop::Protocol("missing Any payload".to_owned()))?;
    let mut message = DynamicMessage::new(descriptor);
    set_field(&mut message, "Value", encoded)?;
    Ok(message.encode_to_vec())
}

fn any_structure_payload(
    pool: &prost_reflect::DescriptorPool,
    feature: &FeatureModel,
    elements: &[Element],
    payload: &Value,
    uploads: &Uploads,
    parameter: &str,
) -> Result<Vec<u8>, EncodeStop> {
    let Value::Object(object) = payload else {
        return Err(EncodeStop::Invalid(
            "Any structure must be an object".to_owned(),
        ));
    };
    let mut values = HashMap::new();
    for (key, value) in object {
        let normalized = match value {
            Value::Object(typed) if typed.contains_key("type") => {
                typed.get("value").cloned().unwrap_or(Value::Null)
            }
            other => other.clone(),
        };
        values.insert(key.clone(), normalized);
    }
    let mut fields = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        let number = i32::try_from(index + 1).unwrap_or(i32::MAX);
        fields.push(prost_types::FieldDescriptorProto {
            name: Some(element.identifier.clone()),
            number: Some(number),
            label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
            r#type: Some(prost_types::field_descriptor_proto::Type::Message as i32),
            type_name: Some(any_field_type(&element.data_type)),
            ..Default::default()
        });
    }
    let mut file_pool = pool.clone();
    file_pool
        .add_file_descriptor_proto(prost_types::FileDescriptorProto {
            name: Some("sila/any/structure.proto".to_owned()),
            package: Some("sila.any.structure".to_owned()),
            syntax: Some("proto3".to_owned()),
            dependency: vec!["SiLAFramework.proto".to_owned()],
            message_type: vec![
                prost_types::DescriptorProto {
                    name: Some("StructValue".to_owned()),
                    field: fields,
                    ..Default::default()
                },
                prost_types::DescriptorProto {
                    name: Some("AnyPayload".to_owned()),
                    field: vec![prost_types::FieldDescriptorProto {
                        name: Some("Value".to_owned()),
                        number: Some(1),
                        label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
                        r#type: Some(prost_types::field_descriptor_proto::Type::Message as i32),
                        type_name: Some(".sila.any.structure.StructValue".to_owned()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .map_err(|error| EncodeStop::Protocol(error.to_string()))?;
    let struct_desc = file_pool
        .get_message_by_name("sila.any.structure.StructValue")
        .ok_or_else(|| EncodeStop::Protocol("missing Any structure".to_owned()))?;
    let mut structure = DynamicMessage::new(struct_desc);
    fill_message(
        pool,
        feature,
        &mut structure,
        elements,
        &values,
        uploads,
        parameter,
    )?;
    let payload_desc = file_pool
        .get_message_by_name("sila.any.structure.AnyPayload")
        .ok_or_else(|| EncodeStop::Protocol("missing Any payload".to_owned()))?;
    let mut message = DynamicMessage::new(payload_desc);
    set_field(&mut message, "Value", Wire::Message(structure))?;
    Ok(message.encode_to_vec())
}

fn any_field_type(data_type: &SilaType) -> String {
    match data_type.effective() {
        SilaType::Basic(Basic::String | Basic::Any)
        | SilaType::Structure(_)
        | SilaType::Reference(_) => ".sila2.org.silastandard.String".to_owned(),
        SilaType::Basic(Basic::Integer) => ".sila2.org.silastandard.Integer".to_owned(),
        SilaType::Basic(Basic::Real) => ".sila2.org.silastandard.Real".to_owned(),
        SilaType::Basic(Basic::Boolean) => ".sila2.org.silastandard.Boolean".to_owned(),
        SilaType::Basic(Basic::Binary) => ".sila2.org.silastandard.Binary".to_owned(),
        SilaType::Basic(Basic::Date) => ".sila2.org.silastandard.Date".to_owned(),
        SilaType::Basic(Basic::Time) => ".sila2.org.silastandard.Time".to_owned(),
        SilaType::Basic(Basic::Timestamp) => ".sila2.org.silastandard.Timestamp".to_owned(),
        SilaType::List(item) => any_field_type(item),
        SilaType::Constrained { inner, .. } => any_field_type(inner),
    }
}

fn type_xml(data_type: &SilaType) -> Result<String, EncodeStop> {
    Ok(format!(
        "<DataType xmlns=\"http://www.sila-standard.org\">{}</DataType>",
        type_xml_inner(data_type)?
    ))
}

fn type_xml_inner(data_type: &SilaType) -> Result<String, EncodeStop> {
    match data_type {
        SilaType::Basic(basic) => Ok(format!("<Basic>{}</Basic>", basic_name(*basic))),
        SilaType::List(item) => Ok(format!(
            "<List><DataType>{}</DataType></List>",
            type_xml_inner(item)?
        )),
        SilaType::Structure(elements) => {
            let mut body = String::from("<Structure>");
            for element in elements {
                use std::fmt::Write as _;
                let _ = write!(
                    body,
                    "<Element><Identifier>{}</Identifier><DisplayName>{}</DisplayName><Description>{}</Description><DataType>{}</DataType></Element>",
                    element.identifier,
                    element.identifier,
                    element.identifier,
                    type_xml_inner(&element.data_type)?
                );
            }
            body.push_str("</Structure>");
            Ok(body)
        }
        SilaType::Constrained { inner, constraints } => Ok(format!(
            "<Constrained><DataType>{}</DataType><Constraints>{}</Constraints></Constrained>",
            type_xml_inner(inner)?,
            constraint_xml(constraints)
        )),
        SilaType::Reference(_) => Err(EncodeStop::Invalid(
            "Any cannot contain a custom data type".to_owned(),
        )),
    }
}

fn constraint_xml(constraints: &Constraints) -> String {
    if constraints.length == Some(0) {
        "<Length>0</Length>".to_owned()
    } else {
        String::new()
    }
}

fn basic_xml(name: &str) -> String {
    format!("<DataType xmlns=\"http://www.sila-standard.org\"><Basic>{name}</Basic></DataType>")
}

fn basic_name(basic: Basic) -> &'static str {
    match basic {
        Basic::String => "String",
        Basic::Integer => "Integer",
        Basic::Real => "Real",
        Basic::Boolean => "Boolean",
        Basic::Binary => "Binary",
        Basic::Date => "Date",
        Basic::Time => "Time",
        Basic::Timestamp => "Timestamp",
        Basic::Any => "Any",
    }
}

pub(crate) fn decode_fields(
    feature: &FeatureModel,
    elements: &[Element],
    message: &DynamicMessage,
) -> Result<Map<String, Value>, A2aLabError> {
    let mut object = Map::new();
    for element in elements {
        let field = message
            .descriptor()
            .get_field_by_name(&element.identifier)
            .ok_or_else(|| {
                A2aLabError::protocol(format!("missing field {}", element.identifier))
            })?;
        if !message.has_field(&field) && !field.is_list() {
            continue;
        }
        object.insert(
            element.identifier.clone(),
            decode_type(feature, &element.data_type, &message.get_field(&field))?,
        );
    }
    Ok(object)
}

fn decode_type(
    feature: &FeatureModel,
    data_type: &SilaType,
    value: &Wire,
) -> Result<Value, A2aLabError> {
    match data_type {
        SilaType::Constrained { inner, .. } => decode_type(feature, inner, value),
        SilaType::Reference(name) => {
            let inner = feature
                .data_type(name)
                .ok_or_else(|| A2aLabError::protocol(format!("unknown SiLA type {name}")))?;
            let message = value
                .as_message()
                .ok_or_else(|| A2aLabError::protocol("custom type was not a message"))?;
            let field = message
                .descriptor()
                .get_field_by_name(name)
                .ok_or_else(|| A2aLabError::protocol(format!("custom type {name} has no field")))?;
            decode_type(feature, inner, &message.get_field(&field))
        }
        SilaType::Structure(elements) => {
            let message = value
                .as_message()
                .ok_or_else(|| A2aLabError::protocol("structure was not a message"))?;
            Ok(Value::Object(decode_fields(feature, elements, message)?))
        }
        SilaType::List(item) => {
            let items = value
                .as_list()
                .ok_or_else(|| A2aLabError::protocol("list was not repeated"))?;
            Ok(Value::Array(
                items
                    .iter()
                    .map(|item_value| decode_type(feature, item, item_value))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        SilaType::Basic(basic) => decode_basic(feature, *basic, value),
    }
}

fn decode_basic(_feature: &FeatureModel, basic: Basic, value: &Wire) -> Result<Value, A2aLabError> {
    let message = value
        .as_message()
        .ok_or_else(|| A2aLabError::protocol("basic value was not a message"))?;
    match basic {
        Basic::String => Ok(Value::String(field_string(message, "value")?)),
        Basic::Integer => Ok(json!(field_i64(message, "value")?)),
        Basic::Real => Ok(json!(field_f64(message, "value")?)),
        Basic::Boolean => Ok(Value::Bool(field_bool(message, "value")?)),
        Basic::Binary => decode_binary(message),
        Basic::Date => Ok(Value::String(format_date(message)?)),
        Basic::Time => Ok(Value::String(format_time(message)?)),
        Basic::Timestamp => Ok(Value::String(format_timestamp(message)?)),
        Basic::Any => Ok(decode_any(message)),
    }
}

fn decode_binary(message: &DynamicMessage) -> Result<Value, A2aLabError> {
    if let Some(field) = message.descriptor().get_field_by_name("binaryTransferUUID")
        && message.has_field(&field)
    {
        return Ok(json!({ TRANSFER: field_string(message, "binaryTransferUUID")? }));
    }
    let bytes = field_bytes(message, "value")?;
    Ok(Value::String(encode_base64(&bytes)))
}

fn decode_any(message: &DynamicMessage) -> Value {
    Value::String(field_string(message, "type").unwrap_or_default())
}

fn format_date(message: &DynamicMessage) -> Result<String, A2aLabError> {
    Ok(format!(
        "{:04}-{:02}-{:02}{}",
        field_u32(message, "year")?,
        field_u32(message, "month")?,
        field_u32(message, "day")?,
        format_zone(message)
    ))
}

fn format_time(message: &DynamicMessage) -> Result<String, A2aLabError> {
    Ok(format!(
        "{:02}:{:02}:{:02}.{:03}{}",
        field_u32(message, "hour")?,
        field_u32(message, "minute")?,
        field_u32(message, "second")?,
        field_u32(message, "millisecond")?,
        format_zone(message)
    ))
}

fn format_timestamp(message: &DynamicMessage) -> Result<String, A2aLabError> {
    Ok(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}{}",
        field_u32(message, "year")?,
        field_u32(message, "month")?,
        field_u32(message, "day")?,
        field_u32(message, "hour")?,
        field_u32(message, "minute")?,
        field_u32(message, "second")?,
        field_u32(message, "millisecond")?,
        format_zone(message)
    ))
}

fn format_zone(message: &DynamicMessage) -> String {
    let zone = message
        .descriptor()
        .get_field_by_name("timezone")
        .and_then(|field| message.get_field(&field).as_message().cloned());
    let Some(zone) = zone else {
        return "+00:00".to_owned();
    };
    let hours = field_i32(&zone, "hours").unwrap_or(0);
    let minutes = field_u32(&zone, "minutes").unwrap_or(0);
    format!("{hours:+03}:{minutes:02}")
}

pub(crate) fn collect_transfers(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk_transfers(value, &mut found);
    found
}

fn walk_transfers(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(uuid)) = object.get(TRANSFER) {
                found.push(uuid.clone());
            }
            for child in object.values() {
                walk_transfers(child, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_transfers(item, found);
            }
        }
        _ => {}
    }
}

pub(crate) fn replace_transfer(value: &mut Value, uuid: &str, encoded: &str) {
    match value {
        Value::Object(object) => {
            if object.get(TRANSFER).and_then(Value::as_str) == Some(uuid) {
                *value = Value::String(encoded.to_owned());
                return;
            }
            for child in object.values_mut() {
                replace_transfer(child, uuid, encoded);
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_transfer(item, uuid, encoded);
            }
        }
        _ => {}
    }
}

pub(crate) fn execution_uuid(
    pool: &prost_reflect::DescriptorPool,
    uuid: &str,
) -> Result<DynamicMessage, A2aLabError> {
    let descriptor = pool
        .get_message_by_name("sila2.org.silastandard.CommandExecutionUUID")
        .ok_or_else(|| A2aLabError::protocol("missing CommandExecutionUUID"))?;
    let mut message = DynamicMessage::new(descriptor);
    set_field(&mut message, "value", Wire::String(uuid.to_owned()))
        .map_err(EncodeStop::into_error)?;
    Ok(message)
}

pub(crate) fn read_execution_uuid(message: &DynamicMessage) -> Result<String, A2aLabError> {
    let nested = message
        .descriptor()
        .get_field_by_name("commandExecutionUUID")
        .and_then(|field| message.get_field(&field).as_message().cloned())
        .ok_or_else(|| A2aLabError::protocol("missing command execution UUID"))?;
    field_string(&nested, "value")
}

pub(crate) struct ExecutionSnapshot {
    pub status: i32,
    pub progress: Option<f64>,
}

pub(crate) fn read_execution_info(message: &DynamicMessage) -> ExecutionSnapshot {
    let status = match message.descriptor().get_field_by_name("commandStatus") {
        Some(field) => message.get_field(&field).as_enum_number().unwrap_or(0),
        None => 0,
    };
    let progress = message
        .descriptor()
        .get_field_by_name("progressInfo")
        .and_then(|field| message.get_field(&field).as_message().cloned())
        .and_then(|real| field_f64(&real, "value").ok());
    ExecutionSnapshot { status, progress }
}

pub(crate) fn sila_error_from_status(
    pool: &prost_reflect::DescriptorPool,
    status: &tonic::Status,
) -> Option<crate::sila::consumer::rpc::SilaFailure> {
    if status.code() != tonic::Code::Aborted {
        return None;
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(status.message())
        .ok()?;
    let descriptor = pool.get_message_by_name("sila2.org.silastandard.SiLAError")?;
    let message = DynamicMessage::decode(descriptor, raw.as_slice()).ok()?;
    failure_from_message(&message)
}

fn failure_from_message(
    message: &DynamicMessage,
) -> Option<crate::sila::consumer::rpc::SilaFailure> {
    if let Some(error) = nested(message, "validationError") {
        return Some(crate::sila::consumer::rpc::SilaFailure {
            kind: "validation".to_owned(),
            identifier: field_string(&error, "parameter").ok(),
            message: field_string(&error, "message").unwrap_or_default(),
        });
    }
    if let Some(error) = nested(message, "definedExecutionError") {
        return Some(crate::sila::consumer::rpc::SilaFailure {
            kind: "defined_execution".to_owned(),
            identifier: field_string(&error, "errorIdentifier").ok(),
            message: field_string(&error, "message").unwrap_or_default(),
        });
    }
    if let Some(error) = nested(message, "undefinedExecutionError") {
        return Some(crate::sila::consumer::rpc::SilaFailure {
            kind: "undefined_execution".to_owned(),
            identifier: None,
            message: field_string(&error, "message").unwrap_or_default(),
        });
    }
    if let Some(error) = nested(message, "frameworkError") {
        let identifier = error
            .descriptor()
            .get_field_by_name("errorType")
            .and_then(|field| error.get_field(&field).as_enum_number())
            .map(|number| number.to_string());
        return Some(crate::sila::consumer::rpc::SilaFailure {
            kind: "framework".to_owned(),
            identifier,
            message: field_string(&error, "message").unwrap_or_default(),
        });
    }
    None
}

fn nested(message: &DynamicMessage, name: &str) -> Option<DynamicMessage> {
    let field = message.descriptor().get_field_by_name(name)?;
    if !message.has_field(&field) {
        return None;
    }
    message.get_field(&field).as_message().cloned()
}

fn set_field(message: &mut DynamicMessage, name: &str, value: Wire) -> Result<(), EncodeStop> {
    let descriptor = message.descriptor();
    let field = descriptor.get_field_by_name(name).ok_or_else(|| {
        EncodeStop::Protocol(format!(
            "message {} has no field {name}",
            descriptor.full_name()
        ))
    })?;
    message.set_field(&field, value);
    Ok(())
}

fn require_string(value: &Value) -> Result<String, EncodeStop> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| EncodeStop::Invalid("expected a string".to_owned()))
}

fn require_i64(value: &Value) -> Result<i64, EncodeStop> {
    value
        .as_i64()
        .ok_or_else(|| EncodeStop::Invalid("expected an integer".to_owned()))
}

#[allow(clippy::cast_precision_loss)]
fn require_f64(value: &Value) -> Result<f64, EncodeStop> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|number| number as f64)) // integer JSON is exact for SiLA command inputs
        .ok_or_else(|| EncodeStop::Invalid("expected a number".to_owned()))
}

fn number(value: Option<&str>, field: &str) -> Result<u32, EncodeStop> {
    value
        .ok_or_else(|| EncodeStop::Invalid(format!("{field} is missing")))?
        .parse()
        .map_err(|_| EncodeStop::Invalid(format!("{field} is not a number")))
}

fn split_zone(text: &str) -> Result<(&str, i32, u32), EncodeStop> {
    let (body, zone) = if let Some(body) = text.strip_suffix('Z') {
        return Ok((body, 0, 0));
    } else {
        let split = text.rfind(['+', '-']).ok_or_else(|| {
            EncodeStop::Invalid("date or time is missing a timezone offset".to_owned())
        })?;
        text.split_at(split)
    };
    let (sign, rest) = zone.split_at(1);
    let (hours, minutes) = rest
        .split_once(':')
        .ok_or_else(|| EncodeStop::Invalid("timezone offset must be ±HH:MM".to_owned()))?;
    let mut hours: i32 = hours
        .parse()
        .map_err(|_| EncodeStop::Invalid("timezone hour is not a number".to_owned()))?;
    if sign == "-" {
        hours = -hours;
    }
    let minutes = minutes
        .parse()
        .map_err(|_| EncodeStop::Invalid("timezone minute is not a number".to_owned()))?;
    Ok((body, hours, minutes))
}

fn clock(clock: &str) -> Result<(u32, u32, u32, u32), EncodeStop> {
    let (time, fraction) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut parts = time.split(':');
    Ok((
        number(parts.next(), "hour")?,
        number(parts.next(), "minute")?,
        number(parts.next(), "second")?,
        fraction
            .parse()
            .map_err(|_| EncodeStop::Invalid("millisecond is not a number".to_owned()))?,
    ))
}

fn decode_base64(text: &str) -> Result<Vec<u8>, EncodeStop> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| EncodeStop::Invalid("binary is not base64".to_owned()))
}

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn field_string(message: &DynamicMessage, name: &str) -> Result<String, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not a string")))
}

fn field_i64(message: &DynamicMessage, name: &str) -> Result<i64, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_i64()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not an integer")))
}

fn field_f64(message: &DynamicMessage, name: &str) -> Result<f64, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_f64()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not a number")))
}

fn field_bool(message: &DynamicMessage, name: &str) -> Result<bool, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_bool()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not a boolean")))
}

fn field_u32(message: &DynamicMessage, name: &str) -> Result<u32, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_u32()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not an unsigned integer")))
}

fn field_i32(message: &DynamicMessage, name: &str) -> Result<i32, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_i32()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not an integer")))
}

fn field_bytes(message: &DynamicMessage, name: &str) -> Result<Bytes, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_bytes()
        .cloned()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not bytes")))
}

impl EncodeStop {
    pub(crate) fn into_error(self) -> A2aLabError {
        match self {
            Self::Invalid(message) => A2aLabError::invalid("input", message),
            Self::Protocol(message) => A2aLabError::protocol(message),
            Self::Upload { parameter, .. } => {
                A2aLabError::protocol(format!("binary for {parameter} was not uploaded"))
            }
        }
    }
}
