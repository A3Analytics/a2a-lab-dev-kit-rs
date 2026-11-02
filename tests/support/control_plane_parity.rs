use std::time::Duration;

use serde_json::Value;

const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40, 0x01];

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn parity_fail(case: &str, a2a: &str, mcp: &str) -> ! {
    panic!("case {case}\nA2A: {a2a}\nMCP: {mcp}");
}

async fn open_auth(lab: Arc<dyn A2aLabApi>) -> (Endpoints, String, String) {
    let a2a_lab = Arc::clone(&lab);
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    let schemes = [(
        "bearerAuth".to_owned(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "bearer".to_owned(),
            description: None,
            bearer_format: None,
        }),
    )]
    .into();
    let requirements = vec![[("bearerAuth".to_owned(), Vec::new())].into()];
    tokio::spawn(async move {
        A2aServer::new(&a2a_lab)
            .with_security(schemes, requirements)
            .listen(a2a_listener)
            .await
            .unwrap();
    });
    let mcp_lab = Arc::clone(&lab);
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    let schemes = [(
        "bearerAuth".to_owned(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "bearer".to_owned(),
            description: None,
            bearer_format: None,
        }),
    )]
    .into();
    let requirements = vec![[("bearerAuth".to_owned(), Vec::new())].into()];
    tokio::spawn(async move {
        McpServer::new(&mcp_lab)
            .with_security(schemes, requirements)
            .serve_http(mcp_listener)
            .await
            .unwrap();
    });
    let a2a_base = format!("http://{a2a_address}");
    let mcp_origin = format!("http://{mcp_address}");
    let a2a = A2aClient::new(&a2a_base)
        .unwrap()
        .with_bearer_token("lab-token");
    let mcp = McpLab::connect(&format!("{mcp_origin}/mcp")).await.unwrap();
    (
        Endpoints { a2a, mcp },
        a2a_base,
        mcp_origin,
    )
}

fn assert_no_media_bytes(value: &Value) {
    let text = value.to_string();
    assert!(value.get("data").is_none(), "{text}");
    assert!(value.get("jpeg").is_none(), "{text}");
    assert!(value.get("frames").is_none(), "{text}");
    assert!(!text.contains("/9j/"), "{text}");
    assert!(!text.contains("data:video"), "{text}");
    assert!(!text.contains("vault.internal"), "{text}");
}

fn assert_relative_path(path: &str) {
    assert!(path.starts_with("/media/"));
    assert!(!path.contains("://"));
    assert!(!path.contains('?'));
    assert!(!path.contains('#'));
    assert!(!path.contains("lab-token"));
}

async fn timeline_json(a2a: &A2aClient, mcp: &McpLab, source: &str) -> (Value, Value) {
    let request = GetVideoTimelineRequest::new(source_id(source));
    let a2a_value = match a2a.get_video_timeline(request.clone()).await {
        Ok(timeline) => serde_json::to_value(timeline).unwrap(),
        Err(error) => parity_fail("timeline", &error.to_string(), "ok"),
    };
    let outcome = match mcp.execute(A2aLabCommand::GetVideoTimeline(request)).await {
        Ok(outcome) => outcome,
        Err(error) => parity_fail("timeline", "ok", &error.to_string()),
    };
    let A2aLabResult::GetVideoTimeline(timeline) = outcome.task.result else {
        parity_fail("timeline", "ok", "unexpected result");
    };
    (a2a_value, serde_json::to_value(timeline).unwrap())
}

async fn follow_json(a2a: &A2aClient, mcp: &McpLab, source: &str) -> (Value, Value) {
    let request = FollowVideoRequest::new(source_id(source));
    let a2a_value = match a2a.follow_video(request.clone()).await {
        Ok(follow) => serde_json::to_value(follow).unwrap(),
        Err(error) => parity_fail("follow_video", &error.to_string(), "ok"),
    };
    let outcome = match mcp.execute(A2aLabCommand::FollowVideo(request)).await {
        Ok(outcome) => outcome,
        Err(error) => parity_fail("follow_video", "ok", &error.to_string()),
    };
    let A2aLabResult::FollowVideo(follow) = outcome.task.result else {
        parity_fail("follow_video", "ok", "unexpected result");
    };
    (a2a_value, serde_json::to_value(follow).unwrap())
}

