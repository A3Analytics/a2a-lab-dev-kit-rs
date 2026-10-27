use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabService, A2aServer, MAX_VIDEO_CLIP_DURATION_US,
    McpLab, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos,
    TckMalformedVideoRequests, TimeRange, UtcTimestamp, VideoSourceId, bind_local,
};
use serde_json::json;

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn service() -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::new())
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
async fn public_tck_seam_preserves_and_rejects_three_malformed_video_requests() {
    let source_id = VideoSourceId::new("camera").unwrap();
    let start = timestamp("2024-01-01T00:00:00Z");
    let range = TimeRange::new(start, timestamp("2024-01-01T00:00:02Z")).unwrap();
    let list_sources = TckMalformedVideoRequests::list_video_sources_zero_limit();
    let reversed =
        TckMalformedVideoRequests::get_video_clip_reversed_range(source_id.clone(), range);
    let over_limit = TckMalformedVideoRequests::get_video_clip_over_limit(source_id, start);
    let over_end = start.saturating_add_micros(
        i64::try_from(MAX_VIDEO_CLIP_DURATION_US.saturating_add(1)).unwrap_or(i64::MAX),
    );

    assert_eq!(
        serde_json::to_value(&list_sources).unwrap(),
        json!({"page": {"limit": 0}})
    );
    assert_eq!(
        serde_json::to_value(&reversed).unwrap(),
        json!({
            "source_id": "camera",
            "range": {"start": "2024-01-01T00:00:02Z", "end": "2024-01-01T00:00:00Z"}
        })
    );
    assert_eq!(
        serde_json::to_value(&over_limit).unwrap(),
        json!({
            "source_id": "camera",
            "range": {"start": start.to_rfc3339(), "end": over_end.to_rfc3339()}
        })
    );

    let lab = service();
    let a2a = A2aClient::new(&serve_a2a(lab.clone()).await).unwrap();
    let mcp = McpLab::connect(&serve_mcp(lab).await).await.unwrap();

    let a2a_errors = [
        a2a.list_video_sources(list_sources.clone())
            .await
            .unwrap_err(),
        a2a.get_video_clip(reversed.clone()).await.unwrap_err(),
        a2a.get_video_clip(over_limit.clone()).await.unwrap_err(),
    ];
    let mcp_errors = [
        mcp.execute(A2aLabCommand::ListVideoSources(list_sources))
            .await
            .unwrap_err(),
        mcp.execute(A2aLabCommand::GetVideoClip(reversed))
            .await
            .unwrap_err(),
        mcp.execute(A2aLabCommand::GetVideoClip(over_limit))
            .await
            .unwrap_err(),
    ];

    for error in a2a_errors.into_iter().chain(mcp_errors) {
        assert_eq!(error.code(), "invalid", "{error}");
    }
}
