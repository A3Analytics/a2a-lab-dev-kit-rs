struct Endpoints {
    a2a: A2aClient,
    mcp: McpLab,
}

enum Op {
    List(ListVideoSourcesRequest),
    Frame(GetVideoFrameAtRequest),
    Clip(GetVideoClipRequest),
}

enum Outcome {
    Sources(Page<VideoSource>),
    Frame(GetVideoFrameAtResult),
    Clip(VideoClip),
    Error(A2aLabError),
}

async fn catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos.insert_source(source("zeta")).await.unwrap();
    videos.insert_source(source("cam")).await.unwrap();
    videos.insert_source(source("empty")).await.unwrap();
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        videos
            .insert_frame(
                VideoSourceId::new("cam").unwrap(),
                timestamp(at),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    videos
}

fn service(videos: MemoryVideos) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

async fn open(lab: Arc<dyn A2aLabApi>) -> Endpoints {
    let a2a_lab = lab.clone();
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&a2a_lab).listen(a2a_listener).await.unwrap();
    });
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(mcp_listener).await.unwrap();
    });
    Endpoints {
        a2a: A2aClient::new(&format!("http://{a2a_address}")).unwrap(),
        mcp: McpLab::connect(&format!("http://{mcp_address}/mcp"))
            .await
            .unwrap(),
    }
}

fn command(op: &Op) -> A2aLabCommand {
    match op {
        Op::List(request) => A2aLabCommand::ListVideoSources(request.clone()),
        Op::Frame(request) => A2aLabCommand::GetVideoFrameAt(request.clone()),
        Op::Clip(request) => A2aLabCommand::GetVideoClip(request.clone()),
    }
}

async fn observe_a2a(client: &A2aClient, op: &Op) -> Outcome {
    let result = match op {
        Op::List(request) => client
            .list_video_sources(request.clone())
            .await
            .map(Outcome::Sources),
        Op::Frame(request) => client
            .get_video_frame_at(request.clone())
            .await
            .map(Outcome::Frame),
        Op::Clip(request) => client
            .get_video_clip(request.clone())
            .await
            .map(Outcome::Clip),
    };
    result.unwrap_or_else(Outcome::Error)
}

async fn observe_mcp(lab: &McpLab, op: &Op) -> Outcome {
    match lab.execute(command(op)).await {
        Ok(outcome) => match (op, outcome.task.result) {
            (Op::List(_), A2aLabResult::ListVideoSources(page)) => Outcome::Sources(page),
            (Op::Frame(_), A2aLabResult::GetVideoFrameAt(frame)) => Outcome::Frame(frame),
            (Op::Clip(_), A2aLabResult::GetVideoClip(clip)) => Outcome::Clip(clip),
            (_, result) => Outcome::Error(A2aLabError::protocol(format!("{result:?}"))),
        },
        Err(error) => Outcome::Error(error),
    }
}

fn same(left: &Outcome, right: &Outcome) -> bool {
    match (left, right) {
        (Outcome::Sources(left), Outcome::Sources(right)) => left == right,
        (Outcome::Frame(left), Outcome::Frame(right)) => {
            left.delta_us() == right.delta_us()
                && left.image().data() == right.image().data()
                && left.actual_captured_at() == right.actual_captured_at()
        }
        (Outcome::Clip(left), Outcome::Clip(right)) => {
            left.source_id() == right.source_id()
                && left.requested_range() == right.requested_range()
                && left.path() == video_clip_path(left.id())
                && right.path() == video_clip_path(right.id())
                && serde_json::to_value(left).unwrap().get("data").is_none()
                && serde_json::to_value(right).unwrap().get("data").is_none()
        }
        (Outcome::Error(left), Outcome::Error(right)) => left.code() == right.code(),
        _ => false,
    }
}

fn render(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Sources(page) => format!(
            "sources ids={:?}",
            page.items()
                .iter()
                .map(|item| item.id().as_str())
                .collect::<Vec<_>>()
        ),
        Outcome::Frame(frame) => format!(
            "frame at={} delta={} bytes={}",
            frame.actual_captured_at(),
            frame.delta_us(),
            frame.image().data().len()
        ),
        Outcome::Clip(clip) => format!(
            "clip id={} path={} range={:?} data={}",
            clip.id(),
            clip.path(),
            clip.requested_range(),
            serde_json::to_value(clip).unwrap().get("data").is_some()
        ),
        Outcome::Error(error) => format!("error code={} message={error}", error.code()),
    }
}