fn same_timeline_json(case: &str, a2a: &Value, mcp: &Value) {
    if a2a != mcp {
        parity_fail(case, &a2a.to_string(), &mcp.to_string());
    }
}

fn same_follow_json(case: &str, a2a: &Value, mcp: &Value) {
    assert_no_media_bytes(a2a);
    assert_no_media_bytes(mcp);
    for key in ["source_id", "codec", "media_type", "playback_8x"] {
        assert_eq!(a2a.get(key), mcp.get(key), "{case} {key}");
    }
    for path in [a2a["path"].as_str().unwrap(), mcp["path"].as_str().unwrap()] {
        assert_relative_path(path);
    }
}

fn same_clip_json(case: &str, a2a: &Value, mcp: &Value) {
    assert_no_media_bytes(a2a);
    assert_no_media_bytes(mcp);
    for key in [
        "source_id",
        "codec",
        "media_type",
        "requested_range",
        "effective_range",
        "playback_8x",
    ] {
        assert_eq!(a2a.get(key), mcp.get(key), "{case} {key}");
    }
    for path in [a2a["path"].as_str().unwrap(), mcp["path"].as_str().unwrap()] {
        assert_relative_path(path);
        assert!(path.ends_with('/'));
    }
}

async fn error_code_a2a(a2a: &A2aClient, command: A2aLabCommand) -> String {
    match command {
        A2aLabCommand::GetVideoClip(request) => a2a.get_video_clip(request).await,
        _ => unreachable!("clip errors only"),
    }
    .unwrap_err()
    .code()
    .to_owned()
}

async fn error_code_mcp(mcp: &McpLab, command: A2aLabCommand) -> String {
    mcp.execute(command).await.unwrap_err().code().to_owned()
}

async fn gapped_catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos.insert_source(source("gap")).await.unwrap();
    for at in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:10Z"] {
        videos
            .insert_frame(source_id("gap"), timestamp(at), JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
}

