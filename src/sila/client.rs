//! SiLA 2 discovery, TLS identity, command status, and binary transfer.

use crate::error::SdkError;
use crate::sila::proto;
use crate::workflows::RunState;

/// Maximum SiLA binary-transfer chunk size.
pub const MAX_CHUNK: usize = 2 * 1024 * 1024;

/// Explicit SiLA server endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilaEndpoint {
    /// Host name.
    pub host: String,
    /// gRPC port.
    pub port: u16,
    /// Server UUID from discovery or configuration.
    pub server_uuid: String,
}

/// Maps a SiLA execution status code onto a lab run state.
pub fn execution_state(status: u32) -> Result<RunState, SdkError> {
    match status {
        0 => Ok(RunState::Submitted),
        1 => Ok(RunState::Working),
        2 => Ok(RunState::Completed),
        3 => Ok(RunState::Failed),
        _ => Err(SdkError::protocol(format!(
            "unknown SiLA execution status {status}"
        ))),
    }
}

/// Splits a binary upload into SiLA chunks of at most [`MAX_CHUNK`] bytes.
#[must_use]
pub fn chunk_binary(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes.chunks(MAX_CHUNK).collect()
}

/// Parses a DNS-SD service into a SiLA endpoint.
pub fn parse_discovery(
    service_type: &str,
    host: &str,
    port: u16,
    server_uuid: &str,
) -> Result<SilaEndpoint, SdkError> {
    if service_type != "_sila._tcp.local." {
        return Err(SdkError::protocol("service is not a SiLA 2 server"));
    }
    if host.is_empty() || port == 0 || server_uuid.is_empty() {
        return Err(SdkError::invalid(
            "sila_endpoint",
            "host, port, and UUID are required",
        ));
    }
    Ok(SilaEndpoint {
        host: host.to_owned(),
        port,
        server_uuid: server_uuid.to_owned(),
    })
}

/// Checks the SiLA certificate common name and server UUID.
pub fn certificate_accepted(
    common_name: &str,
    advertised_uuid: &str,
    certificate_uuid: &str,
) -> Result<(), SdkError> {
    if common_name != "SiLA2" {
        return Err(SdkError::protocol(
            "SiLA certificate common name must be SiLA2",
        ));
    }
    if advertised_uuid != certificate_uuid {
        return Err(SdkError::protocol(
            "SiLA server UUID does not match the certificate",
        ));
    }
    Ok(())
}

/// Starts a workflow through the generated SiLA client and returns its execution UUID.
pub async fn start_workflow(
    client: &mut proto::lab_automation_client::LabAutomationClient<tonic::transport::Channel>,
    workflow_id: &str,
    input_json: &str,
    run_id: &str,
) -> Result<(String, RunState), SdkError> {
    let confirmation = client
        .start_workflow(proto::StartWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
            input_json: input_json.to_owned(),
            run_id: run_id.to_owned(),
        })
        .await
        .map_err(|error| SdkError::transport(error.to_string()))?
        .into_inner();
    if confirmation.command_execution_uuid.is_empty() {
        return Err(SdkError::protocol(
            "SiLA confirmation is missing an execution UUID",
        ));
    }
    let mut info = client
        .start_workflow_info(proto::CommandExecution {
            command_execution_uuid: confirmation.command_execution_uuid.clone(),
        })
        .await
        .map_err(|error| SdkError::transport(error.to_string()))?
        .into_inner();
    let status = info
        .message()
        .await
        .map_err(|error| SdkError::transport(error.to_string()))?
        .ok_or_else(|| SdkError::protocol("SiLA execution stream ended before a status"))?;
    Ok((
        confirmation.command_execution_uuid,
        execution_state(status.command_status)?,
    ))
}
