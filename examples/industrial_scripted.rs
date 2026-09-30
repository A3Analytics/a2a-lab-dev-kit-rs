//! Catalog bindings served by `ScriptedLive`.
//!
//! The endpoint stored on the binding is catalog data. Samples come from the
//! scripted live source.

use a2a_lab_sdk::{
    Asset, AssetKey, Binding, BindingRole, Endpoint, IndustrialLabBuilder, MemoryCatalog, MetricId,
    MetricPoint, MetricProvider, OpcUaIdentityKind, PageRequest, QueryMetricRequest, ScriptedLive,
    SdkError, SecurityMode, SemanticId, SemanticKind, TimeRange, UtcTimestamp,
};

#[tokio::main]
async fn main() -> Result<(), SdkError> {
    let asset = AssetKey::new("https://example.com/aas/pump")?;
    let catalog = MemoryCatalog::new();
    catalog
        .insert(Asset::new(
            asset.clone(),
            None,
            vec![Binding::new(
                "temperature",
                asset,
                SemanticId::new(
                    SemanticKind::Iri,
                    "https://example.com/semantic/temperature",
                )?,
                BindingRole::Metric,
                Endpoint::OpcUa {
                    url: "opc.tcp://lab.example:4840".to_owned(),
                    security_policy: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256"
                        .to_owned(),
                    security_mode: SecurityMode::SignAndEncrypt,
                    identity: OpcUaIdentityKind::Certificate,
                    node_id: "s=Temperature".to_owned(),
                    namespace_uri: "urn:lab:equipment".to_owned(),
                    browse_path: String::new(),
                },
            )?],
        ))
        .await;

    let live = ScriptedLive::new();
    live.insert_metric(MetricPoint::new(
        UtcTimestamp::parse("2024-01-01T00:30:00Z")?,
        21.5,
    )?)
    .await;

    let metrics = IndustrialLabBuilder::new(catalog, live).metrics();
    let samples = metrics
        .query(QueryMetricRequest {
            metric_id: MetricId::new("temperature")?,
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
                UtcTimestamp::parse("2024-01-01T01:00:00Z")?,
            )?,
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!("scripted temperature sample {}", samples.items()[0].value);
    Ok(())
}
