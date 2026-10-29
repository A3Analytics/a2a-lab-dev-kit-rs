#[cfg(feature = "sila2")]
#[tokio::test]
async fn availability_outcomes_stay_distinct_across_transports() {
    compare_coverage_outcomes().await;
    compare_expired_reads().await;
    compare_refused_live_read().await;
    compare_recording_gap_outcomes().await;
    compare_fixed_outcomes().await;
}

#[cfg(feature = "sila2")]
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn compare_coverage_outcomes() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("empty")).await.unwrap();
    videos.insert_source(source("cam")).await.unwrap();
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:10Z",
    ] {
        insert_frame(&videos, "cam", at).await;
    }
    let lab = share(videos);
    let base = listen_a2a(Arc::clone(&lab)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&lab)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let mut sila = SilaVideos::start(lab).await;

    let empty = view_timeline(
        &a2a.get_video_timeline(GetVideoTimelineRequest::new(source_id("empty")))
            .await
            .unwrap(),
    );
    let empty_mcp = mcp_timeline(&mcp, "empty").await;
    let empty_sila = sila.timeline("empty").await;
    same_timeline("empty retention", &empty, &empty_mcp, &empty_sila);
    if empty.newest.is_some() || !empty.intervals.is_empty() {
        diverge("empty retention", "fixture", &empty.render());
    }

    let missed = GetVideoClipRequest::new(
        source_id("empty"),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
    )
    .unwrap();
    expect_a2a_video(
        "no_recording",
        &base,
        &a2a,
        A2aLabCommand::GetVideoClip(missed.clone()),
        "no_recording",
    )
    .await;
    expect_mcp(
        "no_recording",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoClip(missed),
        "no_recording",
    )
    .await;
    expect_code(
        "no_recording",
        "SiLA",
        &sila
            .defined_clip(
                "empty",
                sila_utc(2024, 1, 1, 0, 0, 0),
                sila_utc(2024, 1, 1, 0, 0, 1),
            )
            .await,
        "NoRecording",
    );

    let outside =
        GetVideoClipRequest::new(cam(), range("2025-01-01T00:00:00Z", "2025-01-01T00:00:01Z"))
            .unwrap();
    expect_a2a_video(
        "no_recording outside coverage",
        &base,
        &a2a,
        A2aLabCommand::GetVideoClip(outside.clone()),
        "no_recording",
    )
    .await;
    expect_mcp(
        "no_recording outside coverage",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoClip(outside),
        "no_recording",
    )
    .await;
    expect_code(
        "no_recording outside coverage",
        "SiLA",
        &sila
            .defined_clip(
                "cam",
                sila_utc(2025, 1, 1, 0, 0, 0),
                sila_utc(2025, 1, 1, 0, 0, 1),
            )
            .await,
        "NoRecording",
    );

    let spanning =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z"))
            .unwrap();
    let requested = spanning.range();
    let clip = a2a.get_video_clip(spanning.clone()).await.unwrap();
    assert_eq!(clip.requested_range(), requested);
    assert_eq!(
        clip.effective_range(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z")
    );
    assert_no_media_bytes(&serde_json::to_value(&clip).unwrap());
    let mcp_clip = mcp
        .execute(A2aLabCommand::GetVideoClip(spanning))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_meta) = mcp_clip.task.result else {
        diverge("gap-spanning clip", "MCP", "unexpected result");
    };
    assert_eq!(mcp_meta.effective_range(), clip.effective_range());
    sila
        .clip_bytes(
            "cam",
            sila_utc(2024, 1, 1, 0, 0, 0),
            sila_utc(2024, 1, 1, 0, 0, 11),
        )
        .await;

    let frame = GetVideoFrameAtRequest::new(cam(), timestamp("2025-01-01T00:00:00Z"), None);
    expect_a2a_status(
        "frame miss",
        &base,
        &a2a,
        A2aLabCommand::GetVideoFrameAt(frame.clone()),
        "not_found",
    )
    .await;
    expect_mcp(
        "frame miss",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoFrameAt(frame),
        "not_found",
    )
    .await;
    expect_code(
        "frame miss",
        "SiLA",
        &sila
            .defined_frame("cam", sila_utc(2025, 1, 1, 0, 0, 0))
            .await,
        "VideoFrameNotFound",
    );

    let missing = GetVideoTimelineRequest::new(source_id("missing"));
    expect_a2a_status(
        "unknown source",
        &base,
        &a2a,
        A2aLabCommand::GetVideoTimeline(missing.clone()),
        "not_found",
    )
    .await;
    expect_mcp(
        "unknown source",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoTimeline(missing),
        "not_found",
    )
    .await;
    expect_code(
        "unknown source",
        "SiLA",
        &sila.defined_timeline("missing").await,
        "VideoSourceNotFound",
    );

    let follow = FollowVideoRequest::new(cam());
    expect_a2a_video(
        "live_unavailable",
        &base,
        &a2a,
        A2aLabCommand::FollowVideo(follow.clone()),
        "live_unavailable",
    )
    .await;
    expect_mcp(
        "live_unavailable",
        &mcp_url,
        &mcp,
        A2aLabCommand::FollowVideo(follow),
        "live_unavailable",
    )
    .await;
    let quiet = sila.timeline("cam").await;
    if quiet.live {
        diverge("live off", "SiLA", &quiet.render());
    }

    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(format!("{base}/media/video-clips/missing-clip"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "unknown clip",
        "A2A",
        status,
        &code,
        &message,
        404,
        "not_found",
    );
    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(format!(
                "{}/media/video-clips/missing-clip",
                mcp_url.trim_end_matches("/mcp")
            ))
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "unknown clip",
        "MCP",
        status,
        &code,
        &message,
        404,
        "not_found",
    );
}

