//! Agent Card view and the security schemes this page can satisfy.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::Value;

use crate::codec::redact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub name: String,
    pub description: String,
    pub version: String,
    pub interfaces: Vec<Interface>,
    pub skills: Vec<Skill>,
    pub streaming: Option<bool>,
    pub push_notifications: Option<bool>,
    pub extended_agent_card: Option<bool>,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub binding: String,
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Anonymous { unsupported: Vec<String> },
    Bearer { unsupported: Vec<String> },
    Unsupported { reasons: Vec<String> },
}

impl Card {
    pub fn supports_http_json(&self) -> bool {
        self.interfaces
            .iter()
            .any(|interface| interface.binding == "HTTP+JSON")
    }

    pub fn offers_agent_message(&self) -> bool {
        self.skills.iter().any(|skill| skill.id == "agent-message")
    }

    pub fn access_label(&self) -> String {
        match &self.access {
            Access::Anonymous { .. } => "No credential is required.".to_owned(),
            Access::Bearer { .. } => "Enter a bearer token for this page only.".to_owned(),
            Access::Unsupported { .. } => {
                "This page cannot satisfy the agent security requirements.".to_owned()
            }
        }
    }

    pub fn security_lines(&self) -> Vec<String> {
        let unsupported = match &self.access {
            Access::Anonymous { unsupported } | Access::Bearer { unsupported } => unsupported,
            Access::Unsupported { reasons } => reasons,
        };
        let mut lines = Vec::new();
        if matches!(self.access, Access::Bearer { .. }) {
            lines.push("Bearer: accepted for this page only.".to_owned());
        }
        if matches!(self.access, Access::Anonymous { .. }) {
            lines.push("Anonymous: no credential is sent.".to_owned());
        }
        lines.extend(unsupported.iter().cloned());
        if lines.is_empty() {
            lines.push("No security requirements are advertised.".to_owned());
        }
        lines
    }
}

pub fn parse_card(body: &str) -> Result<Card, ()> {
    let value: Value = serde_json::from_str(body).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    let name = required_string(object.get("name"))?;
    let version = required_string(object.get("version"))?;
    let description = optional_string(object.get("description"));
    let interfaces = object
        .get("supportedInterfaces")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_interface).collect())
        .unwrap_or_default();
    let skills = object
        .get("skills")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_skill).collect())
        .unwrap_or_default();
    let capabilities = object.get("capabilities").and_then(Value::as_object);
    let access = classify(
        object.get("securitySchemes"),
        object.get("securityRequirements"),
    );
    Ok(Card {
        name,
        description,
        version,
        interfaces,
        skills,
        streaming: flag(capabilities, "streaming"),
        push_notifications: flag(capabilities, "pushNotifications"),
        extended_agent_card: flag(capabilities, "extendedAgentCard"),
        access,
    })
}

pub fn capability_lines(card: &Card) -> Vec<String> {
    vec![
        format!("Streaming: {}", on_off(card.streaming)),
        format!("Push notifications: {}", on_off(card.push_notifications)),
        format!("Extended agent card: {}", on_off(card.extended_agent_card)),
        if card.supports_http_json() {
            "HTTP+JSON: advertised.".to_owned()
        } else {
            "HTTP+JSON: not advertised.".to_owned()
        },
    ]
}

pub fn interface_line(interface: &Interface) -> String {
    format!(
        "{}\n{}\n{}",
        interface.binding, interface.version, interface.url
    )
}

pub fn skill_line(skill: &Skill) -> String {
    format!("{}\n{}\n{}", skill.name, skill.id, skill.description)
}

pub fn redact_card(card: &Card, secret: Option<&str>) -> Card {
    let mut card = card.clone();
    card.name = redact(&card.name, secret);
    card.description = redact(&card.description, secret);
    card.version = redact(&card.version, secret);
    for interface in &mut card.interfaces {
        interface.binding = redact(&interface.binding, secret);
        interface.version = redact(&interface.version, secret);
        interface.url = redact(&interface.url, secret);
    }
    for skill in &mut card.skills {
        skill.id = redact(&skill.id, secret);
        skill.name = redact(&skill.name, secret);
        skill.description = redact(&skill.description, secret);
    }
    card
}

