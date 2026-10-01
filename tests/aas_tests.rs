#![cfg(feature = "aas")]

use std::sync::Arc;

use a2a_lab_dev_kit::aas::{AasClient, StaticToken};
use a2a_lab_dev_kit::{
    AssetCatalogProvider, AssetKey, BindingRole, ListAssetsRequest, PageRequest, ProtocolKind,
};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

const PROFILE: &str = "https://admin-shell.io/aas/API/3.2/AssetAdministrationShellRepositoryServiceSpecification/SSP-001";

#[tokio::test]
async fn reads_bindings_and_rejects_bad_profiles_and_tokens() {
    let app = Router::new()
        .route("/description", get(description))
        .route("/shells", get(shells))
        .route("/submodels/{id}", get(submodel))
        .with_state(Arc::new(true));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = AasClient::new(
        &format!("http://{address}"),
        StaticToken::new(Some("token".to_owned())),
    )
    .unwrap();
    let assets = client
        .list_assets(ListAssetsRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    let asset = &assets.items()[0];
    assert_eq!(asset.key().as_str(), "https://example.com/aas/pump");
    let binding = &asset.bindings()[0];
    assert_eq!(binding.lab_id(), "temperature");
    assert_eq!(binding.role(), BindingRole::Metric);
    assert_eq!(binding.endpoint().protocol(), ProtocolKind::OpcUa);
    assert_eq!(
        client
            .get_asset(&AssetKey::new("missing").unwrap())
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );

    let bad = Router::new().route(
        "/description",
        get(|| async { Json(json!({"profiles": ["v2"]})) }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, bad).await.unwrap();
    });
    let client = AasClient::new(&format!("http://{address}"), StaticToken::new(None)).unwrap();
    assert_eq!(
        client
            .list_assets(ListAssetsRequest {
                page: PageRequest::default(),
            })
            .await
            .unwrap_err()
            .code(),
        "protocol"
    );

    let denied = Router::new().route(
        "/description",
        get(|| async { (StatusCode::UNAUTHORIZED, Json(json!({}))) }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, denied).await.unwrap();
    });
    let client = AasClient::new(
        &format!("http://{address}"),
        StaticToken::new(Some("expired".to_owned())),
    )
    .unwrap();
    assert_eq!(
        client
            .list_assets(ListAssetsRequest {
                page: PageRequest::default(),
            })
            .await
            .unwrap_err()
            .code(),
        "protocol"
    );
}

async fn description(headers: HeaderMap) -> Json<Value> {
    assert_eq!(headers.get("authorization").unwrap(), "Bearer token");
    Json(json!({ "profiles": [PROFILE] }))
}

async fn shells() -> Json<Value> {
    Json(json!({
        "result": [{
            "id": "https://example.com/aas/pump",
            "assetInformation": { "globalAssetId": "https://example.com/asset/pump" },
            "submodels": [{ "keys": [{ "type": "Submodel", "value": "https://example.com/sm/bindings" }] }]
        }]
    }))
}

async fn submodel(Path(id): Path<String>) -> Json<Value> {
    assert!(!id.is_empty());
    Json(json!({
        "id": "https://example.com/sm/bindings",
        "semanticId": { "keys": [{ "type": "GlobalReference", "value": "https://a2a-lab.example/LabBindings/1/0" }] },
        "submodelElements": [{
            "semanticId": { "keys": [{ "value": "https://example.com/semantic/temperature" }] },
            "value": [
                { "idShort": "labId", "value": "temperature" },
                { "idShort": "role", "value": "metric" },
                { "idShort": "protocol", "value": "opc_ua" },
                { "idShort": "url", "value": "opc.tcp://lab.example:4840" },
                { "idShort": "securityPolicy", "value": "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256" },
                { "idShort": "securityMode", "value": "sign_and_encrypt" },
                { "idShort": "identity", "value": "certificate" },
                { "idShort": "nodeId", "value": "s=Temperature" },
                { "idShort": "namespaceUri", "value": "urn:lab:equipment" }
            ]
        }]
    }))
}