#[cfg(feature = "sila2")]
async fn mcp_timeline(lab: &McpLab, source: &str) -> TimelineView {
    let outcome = lab
        .execute(A2aLabCommand::GetVideoTimeline(
            GetVideoTimelineRequest::new(source_id(source)),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoTimeline(timeline) = outcome.task.result else {
        diverge("timeline", "MCP", "unexpected result");
    };
    view_timeline(&timeline)
}

#[cfg(feature = "sila2")]
async fn compare_expired_reads() {
    let videos = MemoryVideos::with_clip_ttl(Duration::ZERO);
    videos.insert_source(source("cam")).await.unwrap();
    insert_frame(&videos, "cam", "2024-01-01T00:00:00Z").await;
    let lab = share(videos);
    let base = listen_a2a(Arc::clone(&lab)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&lab)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let request =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"))
            .unwrap();
    let clip = a2a.get_video_clip(request.clone()).await.unwrap();
    assert_no_media_bytes(&serde_json::to_value(&clip).unwrap());
    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(format!("{base}{}", clip.path()))
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "clip_expired",
        "A2A",
        status,
        &code,
        &message,
        404,
        "clip_expired",
    );
    let mcp_clip = mcp
        .execute(A2aLabCommand::GetVideoClip(request))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_clip) = mcp_clip.task.result else {
        diverge("clip_expired", "MCP", "unexpected result");
    };
    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(format!(
                "{}{}",
                mcp_url.trim_end_matches("/mcp"),
                mcp_clip.path()
            ))
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "clip_expired",
        "MCP",
        status,
        &code,
        &message,
        404,
        "clip_expired",
    );
    let mut sila = SilaVideos::start(Arc::clone(&lab)).await;
    // The SiLA command reads the finite body while preparing BinaryDownload.
    expect_code(
        "clip_expired",
        "SiLA",
        &sila
            .defined_clip(
                "cam",
                sila_utc(2024, 1, 1, 0, 0, 0),
                sila_utc(2024, 1, 1, 0, 0, 1),
            )
            .await,
        "ClipExpired",
    );
}

#[cfg(feature = "sila2")]
#[allow(clippy::cognitive_complexity)]
async fn compare_refused_live_read() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let lab = share(videos.clone());
    let base = listen_a2a(Arc::clone(&lab)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&lab)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let a2a_follow = a2a
        .follow_video(FollowVideoRequest::new(cam()))
        .await
        .unwrap();
    let mcp_follow = follow_mcp(&mcp, "cam").await;
    videos
        .set_live_available(&source_id("cam"), false)
        .await
        .unwrap();
    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(a2a.video_live_url(a2a_follow.id()))
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "live_unavailable",
        "A2A",
        status,
        &code,
        &message,
        503,
        "live_unavailable",
    );
    let (status, code, message) = media_error(
        reqwest::Client::new()
            .get(mcp.video_live_url(mcp_follow.id()).unwrap())
            .send()
            .await
            .unwrap(),
    )
    .await;
    expect_media(
        "live_unavailable",
        "MCP",
        status,
        &code,
        &message,
        503,
        "live_unavailable",
    );
    let mut sila = SilaVideos::start(lab).await;
    let quiet = sila.timeline("cam").await;
    if quiet.live {
        diverge("live off is LiveAvailable=false", "SiLA", &quiet.render());
    }
}