fn parse_interface(value: &Value) -> Option<Interface> {
    let object = value.as_object()?;
    Some(Interface {
        binding: required_string(object.get("protocolBinding")).ok()?,
        version: required_string(object.get("protocolVersion")).ok()?,
        url: required_string(object.get("url")).ok()?,
    })
}

fn parse_skill(value: &Value) -> Option<Skill> {
    let object = value.as_object()?;
    Some(Skill {
        id: required_string(object.get("id")).ok()?,
        name: required_string(object.get("name")).ok()?,
        description: optional_string(object.get("description")),
    })
}

fn classify(schemes: Option<&Value>, requirements: Option<&Value>) -> Access {
    let Some(requirements) = requirements.and_then(Value::as_array) else {
        return Access::Anonymous {
            unsupported: Vec::new(),
        };
    };
    if requirements.is_empty() {
        return Access::Anonymous {
            unsupported: Vec::new(),
        };
    }
    let mut anonymous = false;
    let mut bearer = false;
    let mut unsupported = Vec::new();
    for requirement in requirements {
        match requirement_kind(schemes, requirement) {
            Kind::Anonymous => anonymous = true,
            Kind::Bearer => bearer = true,
            Kind::Unsupported(reasons) => unsupported.extend(reasons),
        }
    }
    if bearer {
        Access::Bearer { unsupported }
    } else if anonymous {
        Access::Anonymous { unsupported }
    } else {
        Access::Unsupported {
            reasons: unsupported,
        }
    }
}

enum Kind {
    Anonymous,
    Bearer,
    Unsupported(Vec<String>),
}

fn requirement_kind(schemes: Option<&Value>, requirement: &Value) -> Kind {
    let Some(entries) = requirement.as_object() else {
        return Kind::Unsupported(vec!["A security requirement is unreadable.".to_owned()]);
    };
    if entries.is_empty() {
        return Kind::Anonymous;
    }
    let mut bearer = false;
    let mut reasons = Vec::new();
    for name in entries.keys() {
        match scheme_kind(schemes, name) {
            SchemeKind::Bearer => bearer = true,
            SchemeKind::Unsupported(reason) => reasons.push(format!("{name}: {reason}")),
        }
    }
    if reasons.is_empty() && bearer {
        Kind::Bearer
    } else {
        Kind::Unsupported(reasons)
    }
}

enum SchemeKind {
    Bearer,
    Unsupported(String),
}

fn scheme_kind(schemes: Option<&Value>, name: &str) -> SchemeKind {
    let Some(scheme) = schemes
        .and_then(Value::as_object)
        .and_then(|schemes| schemes.get(name))
    else {
        return SchemeKind::Unsupported("unsupported scheme".to_owned());
    };
    let Some(object) = scheme.as_object() else {
        return SchemeKind::Unsupported("unsupported scheme".to_owned());
    };
    if let Some(http) = object.get("httpAuthSecurityScheme") {
        let name = http.get("scheme").and_then(Value::as_str).unwrap_or("");
        if name.eq_ignore_ascii_case("bearer") {
            return SchemeKind::Bearer;
        }
        return SchemeKind::Unsupported(format!("unsupported HTTP {name} scheme"));
    }
    if object.contains_key("apiKeySecurityScheme") {
        return SchemeKind::Unsupported("unsupported API key scheme".to_owned());
    }
    if object.contains_key("oauth2SecurityScheme") {
        return SchemeKind::Unsupported("unsupported OAuth2 scheme".to_owned());
    }
    if object.contains_key("openIdConnectSecurityScheme") {
        return SchemeKind::Unsupported("unsupported OpenID Connect scheme".to_owned());
    }
    if object.contains_key("mtlsSecurityScheme") {
        return SchemeKind::Unsupported("unsupported mutual TLS scheme".to_owned());
    }
    SchemeKind::Unsupported("unsupported scheme".to_owned())
}

