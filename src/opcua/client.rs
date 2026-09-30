//! Secure OPC UA reads, history, events, and method calls.

use std::path::PathBuf;
use std::str::FromStr;

use opcua_client::{ClientBuilder, HistoryReadAction, IdentityToken, Session};
use opcua_types::{
    DateTime, HistoryData, HistoryReadResult, HistoryReadValueId, NodeId, ReadRawModifiedDetails,
    StatusCode, TimestampsToReturn, Variant,
};

use crate::catalog::Endpoint;
use crate::error::SdkError;
use crate::metrics::MetricPoint;
use crate::time::{TimeRange, UtcTimestamp};

/// Returns the namespace index for `namespace_uri`.
pub fn namespace_index(namespaces: &[String], namespace_uri: &str) -> Result<u16, SdkError> {
    namespaces
        .iter()
        .position(|namespace| namespace == namespace_uri)
        .map(|index| u16::try_from(index).unwrap_or(u16::MAX))
        .ok_or_else(|| SdkError::not_found("opcua namespace", namespace_uri))
}

/// Drops samples that fall on the exclusive end of a lab time range.
#[must_use]
pub fn filter_half_open(points: Vec<MetricPoint>, range: TimeRange) -> Vec<MetricPoint> {
    points
        .into_iter()
        .filter(|point| range.contains(point.timestamp))
        .collect()
}

/// OPC UA client that refuses unsecured sessions and automatic trust.
pub struct OpcUaClient {
    pki_dir: PathBuf,
    username: String,
    password: String,
}

impl OpcUaClient {
    /// Creates a client whose certificates live in `pki_dir`.
    ///
    /// Automatic server trust is disabled. The username and password are sent only
    /// over the configured signed session.
    #[must_use]
    pub fn new(
        pki_dir: impl Into<PathBuf>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            pki_dir: pki_dir.into(),
            username: username.into(),
            password: password.into(),
        }
    }

    /// Reads raw history and applies the half-open lab range.
    pub async fn read_history(
        &self,
        endpoint: &Endpoint,
        range: TimeRange,
    ) -> Result<Vec<MetricPoint>, SdkError> {
        let (session, node) = self.session(endpoint).await?;
        let details = ReadRawModifiedDetails {
            is_read_modified: false,
            start_time: encode_time(range.start())?,
            end_time: encode_time(range.end())?,
            num_values_per_node: 0,
            return_bounds: false,
        };
        let nodes = [HistoryReadValueId {
            node_id: node,
            index_range: opcua_types::NumericRange::None,
            data_encoding: opcua_types::QualifiedName::null(),
            continuation_point: opcua_types::ByteString::null(),
        }];
        let results = session
            .history_read(
                HistoryReadAction::ReadRawModifiedDetails(details),
                TimestampsToReturn::Source,
                false,
                &nodes,
            )
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        let Some(result) = results.into_iter().next() else {
            return Err(SdkError::unavailable("OPC UA history returned no result"));
        };
        if result.status_code.is_bad() {
            return Err(history_error(result.status_code));
        }
        Ok(filter_half_open(
            points_from_history(&result, &range)?,
            range,
        ))
    }

    async fn session(
        &self,
        endpoint: &Endpoint,
    ) -> Result<(std::sync::Arc<Session>, NodeId), SdkError> {
        let Endpoint::OpcUa {
            url,
            security_policy,
            node_id,
            namespace_uri,
            ..
        } = endpoint
        else {
            return Err(SdkError::protocol("expected an OPC UA endpoint"));
        };
        if security_policy.contains("None") {
            return Err(SdkError::invalid(
                "security_policy",
                "unsecured OPC UA sessions are rejected",
            ));
        }
        let mut client = ClientBuilder::new()
            .application_name("a2a-lab")
            .application_uri("urn:a2a-lab:client")
            .product_uri("urn:a2a-lab:sdk")
            .pki_dir(self.pki_dir.clone())
            .trust_server_certs(false)
            .verify_server_certs(true)
            .create_sample_keypair(false)
            .client()
            .map_err(|errors| SdkError::invalid("opcua", errors.join(", ")))?;
        let (session, event_loop) = client
            .connect_to_matching_endpoint(
                url.as_str(),
                IdentityToken::new_user_name(self.username.clone(), self.password.clone()),
            )
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        tokio::spawn(async move {
            let _ = event_loop.run().await;
        });
        let namespaces = session
            .read_namespace_array()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        let index = namespaces
            .get_index(namespace_uri)
            .ok_or_else(|| SdkError::not_found("opcua namespace", namespace_uri.clone()))?;
        let node = NodeId::from_str(&format!("ns={index};{node_id}"))
            .map_err(|error| SdkError::protocol(error.to_string()))?;
        Ok((session, node))
    }
}

fn encode_time(timestamp: UtcTimestamp) -> Result<DateTime, SdkError> {
    DateTime::from_str(&timestamp.to_rfc3339())
        .map_err(|error| SdkError::protocol(error.to_string()))
}

fn history_error(status: StatusCode) -> SdkError {
    if status == StatusCode::BadHistoryOperationUnsupported {
        SdkError::unavailable("OPC UA server does not support history reads")
    } else {
        SdkError::protocol(status.to_string())
    }
}

fn points_from_history(
    result: &HistoryReadResult,
    range: &TimeRange,
) -> Result<Vec<MetricPoint>, SdkError> {
    let HistoryData { data_values, .. } = result
        .history_data
        .inner_as::<HistoryData>()
        .ok_or_else(|| SdkError::protocol("OPC UA history payload was not HistoryData"))?;
    let mut points = Vec::new();
    for value in data_values.clone().unwrap_or_default() {
        let Some(source) = value.source_timestamp else {
            continue;
        };
        let Some(variant) = value.value else {
            continue;
        };
        let Variant::Double(sample) = variant else {
            return Err(SdkError::invalid(
                "value",
                "OPC UA history value is not a double",
            ));
        };
        let timestamp = UtcTimestamp::parse(&source.to_string())?;
        if range.contains(timestamp) {
            points.push(MetricPoint::new(timestamp, sample)?);
        }
    }
    Ok(points)
}
