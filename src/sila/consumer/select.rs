//! JSON pointer selection and SiLA type checks for configured bindings.

use serde_json::{Map, Value};

use crate::error::A2aLabError;
use crate::json_object::JsonObject;
use crate::sila::consumer::model::{Basic, Element, FeatureModel, SilaType};
use crate::time::UtcTimestamp;

pub(crate) fn command_root(elements: &[Element]) -> SilaType {
    SilaType::Structure(elements.to_vec())
}

pub(crate) fn property_root(data_type: &SilaType) -> SilaType {
    SilaType::Structure(vec![Element {
        identifier: "value".to_owned(),
        data_type: data_type.clone(),
    }])
}

pub(crate) fn require_type(
    feature: &FeatureModel,
    root: &SilaType,
    pointer: &str,
    expected: &[Basic],
    list: bool,
) -> Result<SilaType, A2aLabError> {
    let found = pointer_type(feature, root, pointer)?;
    let effective = normalize(feature, &found)?;
    if list {
        let SilaType::List(_) = effective else {
            return Err(A2aLabError::invalid(
                "selector",
                format!("{pointer} must select a list"),
            ));
        };
        return Ok(found);
    }
    let SilaType::Basic(basic) = effective else {
        return Err(A2aLabError::invalid(
            "selector",
            format!("{pointer} must select a scalar"),
        ));
    };
    if !expected.contains(&basic) {
        return Err(A2aLabError::invalid(
            "selector",
            format!("{pointer} has the wrong SiLA type"),
        ));
    }
    Ok(found)
}

pub(crate) fn list_item(
    feature: &FeatureModel,
    data_type: &SilaType,
) -> Result<SilaType, A2aLabError> {
    match normalize(feature, data_type)? {
        SilaType::List(item) => Ok((*item).clone()),
        _ => Err(A2aLabError::invalid("selector", "must select a list")),
    }
}

pub(crate) fn is_timestamp(feature: &FeatureModel, data_type: &SilaType) -> bool {
    matches!(
        normalize(feature, data_type),
        Ok(SilaType::Basic(Basic::Timestamp))
    )
}

pub(crate) fn basic_kind(feature: &FeatureModel, data_type: &SilaType) -> Option<Basic> {
    match normalize(feature, data_type) {
        Ok(SilaType::Basic(basic)) => Some(basic),
        _ => None,
    }
}

pub(crate) fn is_structure(feature: &FeatureModel, data_type: &SilaType) -> bool {
    matches!(normalize(feature, data_type), Ok(SilaType::Structure(_)))
}

pub(crate) fn is_integer(feature: &FeatureModel, data_type: &SilaType) -> bool {
    matches!(
        normalize(feature, data_type),
        Ok(SilaType::Basic(Basic::Integer))
    )
}

pub(crate) fn pointer_type(
    feature: &FeatureModel,
    root: &SilaType,
    pointer: &str,
) -> Result<SilaType, A2aLabError> {
    let mut current = root.clone();
    for token in tokens(pointer)? {
        current = step(feature, &current, &token)?;
    }
    Ok(current)
}

pub(crate) fn select<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, A2aLabError> {
    value
        .pointer(pointer)
        .ok_or_else(|| A2aLabError::protocol(format!("response has no value at {pointer}")))
}

pub(crate) fn set_pointer(
    object: &JsonObject,
    pointer: &str,
    value: Value,
) -> Result<JsonObject, A2aLabError> {
    let mut root = Value::Object(object.as_map().clone());
    {
        let mut cursor = &mut root;
        let parts = tokens(pointer)?;
        for token in &parts[..parts.len() - 1] {
            let Value::Object(map) = cursor else {
                return Err(A2aLabError::invalid(
                    "request",
                    format!("{pointer} does not stay inside an object"),
                ));
            };
            cursor = map
                .entry(token.clone())
                .or_insert_with(|| Value::Object(Map::new()));
        }
        let Value::Object(map) = cursor else {
            return Err(A2aLabError::invalid(
                "request",
                format!("{pointer} does not stay inside an object"),
            ));
        };
        map.insert(parts[parts.len() - 1].clone(), value);
    }
    JsonObject::try_from_value(root)
}

pub(crate) fn timestamp_text(value: &Value) -> Result<UtcTimestamp, A2aLabError> {
    let text = value
        .as_str()
        .ok_or_else(|| A2aLabError::protocol("timestamp must be a string"))?;
    let normalized = if text.contains('T') {
        text.to_owned()
    } else {
        text.replacen(' ', "T", 1)
    };
    UtcTimestamp::parse(&normalized)
}

pub(crate) fn sila_timestamp(value: UtcTimestamp) -> String {
    let text = value.to_rfc3339();
    let text = text.trim_end_matches('Z');
    let (body, fraction) = text.split_once('.').unwrap_or((text, "000"));
    let millis = fraction.chars().take(3).collect::<String>();
    let millis = format!("{millis:0<3}");
    format!("{body}.{millis}+00:00").replacen('T', " ", 1)
}

