//! Encoding and JSON mapping for the IDTA AAS HTTP API.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::catalog::{
    Asset, AssetKey, Binding, BindingRole, Endpoint, OpcUaIdentityKind, SecurityMode, SemanticId,
    SemanticKind,
};
use crate::error::SdkError;

pub(crate) const BINDING_SEMANTIC: &str = "https://a2a-lab.example/LabBindings/1/0";

pub(crate) fn base64url(value: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = value.as_bytes();
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b11) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b1111) << 2) | (third >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b11_1111) as usize] as char);
        }
    }
    encoded
}

pub(crate) fn supports_repository(description: &Value) -> bool {
    description
        .get("profiles")
        .and_then(Value::as_array)
        .is_some_and(|profiles| {
            profiles.iter().any(|profile| {
                profile.as_str().is_some_and(|profile| {
                    profile.contains("3.2")
                        && profile
                            .contains("AssetAdministrationShellRepositoryServiceSpecification")
                })
            })
        })
}

pub(crate) fn shells(document: &Value) -> Vec<Value> {
    document
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn assets_from_shells(
    shells: &[Value],
    submodels: &[Value],
) -> Result<Vec<Asset>, SdkError> {
    let mut assets = Vec::new();
    for shell in shells {
        let id = text(shell, "id").ok_or_else(|| SdkError::protocol("shell is missing id"))?;
        let key = AssetKey::new(id)?;
        let global = shell
            .pointer("/assetInformation/globalAssetId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut bindings = Vec::new();
        for reference in submodel_ids(shell) {
            if let Some(submodel) = submodels
                .iter()
                .find(|submodel| text(submodel, "id") == Some(reference))
                && is_binding_submodel(submodel)
            {
                bindings.extend(bindings_from_submodel(&key, submodel)?);
            }
        }
        assets.push(Asset::new(key, global, bindings));
    }
    Ok(assets)
}

fn submodel_ids(shell: &Value) -> Vec<&str> {
    shell
        .get("submodels")
        .and_then(Value::as_array)
        .map(|references| {
            references
                .iter()
                .filter_map(|reference| reference.pointer("/keys/0/value").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

fn is_binding_submodel(submodel: &Value) -> bool {
    semantic_value(submodel) == Some(BINDING_SEMANTIC)
}

fn bindings_from_submodel(asset: &AssetKey, submodel: &Value) -> Result<Vec<Binding>, SdkError> {
    let Some(elements) = submodel.get("submodelElements").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    elements
        .iter()
        .map(|element| binding_from_element(asset, element))
        .collect()
}

fn binding_from_element(asset: &AssetKey, element: &Value) -> Result<Binding, SdkError> {
    let fields = properties(element);
    let lab_id = field(&fields, "labId")?;
    let role = match field(&fields, "role")? {
        "log_source" => BindingRole::LogSource,
        "metric" => BindingRole::Metric,
        "task" => BindingRole::Task,
        other => {
            return Err(SdkError::protocol(format!(
                "unknown binding role `{other}`"
            )));
        }
    };
    let semantic = SemanticId::new(
        SemanticKind::Iri,
        semantic_value(element).unwrap_or(BINDING_SEMANTIC),
    )?;
    let endpoint = match field(&fields, "protocol")? {
        "opc_ua" => Endpoint::OpcUa {
            url: field(&fields, "url")?.to_owned(),
            security_policy: field(&fields, "securityPolicy")?.to_owned(),
            security_mode: security_mode(field(&fields, "securityMode")?)?,
            identity: identity(field(&fields, "identity")?)?,
            node_id: field(&fields, "nodeId")?.to_owned(),
            namespace_uri: field(&fields, "namespaceUri")?.to_owned(),
            browse_path: fields.get("browsePath").cloned().unwrap_or_default(),
        },
        "sila2" => Endpoint::Sila2 {
            host: field(&fields, "host")?.to_owned(),
            port: field(&fields, "port")?
                .parse()
                .map_err(|_| SdkError::protocol("SiLA port is not a number"))?,
            feature: field(&fields, "feature")?.to_owned(),
            member: field(&fields, "member")?.to_owned(),
            version: field(&fields, "version")?.to_owned(),
        },
        other => return Err(SdkError::protocol(format!("unknown protocol `{other}`"))),
    };
    Binding::new(lab_id, asset.clone(), semantic, role, endpoint)
}

fn properties(element: &Value) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    if let Some(children) = element.get("value").and_then(Value::as_array) {
        for child in children {
            if let (Some(name), Some(value)) = (text(child, "idShort"), text(child, "value")) {
                fields.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    fields
}

fn field<'a>(fields: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, SdkError> {
    fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| SdkError::protocol(format!("binding is missing `{name}`")))
}

fn security_mode(value: &str) -> Result<SecurityMode, SdkError> {
    match value {
        "sign" => Ok(SecurityMode::Sign),
        "sign_and_encrypt" => Ok(SecurityMode::SignAndEncrypt),
        _ => Err(SdkError::protocol("unsupported OPC UA security mode")),
    }
}

fn identity(value: &str) -> Result<OpcUaIdentityKind, SdkError> {
    match value {
        "username" => Ok(OpcUaIdentityKind::Username),
        "certificate" => Ok(OpcUaIdentityKind::Certificate),
        _ => Err(SdkError::protocol("unsupported OPC UA identity")),
    }
}

fn semantic_value(value: &Value) -> Option<&str> {
    value
        .pointer("/semanticId/keys/0/value")
        .and_then(Value::as_str)
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