fn required_string(value: Option<&Value>) -> Result<String, ()> {
    value.and_then(Value::as_str).map(str::to_owned).ok_or(())
}

fn optional_string(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_owned()
}

fn flag(capabilities: Option<&serde_json::Map<String, Value>>, name: &str) -> Option<bool> {
    capabilities?.get(name)?.as_bool()
}

fn on_off(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "not advertised",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(requirements: Value, schemes: Value) -> Card {
        parse_card(
            &json!({
                "name": "<b>lab</b>",
                "description": "desc",
                "version": "0.1.0",
                "supportedInterfaces": [{
                    "url": "http://127.0.0.1:9",
                    "protocolBinding": "HTTP+JSON",
                    "protocolVersion": "1.0"
                }],
                "capabilities": {"streaming": true, "pushNotifications": false},
                "skills": [{"id": "list-tasks", "name": "List tasks", "description": "List"}],
                "securitySchemes": schemes,
                "securityRequirements": requirements
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn anonymous_card_has_no_credential_prompt() {
        let card = parse_card(
            &json!({
                "name": "a2a-lab",
                "version": "0.1.0",
                "supportedInterfaces": [],
                "skills": []
            })
            .to_string(),
        )
        .unwrap();
        assert!(matches!(card.access, Access::Anonymous { .. }));
        assert!(!card.offers_agent_message());
        assert!(!card.supports_http_json());
        assert_eq!(card.access_label(), "No credential is required.");
    }

    #[test]
    fn bearer_is_accepted_and_other_schemes_stay_unsupported() {
        let card = card(
            json!([{"bearerAuth": []}, {"oauth": []}]),
            json!({
                "bearerAuth": {"httpAuthSecurityScheme": {"scheme": "bearer"}},
                "oauth": {"oauth2SecurityScheme": {"flows": {}}}
            }),
        );
        assert!(matches!(card.access, Access::Bearer { .. }));
        let lines = card.security_lines().join("\n");
        assert!(lines.contains("Bearer: accepted"));
        assert!(lines.contains("oauth: unsupported OAuth2 scheme"));
        assert!(card.supports_http_json());
    }

    #[test]
    fn unsafe_schemes_do_not_fall_back_to_credentials() {
        for (key, value, marker) in [
            (
                "basic",
                json!({"httpAuthSecurityScheme": {"scheme": "basic"}}),
                "HTTP basic",
            ),
            (
                "key",
                json!({"apiKeySecurityScheme": {"location": "query", "name": "key"}}),
                "API key",
            ),
            (
                "oidc",
                json!({"openIdConnectSecurityScheme": {"openIdConnectUrl": "https://idp"}}),
                "OpenID",
            ),
            ("mtls", json!({"mtlsSecurityScheme": {}}), "mutual TLS"),
        ] {
            let card = card(json!([{key: []}]), json!({key: value}));
            let Access::Unsupported { reasons } = card.access else {
                panic!("expected unsupported for {key}");
            };
            assert!(
                reasons.iter().any(|reason| reason.contains(marker)),
                "{reasons:?}"
            );
        }
    }

    #[test]
    fn mixed_requirement_is_unsupported_when_any_scheme_is_unsafe() {
        let card = card(
            json!([{"bearerAuth": [], "key": []}]),
            json!({
                "bearerAuth": {"httpAuthSecurityScheme": {"scheme": "Bearer"}},
                "key": {"apiKeySecurityScheme": {"location": "header", "name": "x-api-key"}}
            }),
        );
        assert!(matches!(card.access, Access::Unsupported { .. }));
    }

    #[test]
    fn markup_in_card_fields_stays_literal() {
        let card = card(json!([]), json!({}));
        assert_eq!(card.name, "<b>lab</b>");
        assert!(skill_line(&card.skills[0]).contains("list-tasks"));
        let redacted = redact_card(&card, Some("<b>"));
        assert_eq!(redacted.name, "[redacted]lab</b>");
    }
}
