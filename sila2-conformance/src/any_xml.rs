//! Validator for SiLA Any type XML (`AnyTypeDataType.xsd`).

const NS: &str = "http://www.sila-standard.org";

const BASICS: &[&str] = &[
    "String",
    "Integer",
    "Real",
    "Boolean",
    "Binary",
    "Date",
    "Time",
    "Timestamp",
    "Any",
];

const UNITS: &[&str] = &[
    "Dimensionless",
    "Meter",
    "Kilogram",
    "Second",
    "Ampere",
    "Kelvin",
    "Mole",
    "Candela",
];

const IDENTIFIERS: &[&str] = &[
    "FeatureIdentifier",
    "CommandIdentifier",
    "CommandParameterIdentifier",
    "CommandResponseIdentifier",
    "IntermediateCommandResponseIdentifier",
    "DefinedExecutionErrorIdentifier",
    "PropertyIdentifier",
    "TypeIdentifier",
    "MetadataIdentifier",
];

pub fn is_valid_any_type_xml(xml: &str) -> bool {
    parse_root(xml).is_ok_and(|root| validate_data_type(&root))
}

type Attr = (String, String);
type TagHead = (String, Vec<Attr>, bool);

struct Elem {
    name: String,
    ns: String,
    text: String,
    children: Vec<Elem>,
}

struct Cursor<'a> {
    rest: &'a str,
}

fn validate_data_type(elem: &Elem) -> bool {
    if !named(elem, "DataType") {
        return false;
    }
    let children = elements(elem);
    let Some(child) = only(children) else {
        return false;
    };
    match child.name.as_str() {
        "Basic" => validate_basic(child),
        "List" => validate_list(child),
        "Structure" => validate_structure(child),
        "Constrained" => validate_constrained(child),
        "DataTypeIdentifier" => child.children.is_empty() && is_identifier(&child.text),
        _ => false,
    }
}

fn validate_basic(elem: &Elem) -> bool {
    named(elem, "Basic") && elem.children.is_empty() && BASICS.contains(&elem.text.as_str())
}

fn validate_list(elem: &Elem) -> bool {
    named(elem, "List")
        && elements(elem)
            .first()
            .is_some_and(|child| validate_data_type(child))
        && elements(elem).len() == 1
}

fn validate_structure(elem: &Elem) -> bool {
    named(elem, "Structure")
        && !elements(elem).is_empty()
        && elements(elem).iter().all(|child| validate_element(child))
}

fn validate_element(elem: &Elem) -> bool {
    if !named(elem, "Element") {
        return false;
    }
    let children = elements(elem);
    let [identifier, display, description, data_type] = children.as_slice() else {
        return false;
    };
    named(identifier, "Identifier")
        && identifier.children.is_empty()
        && is_identifier(&identifier.text)
        && named(display, "DisplayName")
        && display.children.is_empty()
        && display.text.chars().count() <= 255
        && named(description, "Description")
        && description.children.is_empty()
        && validate_data_type(data_type)
}

fn validate_constrained(elem: &Elem) -> bool {
    if !named(elem, "Constrained") {
        return false;
    }
    let children = elements(elem);
    matches!(children.as_slice(), [data_type, constraints] if validate_data_type(data_type) && validate_constraints(constraints))
}

fn validate_constraints(elem: &Elem) -> bool {
    if !named(elem, "Constraints") {
        return false;
    }
    let mut seen = Vec::new();
    for child in elements(elem) {
        if seen.contains(&child.name.as_str()) || !validate_constraint(child) {
            return false;
        }
        seen.push(child.name.as_str());
    }
    true
}

fn validate_constraint(elem: &Elem) -> bool {
    match elem.name.as_str() {
        "Length" => elem.children.is_empty() && is_non_negative(&elem.text),
        "MinimalLength"
        | "MaximalLength"
        | "ElementCount"
        | "MinimalElementCount"
        | "MaximalElementCount" => elem.children.is_empty() && is_positive(&elem.text),
        "Pattern" | "MaximalExclusive" | "MaximalInclusive" | "MinimalExclusive"
        | "MinimalInclusive" => elem.children.is_empty(),
        "FullyQualifiedIdentifier" => {
            elem.children.is_empty() && IDENTIFIERS.contains(&elem.text.as_str())
        }
        "Set" => validate_set(elem),
        "Unit" => validate_unit(elem),
        "ContentType" => validate_content_type(elem),
        "Schema" => validate_schema(elem),
        "AllowedTypes" => {
            !elements(elem).is_empty()
                && elements(elem).iter().all(|child| validate_data_type(child))
        }
        _ => false,
    }
}