async fn prepare_encoded_live(videos: &MemoryVideos, codec: VideoCodec, init: &[u8], track: &str) {
    let media_type = match codec {
        VideoCodec::H264 | VideoCodec::H265 => "video/mp4",
        VideoCodec::Mjpeg => a2a_lab_dev_kit::MJPEG_MEDIA_TYPE,
    };
    videos
        .insert_source(
            VideoSource::with_capabilities(
                source_id("enc"),
                "Enc",
                "bench",
                None,
                None,
                640,
                480,
                30,
                media_type,
                vec![codec],
            )
            .unwrap(),
        )
        .await
        .unwrap();
    videos
        .insert_dvr_track(
            source_id("enc"),
            DvrTrackId::new(track).unwrap(),
            codec,
            init.to_vec(),
        )
        .await
        .unwrap();
    for (start, end) in [
        ("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        ("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
    ] {
        videos
            .insert_dvr_segment(
                source_id("enc"),
                DvrTrackId::new(track).unwrap(),
                range(start, end),
                true,
                codec,
                vec![1, 2, 3],
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&source_id("enc"), true)
        .await
        .unwrap();
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

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn video_control_plane_parity_covers_sessions_codecs_and_errors() {
    let endpoints = open(service(gapped_catalog().await)).await;
    let (a2a_timeline, mcp_timeline) =
        timeline_json(&endpoints.a2a, &endpoints.mcp, "gap").await;
    same_timeline_json("gapped timeline", &a2a_timeline, &mcp_timeline);

    let videos = catalog().await;
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let endpoints = open(service(videos)).await;
    let (a2a_follow, mcp_follow) = follow_json(&endpoints.a2a, &endpoints.mcp, "cam").await;
    same_follow_json("legacy mjpeg follow", &a2a_follow, &mcp_follow);
    assert!(a2a_follow.get("playback_8x").is_none());

    let encoded = MemoryVideos::new();
    prepare_encoded_live(&encoded, VideoCodec::H264, H264_INIT, "h264").await;
    let endpoints = open(service(encoded)).await;
    let (a2a_follow, mcp_follow) = follow_json(&endpoints.a2a, &endpoints.mcp, "enc").await;
    same_follow_json("encoded h264 follow", &a2a_follow, &mcp_follow);
    let trick = &a2a_follow["playback_8x"];
    assert!(trick.is_object());
    assert!(trick.get("fallback_path").is_some());
    assert!(a2a_follow.get("playback_2x").is_none());
    assert!(a2a_follow.get("playback_4x").is_none());

    let h265 = MemoryVideos::new();
    prepare_encoded_live(&h265, VideoCodec::H265, H265_INIT, "hevc").await;
    let endpoints = open(service(h265)).await;
    let (a2a_follow, mcp_follow) = follow_json(&endpoints.a2a, &endpoints.mcp, "enc").await;
    same_follow_json("h265 follow metadata", &a2a_follow, &mcp_follow);
    assert_eq!(a2a_follow["codec"], "h265");
    assert_eq!(mcp_follow["codec"], "h265");

    let endpoints = open(service(span_catalog().await)).await;
    let over = TckMalformedVideoRequests::get_video_clip_empty_range(
        source_id("span"),
        timestamp("2024-01-01T00:00:00Z"),
    );
    let a2a_code = error_code_a2a(&endpoints.a2a, A2aLabCommand::GetVideoClip(over.clone())).await;
    let mcp_code = error_code_mcp(&endpoints.mcp, A2aLabCommand::GetVideoClip(over)).await;
    if a2a_code != mcp_code {
        parity_fail(
            "one-hour-plus clip",
            &format!("code={a2a_code}"),
            &format!("code={mcp_code}"),
        );
    }
    assert_eq!(a2a_code, "invalid");

    let sixty = {
        let start = timestamp("2024-01-01T00:00:00Z");
        let end = start.saturating_add_micros(i64::try_from(MAX_VIDEO_CLIP_DURATION_US).unwrap());
        GetVideoClipRequest::new(source_id("span"), TimeRange::new(start, end).unwrap()).unwrap()
    };
    let a2a_sixty = serde_json::to_value(endpoints.a2a.get_video_clip(sixty.clone()).await.unwrap())
        .unwrap();
    let outcome = endpoints
        .mcp
        .execute(A2aLabCommand::GetVideoClip(sixty))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_sixty) = outcome.task.result else {
        parity_fail("sixty-second clip", "ok", "unexpected result");
    };
    same_clip_json(
        "sixty-second selection",
        &a2a_sixty,
        &serde_json::to_value(mcp_sixty).unwrap(),
    );

    let expired_videos = MemoryVideos::with_clip_ttl(Duration::from_millis(1));
    expired_videos.insert_source(source("cam")).await.unwrap();
    expired_videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let clip = expired_videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let (_auth, a2a_base, mcp_origin) = open_auth(service(expired_videos)).await;
    let gated = clip.path();
    for (label, base) in [("A2A", a2a_base.as_str()), ("MCP", mcp_origin.as_str())] {
        let expired = reqwest::Client::new()
            .get(format!("{base}{gated}"))
            .bearer_auth("lab-token")
            .send()
            .await
            .unwrap();
        let (status, code, _) = media_error(expired).await;
        if status != 404 {
            parity_fail("expired session", label, &format!("status={status} code={code}"));
        }
        if code != "clip_expired" {
            parity_fail("expired session", label, &format!("code={code}"));
        }
    }
    for (label, base) in [("A2A", a2a_base.as_str()), ("MCP", mcp_origin.as_str())] {
        let denied = reqwest::Client::new()
            .get(format!("{base}{gated}"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401, "{label}");
        assert_eq!(header(&denied, "cache-control"), "no-store", "{label}");
        assert_eq!(header(&denied, "x-content-type-options"), "nosniff", "{label}");
    }
}

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

async fn media_error(response: reqwest::Response) -> (u16, String, String) {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let code = value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    (status, code, message)
}