fn step(feature: &FeatureModel, current: &SilaType, token: &str) -> Result<SilaType, A2aLabError> {
    match normalize(feature, current)? {
        SilaType::Structure(elements) => elements
            .into_iter()
            .find(|element| element.identifier == token)
            .map(|element| element.data_type)
            .ok_or_else(|| A2aLabError::invalid("selector", format!("unknown field {token}"))),
        SilaType::List(item) => {
            if token.chars().all(|character| character.is_ascii_digit()) {
                Ok(*item)
            } else {
                step(feature, &item, token)
            }
        }
        _ => Err(A2aLabError::invalid(
            "selector",
            format!("{token} is not inside a structure"),
        )),
    }
}

fn normalize(feature: &FeatureModel, data_type: &SilaType) -> Result<SilaType, A2aLabError> {
    match data_type {
        SilaType::Constrained { inner, .. } => normalize(feature, inner),
        SilaType::Reference(name) => {
            let inner = feature.data_type(name).ok_or_else(|| {
                A2aLabError::invalid("selector", format!("unknown data type {name}"))
            })?;
            normalize(feature, inner)
        }
        other => Ok(other.clone()),
    }
}

fn tokens(pointer: &str) -> Result<Vec<String>, A2aLabError> {
    if !pointer.starts_with('/') {
        return Err(A2aLabError::invalid("selector", "must be a JSON pointer"));
    }
    pointer
        .split('/')
        .skip(1)
        .map(|token| {
            if token.is_empty() {
                return Err(A2aLabError::invalid("selector", "must be a JSON pointer"));
            }
            Ok(token.replace("~1", "/").replace("~0", "~"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::json_object::JsonObject;
    use crate::sila::consumer::fdl::parse_feature;
    use crate::sila::consumer::model::Basic;
    use crate::sila::consumer::select::{self, command_root};
    use crate::time::UtcTimestamp;

    const FEATURE: &str = r#"
<Feature SiLA2Version="1.0" FeatureVersion="1.0" Originator="org.a2alab" Category="tests">
  <Identifier>Mapped</Identifier>
  <DisplayName>Mapped</DisplayName>
  <Description>Mapped</Description>
  <Command>
    <Identifier>Read</Identifier>
    <DisplayName>Read</DisplayName>
    <Description>Read</Description>
    <Observable>No</Observable>
    <Parameter>
      <Identifier>Start</Identifier>
      <DisplayName>Start</DisplayName>
      <Description>Start</Description>
      <DataType><Basic>Timestamp</Basic></DataType>
    </Parameter>
    <Response>
      <Identifier>Records</Identifier>
      <DisplayName>Records</DisplayName>
      <Description>Records</Description>
      <DataType><List><DataType><Structure>
        <Element>
          <Identifier>Timestamp</Identifier>
          <DisplayName>Timestamp</DisplayName>
          <Description>Timestamp</Description>
          <DataType><Basic>Timestamp</Basic></DataType>
        </Element>
        <Element>
          <Identifier>Value</Identifier>
          <DisplayName>Value</DisplayName>
          <Description>Value</Description>
          <DataType><Basic>Real</Basic></DataType>
        </Element>
      </Structure></DataType></List></DataType>
    </Response>
  </Command>
</Feature>
"#;

    #[test]
    fn checks_list_and_scalar_selectors() {
        let feature = parse_feature(FEATURE).unwrap();
        let command = feature.command("Read").unwrap();
        let response = command_root(&command.responses);
        let records = select::require_type(&feature, &response, "/Records", &[], true).unwrap();
        let item = select::list_item(&feature, &records).unwrap();
        select::require_type(
            &feature,
            &item,
            "/Timestamp",
            &[Basic::Timestamp, Basic::String],
            false,
        )
        .unwrap();
        assert!(select::require_type(&feature, &item, "/Value", &[Basic::String], false).is_err());
        assert!(select::pointer_type(&feature, &response, "/Missing").is_err());
    }

    #[test]
    fn sets_a_nested_pointer_and_formats_timestamps() {
        let object = JsonObject::parse(r#"{"Range":{}}"#).unwrap();
        let updated = select::set_pointer(&object, "/Range/Start", json!("stamp")).unwrap();
        assert_eq!(updated.as_map()["Range"]["Start"].as_str(), Some("stamp"));
        let stamp = UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap();
        assert_eq!(
            select::sila_timestamp(stamp),
            "2024-01-01 00:30:00.000+00:00"
        );
        let value = json!("2024-01-01 00:30:00.000+00:00");
        assert_eq!(select::timestamp_text(&value).unwrap(), stamp);
    }
}