fn validate_set(elem: &Elem) -> bool {
    let children = elements(elem);
    !children.is_empty()
        && children
            .iter()
            .all(|child| named(child, "Value") && child.children.is_empty())
}

fn validate_unit(elem: &Elem) -> bool {
    let children = elements(elem);
    let Some((label, factor, offset, components)) =
        children.split_first().and_then(|(label, rest)| {
            let (factor, rest) = rest.split_first()?;
            let (offset, components) = rest.split_first()?;
            Some((label, factor, offset, components))
        })
    else {
        return false;
    };
    named(label, "Label")
        && label.children.is_empty()
        && named(factor, "Factor")
        && factor.children.is_empty()
        && is_decimal(&factor.text)
        && named(offset, "Offset")
        && offset.children.is_empty()
        && is_decimal(&offset.text)
        && !components.is_empty()
        && components
            .iter()
            .all(|child| validate_unit_component(child))
}

fn validate_unit_component(elem: &Elem) -> bool {
    let children = elements(elem);
    matches!(children.as_slice(), [unit, exponent]
        if named(elem, "UnitComponent")
            && named(unit, "SIUnit")
            && unit.children.is_empty()
            && UNITS.contains(&unit.text.as_str())
            && named(exponent, "Exponent")
            && exponent.children.is_empty()
            && is_integer(&exponent.text))
}

fn validate_content_type(elem: &Elem) -> bool {
    let children = elements(elem);
    let Some((kind, rest)) = children.split_first() else {
        return false;
    };
    let Some((subtype, rest)) = rest.split_first() else {
        return false;
    };
    named(elem, "ContentType")
        && named(kind, "Type")
        && kind.children.is_empty()
        && named(subtype, "Subtype")
        && subtype.children.is_empty()
        && match rest {
            [] => true,
            [parameters] => validate_parameters(parameters),
            _ => false,
        }
}

fn validate_parameters(elem: &Elem) -> bool {
    named(elem, "Parameters")
        && !elements(elem).is_empty()
        && elements(elem).iter().all(|parameter| {
            let children = elements(parameter);
            matches!(children.as_slice(), [attribute, value]
                if named(parameter, "Parameter")
                    && named(attribute, "Attribute")
                    && attribute.children.is_empty()
                    && named(value, "Value")
                    && value.children.is_empty())
        })
}

fn validate_schema(elem: &Elem) -> bool {
    let children = elements(elem);
    matches!(children.as_slice(), [kind, location]
        if named(kind, "Type")
            && kind.children.is_empty()
            && matches!(kind.text.as_str(), "Xml" | "Json")
            && location.children.is_empty()
            && matches!(location.name.as_str(), "Url" | "Inline"))
}

fn named(elem: &Elem, name: &str) -> bool {
    elem.ns == NS && elem.name == name
}

fn elements(elem: &Elem) -> Vec<&Elem> {
    elem.children
        .iter()
        .filter(|child| child.ns == NS || !child.name.is_empty())
        .collect()
}

