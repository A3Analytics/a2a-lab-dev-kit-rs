#[cfg(feature = "sila2")]
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn legacy_clients_keep_success_malformed_and_the_sixty_second_boundary() {
    let lab = service(catalog().await);
    let endpoints = open(Arc::clone(&lab)).await;
    compare_catalog(&endpoints).await;
    compare_malformed(&endpoints).await;
    let request =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"))
            .unwrap();
    let expected = vec![
        "2024-01-01T00:00:00Z".to_owned(),
        "2024-01-01T00:00:01Z".to_owned(),
    ];
    let a2a_clip = endpoints.a2a.get_video_clip(request.clone()).await.unwrap();
    let a2a_stamps = clip_stamps(endpoints.a2a.open_video_clip(a2a_clip.id()).await.unwrap()).await;
    let mcp_outcome = endpoints
        .mcp
        .execute(A2aLabCommand::GetVideoClip(request))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_clip) = mcp_outcome.task.result else {
        diverge("half-open clip", "MCP", "unexpected result");
    };
    let mcp_stamps = clip_stamps(endpoints.mcp.open_video_clip(mcp_clip.id()).await.unwrap()).await;
    if a2a_stamps != expected {
        diverge("half-open clip", "A2A", &format!("{a2a_stamps:?}"));
    }
    if mcp_stamps != expected {
        diverge("half-open clip", "MCP", &format!("{mcp_stamps:?}"));
    }
    let mut sila = SilaVideos::start(lab).await;
    let (path, bytes) = sila
        .clip_bytes(
            "cam",
            sila_utc(2024, 1, 1, 0, 0, 0),
            sila_utc(2024, 1, 1, 0, 0, 2),
        )
        .await;
    assert_relative_media_path(&path);
    let sila_stamps = stamps_in(&bytes);
    if sila_stamps != expected {
        diverge("half-open clip", "SiLA", &format!("{sila_stamps:?}"));
    }
    assert_sila_catalog_and_malformed(&mut sila).await;

    let span = share(span_catalog().await);
    let base = listen_a2a(Arc::clone(&span)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&span)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let request = sixty_second_request();
    let expected = span_stamps();
    let a2a_clip = a2a.get_video_clip(request.clone()).await.unwrap();
    assert_no_media_bytes(&serde_json::to_value(&a2a_clip).unwrap());
    let a2a_stamps = clip_stamps(a2a.open_video_clip(a2a_clip.id()).await.unwrap()).await;
    let mcp_clip = mcp
        .execute(A2aLabCommand::GetVideoClip(request))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_clip) = mcp_clip.task.result else {
        diverge("60-second boundary", "MCP", "unexpected result");
    };
    assert_no_media_bytes(&serde_json::to_value(&mcp_clip).unwrap());
    let mcp_stamps = clip_stamps(mcp.open_video_clip(mcp_clip.id()).await.unwrap()).await;
    if a2a_stamps != expected {
        diverge("60-second boundary", "A2A", &format!("{a2a_stamps:?}"));
    }
    if mcp_stamps != expected {
        diverge("60-second boundary", "MCP", &format!("{mcp_stamps:?}"));
    }
    let over = TckMalformedVideoRequests::get_video_clip_over_limit(
        source_id("span"),
        timestamp("2024-01-01T00:00:00Z"),
    );
    expect_code(
        "60-second boundary",
        "A2A",
        a2a.get_video_clip(over.clone()).await.unwrap_err().code(),
        "invalid",
    );
    expect_code(
        "60-second boundary",
        "MCP",
        mcp.execute(A2aLabCommand::GetVideoClip(over))
            .await
            .unwrap_err()
            .code(),
        "invalid",
    );
    let mut span_sila = SilaVideos::start(span).await;
    let (path, bytes) = span_sila
        .clip_bytes(
            "span",
            sila_utc(2024, 1, 1, 0, 0, 0),
            sila_utc(2024, 1, 1, 0, 1, 0),
        )
        .await;
    assert_relative_media_path(&path);
    let sila_stamps = stamps_in(&bytes);
    if sila_stamps != expected {
        diverge("60-second boundary", "SiLA", &format!("{sila_stamps:?}"));
    }
    let over = span_sila
        .client
        .get_video_clip(clip_params(
            "span",
            sila_utc(2024, 1, 1, 0, 0, 0),
            sila_ms(1, 0, 1),
        ))
        .await
        .unwrap_err();
    assert_validation(&over);
}