fn report_clients(case: &str, a2a: &Outcome, mcp: &Outcome) -> ! {
    panic!("case {case}\nA2A: {}\nMCP: {}", render(a2a), render(mcp));
}

async fn compare(endpoints: &Endpoints, name: &str, op: Op, expect: fn(&Outcome) -> bool) {
    let a2a = observe_a2a(&endpoints.a2a, &op).await;
    let mcp = observe_mcp(&endpoints.mcp, &op).await;
    if !same(&a2a, &mcp) {
        report_clients(name, &a2a, &mcp);
    }
    assert!(
        expect(&a2a),
        "case {name} shared result diverged from the fixture\nA2A: {}\nMCP: {}",
        render(&a2a),
        render(&mcp)
    );
}

fn source_ids(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Sources(page) => page
            .items()
            .iter()
            .map(|item| item.id().as_str().to_owned())
            .collect(),
        _ => Vec::new(),
    }
}

fn code_is(outcome: &Outcome, code: &str) -> bool {
    matches!(outcome, Outcome::Error(error) if error.code() == code)
}

async fn compare_catalog(endpoints: &Endpoints) {
    compare(
        endpoints,
        "list video sources",
        Op::List(ListVideoSourcesRequest::new(page(10)).unwrap()),
        |outcome| source_ids(outcome) == ["cam", "empty", "zeta"],
    )
    .await;
    compare(
        endpoints,
        "frame at exact stamp",
        Op::Frame(GetVideoFrameAtRequest::new(
            cam(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )),
        |outcome| matches!(outcome, Outcome::Frame(frame) if frame.delta_us() == 0 && frame.image().data() == JPEG),
    )
    .await;
    compare(
        endpoints,
        "frame nearest within delta",
        Op::Frame(GetVideoFrameAtRequest::new(
            cam(),
            timestamp("2024-01-01T00:00:00.400000Z"),
            Some(1_000_000),
        )),
        |outcome| {
            matches!(outcome, Outcome::Frame(frame) if frame.actual_captured_at().to_rfc3339() == "2024-01-01T00:00:00Z")
        },
    )
    .await;
    compare(
        endpoints,
        "clip half-open range excludes end",
        Op::Clip(
            GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"))
                .unwrap(),
        ),
        |outcome| match outcome {
            Outcome::Clip(clip) => {
                let encoded = serde_json::to_value(clip).unwrap();
                encoded.get("data").is_none()
                    && encoded.get("jpeg").is_none()
                    && clip.path() == video_clip_path(clip.id())
                    && clip.requested_range().end() == timestamp("2024-01-01T00:00:02Z")
            }
            _ => false,
        },
    )
    .await;
    compare(
        endpoints,
        "unknown source",
        Op::Frame(GetVideoFrameAtRequest::new(
            VideoSourceId::new("missing").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )),
        |outcome| code_is(outcome, "not_found"),
    )
    .await;
    compare(
        endpoints,
        "clip outside coverage",
        Op::Clip(
            GetVideoClipRequest::new(cam(), range("2025-01-01T00:00:00Z", "2025-01-01T00:00:01Z"))
                .unwrap(),
        ),
        |outcome| code_is(outcome, "no_recording"),
    )
    .await;
}

async fn compare_malformed(endpoints: &Endpoints) {
    compare(
        endpoints,
        "list_video_sources zero limit",
        Op::List(TckMalformedVideoRequests::list_video_sources_zero_limit()),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
    compare(
        endpoints,
        "get_video_clip reversed range",
        Op::Clip(TckMalformedVideoRequests::get_video_clip_reversed_range(
            cam(),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        )),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
    compare(
        endpoints,
        "get_video_clip over limit",
        Op::Clip(TckMalformedVideoRequests::get_video_clip_empty_range(
            cam(),
            timestamp("2024-01-01T00:00:00Z"),
        )),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
}