fn only(children: Vec<&Elem>) -> Option<&Elem> {
    if children.len() == 1 {
        children.first().copied()
    } else {
        None
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
        && value.chars().count() <= 255
}

fn is_non_negative(value: &str) -> bool {
    let digits = value.strip_prefix('+').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_positive(value: &str) -> bool {
    let digits = value.strip_prefix('+').unwrap_or(value);
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && digits.bytes().any(|byte| byte != b'0')
}

fn is_integer(value: &str) -> bool {
    let digits = integer_body(value).unwrap_or("");
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn integer_body(value: &str) -> Option<&str> {
    value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .or(Some(value))
}

fn is_decimal(value: &str) -> bool {
    let body = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    let mut parts = body.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    parts.next().is_none()
        && !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn parse_root(xml: &str) -> Result<Elem, ()> {
    let mut cursor = Cursor { rest: xml.trim() };
    cursor.skip_misc()?;
    let root = cursor.parse_element("")?;
    cursor.skip_misc()?;
    if cursor.rest.is_empty() {
        Ok(root)
    } else {
        Err(())
    }
}

impl Cursor<'_> {
    fn skip_misc(&mut self) -> Result<(), ()> {
        loop {
            self.skip_ws();
            if self.rest.starts_with("<?") {
                self.skip_until("?>")?;
            } else if self.rest.starts_with("<!--") {
                self.skip_until("-->")?;
            } else if self.rest.starts_with("<!") {
                return Err(());
            } else {
                return Ok(());
            }
        }
    }

    fn parse_element(&mut self, parent_ns: &str) -> Result<Elem, ()> {
        self.skip_ws();
        self.consume("<")?;
        if self.rest.starts_with('/') || self.rest.starts_with('!') || self.rest.starts_with('?') {
            return Err(());
        }
        let qualified = self.read_name()?;
        let (mut ns, prefixes, empty) = self.read_tag(parent_ns)?;
        let (prefix, name) = split_name(&qualified);
        if let Some(prefix) = prefix {
            ns = prefixes
                .iter()
                .find(|(key, _)| key == prefix)
                .map(|(_, uri)| uri.clone())
                .ok_or(())?;
        }
        if empty {
            return Ok(Elem {
                name,
                ns,
                text: String::new(),
                children: Vec::new(),
            });
        }
        self.read_body(name, ns)
    }

    fn read_body(&mut self, name: String, ns: String) -> Result<Elem, ()> {
        let mut text = String::new();
        let mut children = Vec::new();
        loop {
            if self.rest.is_empty() {
                return Err(());
            }
            if self.rest.starts_with("</") {
                self.consume("</")?;
                let closing = self.read_name()?;
                self.skip_ws();
                self.consume(">")?;
                if closing == name || closing.ends_with(&format!(":{name}")) {
                    return Ok(Elem {
                        name,
                        ns,
                        text: text.trim().to_owned(),
                        children,
                    });
                }
                return Err(());
            }
            if self.rest.starts_with('<') {
                children.push(self.parse_element(&ns)?);
            } else {
                text.push_str(&self.read_text()?);
            }
        }
    }

    fn read_tag(&mut self, parent_ns: &str) -> Result<TagHead, ()> {
        let mut ns = parent_ns.to_owned();
        let mut prefixes = Vec::new();
        loop {
            self.skip_ws();
            if self.consume("/>").is_ok() {
                return Ok((ns, prefixes, true));
            }
            if self.consume(">").is_ok() {
                return Ok((ns, prefixes, false));
            }
            let (key, value) = self.read_attribute()?;
            if key == "xmlns" {
                ns = value;
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                prefixes.push((prefix.to_owned(), value));
            }
        }
    }

    fn read_attribute(&mut self) -> Result<(String, String), ()> {
        let name = self.read_name()?;
        self.skip_ws();
        self.consume("=")?;
        self.skip_ws();
        let quote = self.rest.chars().next().ok_or(())?;
        if quote != '"' && quote != '\'' {
            return Err(());
        }
        self.bump(1);
        let end = self.rest.find(quote).ok_or(())?;
        let value = self.rest.get(..end).ok_or(())?.to_owned();
        self.bump(end + 1);
        Ok((name, decode_entities(&value)?))
    }

    fn read_text(&mut self) -> Result<String, ()> {
        let end = self.rest.find('<').unwrap_or(self.rest.len());
        let raw = self.rest.get(..end).ok_or(())?;
        self.bump(end);
        decode_entities(raw)
    }

    fn read_name(&mut self) -> Result<String, ()> {
        let length = self
            .rest
            .char_indices()
            .take_while(|(_, character)| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | ':' | '-' | '.')
            })
            .last()
            .map_or(0, |(index, character)| index + character.len_utf8());
        if length == 0 {
            return Err(());
        }
        let name = self.rest.get(..length).ok_or(())?.to_owned();
        self.bump(length);
        Ok(name)
    }

    fn skip_until(&mut self, marker: &str) -> Result<(), ()> {
        let end = self.rest.find(marker).ok_or(())? + marker.len();
        self.bump(end);
        Ok(())
    }

    fn skip_ws(&mut self) {
        let trimmed = self.rest.trim_start();
        self.rest = trimmed;
    }

    fn consume(&mut self, token: &str) -> Result<(), ()> {
        if self.rest.starts_with(token) {
            self.bump(token.len());
            Ok(())
        } else {
            Err(())
        }
    }

    fn bump(&mut self, bytes: usize) {
        self.rest = self.rest.get(bytes..).unwrap_or("");
    }
}

fn split_name(qualified: &str) -> (Option<&str>, String) {
    match qualified.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local.to_owned()),
        None => (None, qualified.to_owned()),
    }
}

fn decode_entities(value: &str) -> Result<String, ()> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        out.push_str(rest.get(..index).ok_or(())?);
        rest = rest.get(index + 1..).ok_or(())?;
        let end = rest.find(';').ok_or(())?;
        let entity = rest.get(..end).ok_or(())?;
        out.push(match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => return Err(()),
        });
        rest = rest.get(end + 1..).ok_or(())?;
    }
    out.push_str(rest);
    Ok(out)
}