async fn span_catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos.insert_source(source("span")).await.unwrap();
    let start = timestamp("2024-01-01T00:00:00Z");
    for second in 0_i32..=60 {
        let at = start.saturating_add_micros(i64::from(second) * 1_000_000);
        videos
            .insert_frame(source_id("span"), at, JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
}

fn sixty_second_request() -> GetVideoClipRequest {
    let start = timestamp("2024-01-01T00:00:00Z");
    let end = start.saturating_add_micros(i64::try_from(MAX_VIDEO_CLIP_DURATION_US).unwrap());
    GetVideoClipRequest::new(source_id("span"), TimeRange::new(start, end).unwrap()).unwrap()
}

fn span_stamps() -> Vec<String> {
    let start = timestamp("2024-01-01T00:00:00Z");
    (0_i32..60)
        .map(|second| {
            start
                .saturating_add_micros(i64::from(second) * 1_000_000)
                .to_rfc3339()
        })
        .collect()
}

#[cfg(feature = "sila2")]
async fn assert_sila_catalog_and_malformed(sila: &mut SilaVideos) {
    use a2a_lab_dev_kit::sila::api::{
        Boolean, Integer, ListVideoSourcesParameters, SilaString, VideoPageRequest, VideoPageStruct,
    };

    let listed = sila
        .client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(VideoPageRequest {
                page_request: Some(VideoPageStruct {
                    has_cursor: Some(Boolean { value: false }),
                    cursor: Some(SilaString {
                        value: String::new(),
                    }),
                    limit: Some(Integer { value: 10 }),
                }),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let ids: Vec<_> = listed
        .sources
        .iter()
        .map(|source| {
            source
                .video_source
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .value
                .clone()
        })
        .collect();
    if ids != ["cam", "empty", "zeta"] {
        diverge("list video sources", "SiLA", &format!("{ids:?}"));
    }
    let frame = sila
        .client
        .get_video_frame_at(frame_params("cam", sila_utc(2024, 1, 1, 0, 0, 0)))
        .await
        .unwrap()
        .into_inner();
    if frame.delta_us.unwrap().value != 0 {
        diverge("frame at exact stamp", "SiLA", &format!("{frame:?}"));
    }
    let invalid = sila
        .client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(VideoPageRequest {
                page_request: Some(VideoPageStruct {
                    has_cursor: Some(Boolean { value: false }),
                    cursor: Some(SilaString {
                        value: String::new(),
                    }),
                    limit: Some(Integer { value: 0 }),
                }),
            }),
        })
        .await
        .unwrap_err();
    assert_validation(&invalid);
    let reversed = sila
        .client
        .get_video_clip(clip_params(
            "cam",
            sila_utc(2024, 1, 1, 0, 0, 2),
            sila_utc(2024, 1, 1, 0, 0, 0),
        ))
        .await
        .unwrap_err();
    assert_validation(&reversed);
    let over = sila
        .client
        .get_video_clip(clip_params(
            "cam",
            sila_utc(2024, 1, 1, 0, 0, 0),
            sila_ms(1, 0, 1),
        ))
        .await
        .unwrap_err();
    assert_validation(&over);
}

#[cfg(feature = "sila2")]
fn assert_validation(status: &tonic::Status) {
    let text = decoded(status);
    for name in [
        "NoRecording",
        "RecordingGap",
        "ClipExpired",
        "LiveUnavailable",
        "VideoSourceNotFound",
        "VideoFrameNotFound",
        "VideoClipNotFound",
        "ProviderUnavailable",
    ] {
        if text.contains(&format!("DefinedExecutionError/{name}")) {
            diverge("malformed", "SiLA", &text);
        }
    }
}
