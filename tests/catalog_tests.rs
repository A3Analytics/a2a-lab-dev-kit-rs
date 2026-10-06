use a2a_lab_dev_kit::{
    Asset, AssetCatalogProvider, AssetKey, Binding, BindingRole, Endpoint, ListAssetsRequest,
    ListBindingsRequest, MemoryCatalog, MetricDescriptor, MetricId, OpcUaIdentityKind, PageRequest,
    SecurityMode, SemanticId, SemanticKind,
};

fn page_request(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn semantic() -> SemanticId {
    SemanticId::new(
        SemanticKind::Iri,
        "https://example.com/semantic/temperature",
    )
    .unwrap()
}

fn endpoint() -> Endpoint {
    Endpoint::OpcUa {
        url: "opc.tcp://lab.example:4840".to_owned(),
        security_policy: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256".to_owned(),
        security_mode: SecurityMode::SignAndEncrypt,
        identity: OpcUaIdentityKind::Certificate,
        node_id: "s=Temperature".to_owned(),
        namespace_uri: "urn:lab:equipment".to_owned(),
        browse_path: "/Objects/Pump/Temperature".to_owned(),
    }
}

#[test]
fn accepts_catalog_identifiers_that_lab_tokens_reject() {
    assert!(AssetKey::new("https://example.com/aas/pump-1").is_ok());
    assert!(SemanticId::new(SemanticKind::Irdi, "0173-1#01-AAR672#002").is_ok());
    assert!(SemanticId::new(SemanticKind::Iri, "not a url").is_err());
    assert!(SemanticId::new(SemanticKind::Irdi, "0173-1").is_err());
    assert!(SemanticId::new(SemanticKind::Custom, "").is_err());
}

#[test]
fn rejects_unsecured_opcua_and_invalid_lab_ids() {
    let mut bad = endpoint();
    let Endpoint::OpcUa {
        security_policy, ..
    } = &mut bad;
    *security_policy = "http://opcfoundation.org/UA/SecurityPolicy#None".to_owned();
    assert!(bad.check().is_err());
    let error = Binding::new(
        "bad/id",
        AssetKey::new("asset").unwrap(),
        semantic(),
        BindingRole::Metric,
        endpoint(),
    );
    assert_eq!(error.unwrap_err().code(), "invalid");
}

#[tokio::test]
async fn pages_assets_and_reports_missing_keys() {
    let catalog = MemoryCatalog::new();
    let asset = Asset::new(
        AssetKey::new("https://example.com/aas/pump").unwrap(),
        Some("https://example.com/asset/pump".to_owned()),
        vec![
            Binding::new(
                "temperature",
                AssetKey::new("https://example.com/aas/pump").unwrap(),
                semantic(),
                BindingRole::Metric,
                endpoint(),
            )
            .unwrap(),
        ],
    );
    catalog.insert(asset).await;
    let page = catalog
        .list_assets(ListAssetsRequest {
            page: page_request(10),
        })
        .await
        .unwrap();
    assert_eq!(page.items().len(), 1);
    let bindings = catalog
        .list_bindings(ListBindingsRequest {
            page: page_request(10),
        })
        .await
        .unwrap();
    assert_eq!(bindings.items()[0].lab_id(), "temperature");
    let missing = catalog
        .get_asset(&AssetKey::new("missing").unwrap())
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}

#[test]
fn descriptor_metadata_round_trips() {
    let metric = MetricDescriptor {
        id: MetricId::new("temperature").unwrap(),
        name: "Temperature".to_owned(),
        description: "Process temperature".to_owned(),
        unit: "C".to_owned(),
        asset_id: Some("https://example.com/aas/pump".to_owned()),
        semantic_id: Some("https://example.com/semantic/temperature".to_owned()),
    };
    let encoded = serde_json::to_string(&metric).unwrap();
    let decoded: MetricDescriptor = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.semantic_id, metric.semantic_id);
    let legacy = r#"{"id":"temperature","name":"Temperature","description":"Process temperature","unit":"C"}"#;
    let decoded: MetricDescriptor = serde_json::from_str(legacy).unwrap();
    assert!(decoded.asset_id.is_none());
}
