//! Configuration for a remote SiLA server used as an A2A-LAB provider.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::{MetricId, SourceId, TaskId};
use crate::json_object::JsonObject;

/// Connection and binding configuration for [`super::SilaProvider`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SilaProviderConfig {
    /// Server host or address.
    pub host: String,
    /// gRPC port.
    pub port: u16,
    /// Server UUID published by `SiLAService`.
    pub server_uuid: String,
    /// Connects without TLS. Encrypted connections stay the default.
    #[serde(default)]
    pub plaintext: bool,
    /// PEM trust anchor. Required unless [`Self::plaintext`] is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<String>,
    /// SiLA client metadata sent with every call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    /// Feature members exposed as lab tasks, logs, or metrics.
    pub bindings: Vec<SilaBinding>,
}

/// One remote feature member assigned to an A2A-LAB role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum SilaBinding {
    /// A command or readable property exposed as a task.
    Task(TaskBinding),
    /// A command or readable property invoked by log queries.
    Logs(LogBinding),
    /// A command or readable property invoked by metric queries.
    Metric(MetricBinding),
}

/// A SiLA command or readable property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SilaMember {
    /// Fully qualified feature identifier.
    pub feature: String,
    /// Command or property.
    pub kind: MemberKind,
    /// Feature member identifier.
    pub identifier: String,
    /// SiLA client metadata merged over the provider metadata for this member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    /// Parameter values included in every call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<JsonObject>,
}

/// Whether a binding calls a command or reads a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    /// An unobservable or observable command.
    Command,
    /// An unobservable property.
    Property,
}

/// Stable task metadata for one SiLA member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskBinding {
    /// A2A-LAB task identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What the task does.
    pub description: String,
    /// Optional asset identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Optional semantic identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
    /// Remote member.
    pub member: SilaMember,
}

/// Maps one SiLA response into log records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogBinding {
    /// A2A-LAB log source identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What the source contains.
    pub description: String,
    /// Optional asset identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Optional semantic identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
    /// Remote member.
    pub member: SilaMember,
    /// Query fields copied into the SiLA request.
    #[serde(default)]
    pub request: RequestBinding,
    /// JSON pointer to the record array.
    pub records: String,
    /// JSON pointer to each record timestamp, relative to one record.
    pub timestamp: String,
    /// JSON pointer to each record level, relative to one record.
    pub level: String,
    /// JSON pointer to each record message, relative to one record.
    pub message: String,
    /// JSON pointer to each record's attribute object, relative to one record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<String>,
    /// JSON pointer to the next-page cursor in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Maps one SiLA response into metric samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetricBinding {
    /// A2A-LAB metric identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What the metric measures.
    pub description: String,
    /// Unit of each sample.
    pub unit: String,
    /// Optional asset identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Optional semantic identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
    /// Remote member.
    pub member: SilaMember,
    /// Query fields copied into the SiLA request.
    #[serde(default)]
    pub request: RequestBinding,
    /// JSON pointer to the sample array.
    pub points: String,
    /// JSON pointer to each sample timestamp, relative to one sample.
    pub timestamp: String,
    /// JSON pointer to each finite numeric value, relative to one sample.
    pub value: String,
    /// JSON pointer to the next-page cursor in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// JSON pointers that receive the A2A-LAB query fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RequestBinding {
    /// Pointer that receives the range start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// Pointer that receives the range end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Pointer that receives the page cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Pointer that receives the page limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
}

impl SilaProviderConfig {
    /// Rejects an unusable endpoint, trust setting, or binding before connecting.
    pub fn check(&self) -> Result<(), A2aLabError> {
        if self.host.is_empty() || self.host.chars().any(char::is_whitespace) || self.port == 0 {
            return Err(A2aLabError::invalid(
                "sila_endpoint",
                "host and port are required",
            ));
        }
        if self.server_uuid.is_empty() {
            return Err(A2aLabError::invalid("server_uuid", "must not be empty"));
        }
        if self.plaintext {
            if self.ca_pem.is_some() {
                return Err(A2aLabError::invalid(
                    "ca_pem",
                    "plaintext connections do not use a certificate",
                ));
            }
        } else if self
            .ca_pem
            .as_deref()
            .is_none_or(|pem| pem.trim().is_empty())
        {
            return Err(A2aLabError::invalid(
                "ca_pem",
                "encrypted SiLA connections require a certificate",
            ));
        }
        let mut tasks = Vec::new();
        let mut logs = Vec::new();
        let mut metrics = Vec::new();
        for binding in &self.bindings {
            match binding {
                SilaBinding::Task(binding) => {
                    remember(
                        &mut tasks,
                        "task",
                        &binding.id,
                        TaskId::new(&binding.id).is_ok(),
                    )?;
                    check_text("name", &binding.name)?;
                    check_member(&binding.member, false)?;
                }
                SilaBinding::Logs(binding) => {
                    remember(
                        &mut logs,
                        "log source",
                        &binding.id,
                        SourceId::new(&binding.id).is_ok(),
                    )?;
                    check_text("name", &binding.name)?;
                    check_query_member(&binding.member, &binding.request)?;
                    pointer("records", &binding.records)?;
                    pointer("timestamp", &binding.timestamp)?;
                    pointer("level", &binding.level)?;
                    pointer("message", &binding.message)?;
                    optional_pointer("attributes", binding.attributes.as_deref())?;
                    optional_pointer("next_cursor", binding.next_cursor.as_deref())?;
                }
                SilaBinding::Metric(binding) => {
                    remember(
                        &mut metrics,
                        "metric",
                        &binding.id,
                        MetricId::new(&binding.id).is_ok(),
                    )?;
                    check_text("name", &binding.name)?;
                    check_text("unit", &binding.unit)?;
                    check_query_member(&binding.member, &binding.request)?;
                    pointer("points", &binding.points)?;
                    pointer("timestamp", &binding.timestamp)?;
                    pointer("value", &binding.value)?;
                    optional_pointer("next_cursor", binding.next_cursor.as_deref())?;
                }
            }
        }
        Ok(())
    }
}

