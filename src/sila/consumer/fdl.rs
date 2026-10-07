//! Feature Definition XML parser.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::A2aLabError;
use crate::sila::consumer::model::{
    Basic, CommandModel, Constraints, DataTypeModel, Element, FeatureModel, MetadataModel,
    PropertyModel, SilaType,
};

/// Parses one Feature Definition document.
pub(crate) fn parse_feature(xml: &str) -> Result<FeatureModel, A2aLabError> {
    let root = parse_tree(xml)?;
    let feature = root
        .children
        .iter()
        .find(|node| node.name == "Feature")
        .or(if root.name == "Feature" {
            Some(&root)
        } else {
            None
        })
        .ok_or_else(|| A2aLabError::protocol("Feature Definition has no Feature element"))?;
    feature_from(feature, xml)
}

fn feature_from(node: &XmlNode, xml: &str) -> Result<FeatureModel, A2aLabError> {
    let originator = attr(node, "Originator")?;
    let category = attr(node, "Category")?;
    let major = feature_major(node.attrs.get("FeatureVersion").map(String::as_str))?;
    let identifier = child_text(node, "Identifier")?;
    let mut model = FeatureModel {
        originator,
        category,
        identifier,
        major,
        display_name: optional_text(node, "DisplayName"),
        description: optional_text(node, "Description"),
        commands: Vec::new(),
        properties: Vec::new(),
        metadata: Vec::new(),
        data_types: Vec::new(),
        xml: xml.to_owned(),
    };
    for child in &node.children {
        match child.name.as_str() {
            "Command" => model.commands.push(command(child)?),
            "Property" => model.properties.push(property(child)?),
            "Metadata" => model.metadata.push(metadata(child)?),
            "DataTypeDefinition" => model.data_types.push(data_type(child)?),
            _ => {}
        }
    }
    Ok(model)
}

fn command(node: &XmlNode) -> Result<CommandModel, A2aLabError> {
    Ok(CommandModel {
        identifier: child_text(node, "Identifier")?,
        display_name: optional_text(node, "DisplayName"),
        description: optional_text(node, "Description"),
        observable: optional_text(node, "Observable").eq_ignore_ascii_case("yes"),
        parameters: elements(node, "Parameter")?,
        responses: elements(node, "Response")?,
        intermediate: elements(node, "IntermediateResponse")?,
    })
}

fn property(node: &XmlNode) -> Result<PropertyModel, A2aLabError> {
    Ok(PropertyModel {
        identifier: child_text(node, "Identifier")?,
        display_name: optional_text(node, "DisplayName"),
        description: optional_text(node, "Description"),
        observable: optional_text(node, "Observable").eq_ignore_ascii_case("yes"),
        data_type: data_type_of(node)?,
    })
}

fn metadata(node: &XmlNode) -> Result<MetadataModel, A2aLabError> {
    Ok(MetadataModel {
        identifier: child_text(node, "Identifier")?,
        data_type: data_type_of(node)?,
    })
}

fn data_type(node: &XmlNode) -> Result<DataTypeModel, A2aLabError> {
    Ok(DataTypeModel {
        identifier: child_text(node, "Identifier")?,
        data_type: data_type_of(node)?,
    })
}

fn elements(node: &XmlNode, name: &str) -> Result<Vec<Element>, A2aLabError> {
    node.children
        .iter()
        .filter(|child| child.name == name)
        .map(|child| {
            Ok(Element {
                identifier: child_text(child, "Identifier")?,
                data_type: data_type_of(child)?,
            })
        })
        .collect()
}

fn data_type_of(node: &XmlNode) -> Result<SilaType, A2aLabError> {
    let data = node
        .children
        .iter()
        .find(|child| child.name == "DataType")
        .ok_or_else(|| A2aLabError::protocol("SiLA element is missing DataType"))?;
    parse_data_type(data)
}

fn parse_data_type(node: &XmlNode) -> Result<SilaType, A2aLabError> {
    let inner = node
        .children
        .first()
        .ok_or_else(|| A2aLabError::protocol("SiLA DataType is empty"))?;
    match inner.name.as_str() {
        "Basic" => Ok(SilaType::Basic(basic(&inner.text)?)),
        "List" => {
            let item = inner
                .children
                .iter()
                .find(|child| child.name == "DataType")
                .ok_or_else(|| A2aLabError::protocol("SiLA list is missing DataType"))?;
            Ok(SilaType::List(Box::new(parse_data_type(item)?)))
        }
        "Structure" => Ok(SilaType::Structure(elements(inner, "Element")?)),
        "DataTypeIdentifier" => Ok(SilaType::Reference(inner.text.trim().to_owned())),
        "Constrained" => {
            let data = inner
                .children
                .iter()
                .find(|child| child.name == "DataType")
                .ok_or_else(|| A2aLabError::protocol("constrained type is missing DataType"))?;
            let constraints = inner
                .children
                .iter()
                .find(|child| child.name == "Constraints")
                .map(constraints)
                .unwrap_or_default();
            Ok(SilaType::Constrained {
                inner: Box::new(parse_data_type(data)?),
                constraints,
            })
        }
        other => Err(A2aLabError::protocol(format!(
            "unsupported SiLA data type {other}"
        ))),
    }
}