#[cfg(feature = "sila2")]
async fn compare_recording_gap_outcomes() {
    let gap_provider = share(FixedVideoError(A2aLabError::recording_gap(
        "requested range crosses a recording gap",
    )));
    let base = listen_a2a(Arc::clone(&gap_provider)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&gap_provider)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let gap_request =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"))
            .unwrap();
    expect_a2a_video(
        "recording_gap",
        &base,
        &a2a,
        A2aLabCommand::GetVideoClip(gap_request.clone()),
        "recording_gap",
    )
    .await;
    expect_mcp(
        "recording_gap",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoClip(gap_request),
        "recording_gap",
    )
    .await;
    let mut sila = SilaVideos::start(gap_provider).await;
    expect_code(
        "recording_gap",
        "SiLA",
        &sila
            .defined_clip(
                "cam",
                sila_utc(2024, 1, 1, 0, 0, 0),
                sila_utc(2024, 1, 1, 0, 0, 1),
            )
            .await,
        "RecordingGap",
    );
}

#[cfg(feature = "sila2")]
#[allow(clippy::cognitive_complexity)]
async fn compare_fixed_outcomes() {
    let refused = share(FixedVideoError(A2aLabError::live_unavailable(
        "live follow is not available",
    )));
    let base = listen_a2a(Arc::clone(&refused)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&refused)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let command = A2aLabCommand::GetVideoTimeline(GetVideoTimelineRequest::new(cam()));
    expect_a2a_video(
        "provider live_unavailable",
        &base,
        &a2a,
        command.clone(),
        "live_unavailable",
    )
    .await;
    expect_mcp(
        "provider live_unavailable",
        &mcp_url,
        &mcp,
        command,
        "live_unavailable",
    )
    .await;
    let mut sila = SilaVideos::start(refused).await;
    expect_code(
        "provider live_unavailable",
        "SiLA",
        &sila.defined_timeline("cam").await,
        "LiveUnavailable",
    );

    let missing = share(FixedVideoError(A2aLabError::not_found(
        "video clip",
        "ghost",
    )));
    let base = listen_a2a(Arc::clone(&missing)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&missing)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let request =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"))
            .unwrap();
    expect_a2a_status(
        "unknown clip",
        &base,
        &a2a,
        A2aLabCommand::GetVideoClip(request.clone()),
        "not_found",
    )
    .await;
    expect_mcp(
        "unknown clip",
        &mcp_url,
        &mcp,
        A2aLabCommand::GetVideoClip(request),
        "not_found",
    )
    .await;
    let mut sila = SilaVideos::start(missing).await;
    expect_code(
        "unknown clip",
        "SiLA",
        &sila
            .defined_clip(
                "cam",
                sila_utc(2024, 1, 1, 0, 0, 0),
                sila_utc(2024, 1, 1, 0, 0, 1),
            )
            .await,
        "VideoClipNotFound",
    );

    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    videos.set_unavailable("videos offline").await;
    let down = share(videos);
    let base = listen_a2a(Arc::clone(&down)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&down)).await;
    let a2a = A2aClient::new(&base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let command = A2aLabCommand::GetVideoTimeline(GetVideoTimelineRequest::new(cam()));
    expect_a2a_status("provider down", &base, &a2a, command.clone(), "unavailable").await;
    expect_mcp("provider down", &mcp_url, &mcp, command, "unavailable").await;
    let mut sila = SilaVideos::start(down).await;
    expect_code(
        "provider down",
        "SiLA",
        &sila.defined_timeline("cam").await,
        "ProviderUnavailable",
    );
}

#[derive(Clone)]
struct FixedVideoError(A2aLabError);

impl VideoProvider for FixedVideoError {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn open_video_clip(
        &self,
        _id: &a2a_lab_dev_kit::VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn follow_video(
        &self,
        _request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }

    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        std::future::ready(Err(self.0.clone()))
    }
}
