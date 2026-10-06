use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aServer, Asset, AssetKey, Binding, BindingRole, Endpoint, IndustrialLabBuilder,
    JsonObject, LabService, ListMetricsRequest, MemoryCatalog, MetricPoint, OpcUaIdentityKind,
    PageRequest, ScriptedLive, SecurityMode, SemanticId, SemanticKind, StartTaskRequest, TaskId,
    TimeRange, UtcTimestamp, bind_local,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn opcua() -> Endpoint {
    Endpoint::OpcUa {
        url: "opc.tcp://lab.example:4840".to_owned(),
        security_policy: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256".to_owned(),
        security_mode: SecurityMode::SignAndEncrypt,
        identity: OpcUaIdentityKind::Certificate,
        node_id: "s=Temperature".to_owned(),
        namespace_uri: "urn:lab:equipment".to_owned(),
        browse_path: String::new(),
    }
}

fn task_endpoint() -> Endpoint {
    Endpoint::OpcUa {
        url: "opc.tcp://lab.example:4840".to_owned(),
        security_policy: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256".to_owned(),
        security_mode: SecurityMode::SignAndEncrypt,
        identity: OpcUaIdentityKind::Certificate,
        node_id: "s=Build".to_owned(),
        namespace_uri: "urn:lab:equipment".to_owned(),
        browse_path: String::new(),
    }
}

#[tokio::test]
async fn a2a_uses_aas_bindings_for_opcua_metrics_and_tasks() {
    let asset = AssetKey::new("https://example.com/aas/pump").unwrap();
    let catalog = MemoryCatalog::new();
    catalog
        .insert(Asset::new(
            asset.clone(),
            None,
            vec![
                Binding::new(
                    "temperature",
                    asset.clone(),
                    SemanticId::new(
                        SemanticKind::Iri,
                        "https://example.com/semantic/temperature",
                    )
                    .unwrap(),
                    BindingRole::Metric,
                    opcua(),
                )
                .unwrap(),
                Binding::new(
                    "build",
                    asset,
                    SemanticId::new(SemanticKind::Iri, "https://example.com/semantic/build")
                        .unwrap(),
                    BindingRole::Task,
                    task_endpoint(),
                )
                .unwrap(),
            ],
        ))
        .await;
    let live = ScriptedLive::new();
    live.insert_metric(MetricPoint::new(timestamp("2024-01-01T00:30:00Z"), 21.5).unwrap())
        .await;
    let builder = IndustrialLabBuilder::new(catalog, live);
    let service = LabService::new(builder.logs(), builder.metrics(), builder.tasks()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    let client = A2aClient::new(&format!("http://{address}")).unwrap();
    let metrics = client
        .list_metrics(ListMetricsRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        metrics.items()[0].semantic_id.as_deref(),
        Some("https://example.com/semantic/temperature")
    );
    let points = client
        .query_metric(a2a_lab_dev_kit::QueryMetricRequest {
            metric_id: a2a_lab_dev_kit::MetricId::new("temperature").unwrap(),
            range: TimeRange::new(
                timestamp("2024-01-01T00:00:00Z"),
                timestamp("2024-01-01T01:00:00Z"),
            )
            .unwrap(),
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert!((points.items()[0].value - 21.5).abs() < f64::EPSILON);
    let started = client
        .start_task(
            StartTaskRequest::new(
                TaskId::new("build").unwrap(),
                JsonObject::parse(r#"{"profile":"standard"}"#).unwrap(),
            )
            .immediate(),
        )
        .await
        .unwrap();
    assert_eq!(started.state, a2a_lab_dev_kit::TaskState::Submitted);
    let _ = Arc::new(client);
}