impl RequestBinding {
    pub(crate) const fn has_parameters(&self) -> bool {
        self.start.is_some() || self.end.is_some() || self.cursor.is_some() || self.limit.is_some()
    }
}

fn remember(
    seen: &mut Vec<String>,
    kind: &'static str,
    id: &str,
    valid: bool,
) -> Result<(), A2aLabError> {
    if !valid {
        return Err(A2aLabError::invalid(
            "id",
            "must be 1..=128 ASCII letters, digits, or . _ : -",
        ));
    }
    if seen.iter().any(|existing| existing == id) {
        return Err(A2aLabError::invalid(
            "id",
            format!("{kind} `{id}` is configured more than once"),
        ));
    }
    seen.push(id.to_owned());
    Ok(())
}

fn check_member(member: &SilaMember, query: bool) -> Result<(), A2aLabError> {
    if member.feature.is_empty() || member.identifier.is_empty() {
        return Err(A2aLabError::invalid(
            "member",
            "feature and identifier are required",
        ));
    }
    if query
        && member.kind == MemberKind::Property
        && member
            .arguments
            .as_ref()
            .is_some_and(|arguments| !arguments.as_map().is_empty())
    {
        return Err(A2aLabError::invalid(
            "arguments",
            "a property read has no command parameters",
        ));
    }
    Ok(())
}

fn check_query_member(member: &SilaMember, request: &RequestBinding) -> Result<(), A2aLabError> {
    check_member(member, true)?;
    if member.kind == MemberKind::Property && request.has_parameters() {
        return Err(A2aLabError::invalid(
            "request",
            "a property read has no command parameters",
        ));
    }
    optional_pointer("start", request.start.as_deref())?;
    optional_pointer("end", request.end.as_deref())?;
    optional_pointer("cursor", request.cursor.as_deref())?;
    optional_pointer("limit", request.limit.as_deref())?;
    Ok(())
}

fn check_text(field: &'static str, value: &str) -> Result<(), A2aLabError> {
    if value.is_empty() {
        return Err(A2aLabError::invalid(field, "must not be empty"));
    }
    Ok(())
}

fn optional_pointer(field: &'static str, value: Option<&str>) -> Result<(), A2aLabError> {
    if let Some(value) = value {
        pointer(field, value)?;
    }
    Ok(())
}

pub(crate) fn pointer(field: &'static str, value: &str) -> Result<(), A2aLabError> {
    if !value.starts_with('/') || value.split('/').skip(1).any(str::is_empty) {
        return Err(A2aLabError::invalid(
            field,
            "must be a JSON pointer to one field",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SilaProviderConfig;

    #[test]
    fn rejects_duplicate_ids_and_plaintext_certificates() {
        let mut config = sample();
        config.plaintext = true;
        config.ca_pem = Some("pem".to_owned());
        assert!(config.check().is_err());
        config.ca_pem = None;
        config.bindings.push(config.bindings[0].clone());
        assert!(config.check().is_err());
    }

    #[test]
    fn rejects_a_selector_that_is_not_a_pointer() {
        let mut config = sample();
        let super::SilaBinding::Logs(binding) = &mut config.bindings[1] else {
            panic!("log binding");
        };
        binding.records = "Records".to_owned();
        assert!(config.check().is_err());
    }

    fn sample() -> SilaProviderConfig {
        serde_json::from_str(
            r#"{
              "host": "127.0.0.1",
              "port": 50052,
              "server_uuid": "11111111-1111-1111-1111-111111111111",
              "plaintext": true,
              "bindings": [
                {"role": "task", "id": "name", "name": "Name", "description": "Read the name", "member": {"feature": "org.silastandard/core/SiLAService/v1", "kind": "property", "identifier": "ServerName"}},
                {"role": "logs", "id": "events", "name": "Events", "description": "Events", "member": {"feature": "com.a3analytics/lab/LabOperations/v1", "kind": "command", "identifier": "QueryLogs"}, "records": "/Records", "timestamp": "/Timestamp", "level": "/Level", "message": "/Message"}
              ]
            }"#,
        )
        .unwrap()
    }
}
