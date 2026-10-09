use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabService, A2aServer, ImageSourceId, McpLab,
    McpServer, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, TckMalformedImageRequests,
    bind_local,
};
use serde_json::json;

fn service() -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(MemoryImages::new())
        .share()
}

async fn serve_a2a(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&lab).listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn serve_mcp(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(listener).await.unwrap();
    });
    format!("http://{address}/mcp")
}

#[tokio::test]
async fn public_tck_seam_preserves_and_rejects_three_malformed_image_requests() {
    let source_id = ImageSourceId::new("camera").unwrap();
    let list_sources = TckMalformedImageRequests::list_image_sources_zero_limit();
    let list_images = TckMalformedImageRequests::list_images_zero_limit(source_id);
    let search = TckMalformedImageRequests::search_images_without_criterion();

    assert_eq!(
        serde_json::to_value(&list_sources).unwrap(),
        json!({"page": {"limit": 0}})
    );
    assert_eq!(
        serde_json::to_value(&list_images).unwrap(),
        json!({"source_id": "camera", "page": {"limit": 0}})
    );
    assert_eq!(
        serde_json::to_value(&search).unwrap(),
        json!({"page": {"limit": 100}})
    );

    let lab = service();
    let a2a = A2aClient::new(&serve_a2a(lab.clone()).await).unwrap();
    let mcp = McpLab::connect(&serve_mcp(lab).await).await.unwrap();

    let a2a_errors = [
        a2a.list_image_sources(list_sources.clone())
            .await
            .unwrap_err(),
        a2a.list_images(list_images.clone()).await.unwrap_err(),
        a2a.search_images(search.clone()).await.unwrap_err(),
    ];
    let mcp_errors = [
        mcp.execute(A2aLabCommand::ListImageSources(list_sources))
            .await
            .unwrap_err(),
        mcp.execute(A2aLabCommand::ListImages(list_images))
            .await
            .unwrap_err(),
        mcp.execute(A2aLabCommand::SearchImages(search))
            .await
            .unwrap_err(),
    ];

    for error in a2a_errors.into_iter().chain(mcp_errors) {
        assert_eq!(error.code(), "invalid", "{error}");
    }
}