fn constraints(node: &XmlNode) -> Constraints {
    let mut parsed = Constraints::default();
    for child in &node.children {
        match child.name.as_str() {
            "Length" => parsed.length = child.text.trim().parse().ok(),
            "MinimalLength" => parsed.min_length = child.text.trim().parse().ok(),
            "MaximalLength" => parsed.max_length = child.text.trim().parse().ok(),
            "Pattern" => parsed.pattern = Some(child.text.trim().to_owned()),
            "FullyQualifiedIdentifier" => parsed.fully_qualified_identifier = true,
            "MinimalInclusive" => parsed.minimum = child.text.trim().parse().ok(),
            "MaximalInclusive" => parsed.maximum = child.text.trim().parse().ok(),
            "MinimalExclusive" => parsed.exclusive_minimum = child.text.trim().parse().ok(),
            "MaximalExclusive" => parsed.exclusive_maximum = child.text.trim().parse().ok(),
            "ElementCount" => parsed.element_count = child.text.trim().parse().ok(),
            "MinimalElementCount" => parsed.min_elements = child.text.trim().parse().ok(),
            "MaximalElementCount" => parsed.max_elements = child.text.trim().parse().ok(),
            "Unit" => {
                parsed.unit = child
                    .children
                    .iter()
                    .find(|item| item.name == "Label")
                    .map(|item| item.text.trim().to_owned());
            }
            "Set" => {
                parsed.enumeration = child
                    .children
                    .iter()
                    .filter(|item| item.name == "Value")
                    .map(|item| item.text.trim().to_owned())
                    .collect();
            }
            _ => {}
        }
    }
    parsed
}

fn basic(text: &str) -> Result<Basic, A2aLabError> {
    match text.trim() {
        "String" => Ok(Basic::String),
        "Integer" => Ok(Basic::Integer),
        "Real" => Ok(Basic::Real),
        "Boolean" => Ok(Basic::Boolean),
        "Binary" => Ok(Basic::Binary),
        "Date" => Ok(Basic::Date),
        "Time" => Ok(Basic::Time),
        "Timestamp" => Ok(Basic::Timestamp),
        "Any" => Ok(Basic::Any),
        other => Err(A2aLabError::protocol(format!(
            "unsupported SiLA basic type {other}"
        ))),
    }
}

fn feature_major(version: Option<&str>) -> Result<u32, A2aLabError> {
    let version = version.unwrap_or("1");
    version
        .split(['.', '_'])
        .next()
        .unwrap_or("1")
        .parse()
        .map_err(|_| A2aLabError::protocol("FeatureVersion major is not a number"))
}

fn attr(node: &XmlNode, name: &str) -> Result<String, A2aLabError> {
    node.attrs
        .get(name)
        .cloned()
        .ok_or_else(|| A2aLabError::protocol(format!("Feature is missing the {name} attribute")))
}

fn child_text(node: &XmlNode, name: &str) -> Result<String, A2aLabError> {
    node.children
        .iter()
        .find(|child| child.name == name)
        .map(|child| child.text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| A2aLabError::protocol(format!("SiLA element is missing {name}")))
}

fn optional_text(node: &XmlNode, name: &str) -> String {
    node.children
        .iter()
        .find(|child| child.name == name)
        .map(|child| child.text.trim().to_owned())
        .unwrap_or_default()
}

struct XmlNode {
    name: String,
    attrs: std::collections::HashMap<String, String>,
    text: String,
    children: Vec<XmlNode>,
}

fn parse_tree(xml: &str) -> Result<XmlNode, A2aLabError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack = vec![XmlNode {
        name: "document".to_owned(),
        attrs: std::collections::HashMap::new(),
        text: String::new(),
        children: Vec::new(),
    }];
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => stack.push(node_from(event.name().as_ref(), &event)?),
            Ok(Event::Empty(event)) => {
                let node = node_from(event.name().as_ref(), &event)?;
                stack
                    .last_mut()
                    .ok_or_else(|| A2aLabError::protocol("SiLA XML ended early"))?
                    .children
                    .push(node);
            }
            Ok(Event::Text(text)) => {
                let decoded = String::from_utf8_lossy(text.as_ref());
                stack
                    .last_mut()
                    .ok_or_else(|| A2aLabError::protocol("SiLA XML ended early"))?
                    .text
                    .push_str(&decoded);
            }
            Ok(Event::End(_)) => {
                let done = stack
                    .pop()
                    .ok_or_else(|| A2aLabError::protocol("SiLA XML ended early"))?;
                stack
                    .last_mut()
                    .ok_or_else(|| A2aLabError::protocol("SiLA XML ended early"))?
                    .children
                    .push(done);
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(A2aLabError::protocol(error.to_string())),
            _ => {}
        }
        buffer.clear();
    }
    Ok(stack.pop().unwrap_or(XmlNode {
        name: "document".to_owned(),
        attrs: std::collections::HashMap::new(),
        text: String::new(),
        children: Vec::new(),
    }))
}

fn node_from(
    name: &[u8],
    event: &quick_xml::events::BytesStart<'_>,
) -> Result<XmlNode, A2aLabError> {
    let raw = String::from_utf8_lossy(name);
    let local = raw.rsplit('}').next().unwrap_or(&raw).to_owned();
    let mut attrs = std::collections::HashMap::new();
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let key = String::from_utf8_lossy(attribute.key.as_ref());
        let key = key.rsplit('}').next().unwrap_or(&key).to_owned();
        let value = attribute
            .unescape_value()
            .map_err(|error| A2aLabError::protocol(error.to_string()))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok(XmlNode {
        name: local,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}
