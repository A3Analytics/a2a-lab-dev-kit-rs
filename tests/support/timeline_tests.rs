const INTENTIONAL_LIVE_DIFFERENCE: &str = "SiLA LabVideos exposes retained timeline metadata and finite BinaryDownload clips but has no follow command and no post-call frame stream";

const MCP_MEDIA_IS_UNGATED: &str =
    "MCP /media/* has no bearer gate and is outside the /mcp host allowlist";
async fn listen_a2a_auth(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&lab).with_security(
        [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into(),
        vec![[("bearerAuth".to_owned(), Vec::new())].into()],
    );
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

struct TimelineCase {
    name: &'static str,
    source: &'static str,
    intervals: &'static [(&'static str, &'static str)],
    newest: Option<&'static str>,
    live: bool,
}

fn timeline_cases() -> &'static [TimelineCase] {
    &[
        TimelineCase {
            name: "empty retention",
            source: "empty",
            intervals: &[],
            newest: None,
            live: false,
        },
        TimelineCase {
            name: "one interval",
            source: "one",
            intervals: &[("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z")],
            newest: Some("2024-01-01T00:00:00Z"),
            live: false,
        },
        TimelineCase {
            name: "adjacent frames as one interval",
            source: "adjacent",
            intervals: &[("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z")],
            newest: Some("2024-01-01T00:00:01Z"),
            live: true,
        },
        TimelineCase {
            name: "genuine gap",
            source: "gap",
            intervals: &[
                ("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
                ("2024-01-01T00:00:10Z", "2024-01-01T00:00:11Z"),
            ],
            newest: Some("2024-01-01T00:00:10Z"),
            live: false,
        },
    ]
}

fn expected_timeline(case: &TimelineCase) -> TimelineView {
    TimelineView {
        source: case.source.to_owned(),
        intervals: case
            .intervals
            .iter()
            .map(|(start, end)| ((*start).to_owned(), (*end).to_owned()))
            .collect(),
        newest: case.newest.map(str::to_owned),
        live: case.live,
    }
}

async fn raw_http(port: u16, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("listener");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut body = Vec::new();
    let mut buf = vec![0_u8; 2048];
    let finished = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(read) => body.extend_from_slice(&buf[..read]),
            }
        }
    })
    .await;
    if finished.is_err() && body.is_empty() {
        diverge("raw http", "media", "no response");
    }
    String::from_utf8_lossy(&body).into_owned()
}

async fn wait_until(mut ready: impl FnMut() -> bool) {
    for _ in 0..50 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition was not met");
}

fn assert_follow_handle(follow: &a2a_lab_dev_kit::VideoFollow) {
    assert_eq!(follow.path(), video_live_path(follow.id()));
    assert_relative_media_path(follow.path());
    assert_no_media_bytes(&serde_json::to_value(follow).unwrap());
    assert_eq!(follow.media_type(), MJPEG_MEDIA_TYPE);
}

async fn authed_get(url: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(url)
        .bearer_auth("lab-token")
        .send()
        .await
        .unwrap()
}

fn assert_unshared_stream(response: &reqwest::Response) {
    if !response.status().is_success() {
        diverge("live headers", "media", &response.status().to_string());
    }
    let cache = header_value(response, "cache-control");
    if cache != "no-store" || cache.contains("public") {
        diverge("live cache", "media", &cache);
    }
    let kind = header_value(response, "content-type");
    if !kind.contains("multipart/x-mixed-replace") {
        diverge("live media type", "media", &kind);
    }
}

fn header_value(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

async fn assert_idle(response: &mut reqwest::Response) {
    let mut buf = Vec::new();
    let idle = tokio::time::timeout(Duration::from_millis(150), async {
        loop {
            if take_stamp(&mut buf).is_some() {
                return;
            }
            match response.chunk().await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
                Ok(None) | Err(_) => return,
            }
        }
    })
    .await;
    if idle.is_ok() {
        diverge(
            "post-subscription idle",
            "media",
            "frame arrived or the stream closed",
        );
    }
}

async fn read_stamps(response: &mut reqwest::Response, count: usize) -> Vec<String> {
    let mut buf = Vec::new();
    let mut stamps = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(2), async {
        while stamps.len() < count {
            if let Some(stamp) = take_stamp(&mut buf) {
                stamps.push(stamp);
                continue;
            }
            match response.chunk().await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
                Ok(None) | Err(_) => break,
            }
        }
    })
    .await;
    if finished.is_err() || stamps.len() != count {
        diverge("post-subscription frames", "media", &format!("{stamps:?}"));
    }
    stamps
}

fn assert_sila_has_no_live_follow() {
    let xml = include_str!("../../src/sila/standard/LabVideos.sila.xml");
    let proto = include_str!("../../proto/sila/LabVideos.proto");
    assert!(xml.contains("No command delivers frames added after the call."));
    assert!(proto.contains("No command delivers frames added after the call."));
    assert!(!xml.contains("FollowVideo"));
    assert!(!proto.contains("FollowVideo"));
    assert!(!xml.contains("<Observable>Yes</Observable>"));
    assert!(INTENTIONAL_LIVE_DIFFERENCE.contains("no follow command"));
    assert!(INTENTIONAL_LIVE_DIFFERENCE.contains("no post-call frame stream"));
}

#[cfg(feature = "sila2")]
#[tokio::test]
async fn timeline_parity_matches_a2a_mcp_and_sila() {
    let lab = share(timeline_catalog().await);
    let a2a_base = listen_a2a(Arc::clone(&lab)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&lab)).await;
    let a2a = A2aClient::new(&a2a_base).unwrap();
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let mut sila = SilaVideos::start(lab).await;
    for case in timeline_cases() {
        let a2a_view = view_timeline(
            &a2a.get_video_timeline(GetVideoTimelineRequest::new(source_id(case.source)))
                .await
                .unwrap_or_else(|error| diverge(case.name, "A2A", &error.to_string())),
        );
        let outcome = mcp
            .execute(A2aLabCommand::GetVideoTimeline(
                GetVideoTimelineRequest::new(source_id(case.source)),
            ))
            .await
            .unwrap_or_else(|error| diverge(case.name, "MCP", &error.to_string()));
        let A2aLabResult::GetVideoTimeline(timeline) = outcome.task.result else {
            diverge(case.name, "MCP", "unexpected result");
        };
        let mcp_view = view_timeline(&timeline);
        let sila_view = sila.timeline(case.source).await;
        same_timeline(case.name, &a2a_view, &mcp_view, &sila_view);
        let expected = expected_timeline(case);
        if a2a_view != expected {
            diverge(case.name, "fixture", &a2a_view.render());
        }
        if case.newest.is_none() {
            assert!(a2a_view.intervals.is_empty() && a2a_view.newest.is_none());
        }
    }
}

#[cfg(feature = "sila2")]
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn live_follow_parity_keeps_the_sila_absence_explicit() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    insert_frame(&videos, "cam", "2024-01-01T00:00:04Z").await;
    let lab = share(videos.clone());
    let a2a_base = listen_a2a_auth(Arc::clone(&lab)).await;
    let (mcp_url, _) = listen_mcp(Arc::clone(&lab)).await;
    let a2a = A2aClient::new(&a2a_base)
        .unwrap()
        .with_bearer_token("lab-token");
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let a2a_follow = a2a
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let mcp_follow = follow_mcp(&mcp, "cam").await;
    assert_follow_handle(&a2a_follow);
    assert_follow_handle(&mcp_follow);
    let a2a_url = a2a.video_live_url(a2a_follow.id());
    let mcp_url_live = mcp.video_live_url(mcp_follow.id()).unwrap();
    assert!(a2a_url.ends_with(a2a_follow.path()));
    assert!(mcp_url_live.ends_with(mcp_follow.path()));
    assert!(!a2a_url.contains("lab-token") && !a2a_url.contains('?'));
    assert!(!mcp_url_live.contains("access_token") && !mcp_url_live.contains('?'));
    let bare = McpLab::connect_transport(rmcp::transport::StreamableHttpClientTransport::from_uri(
        mcp_url.as_str(),
    ))
    .await
    .unwrap();
    let origin = bare.video_live_url(mcp_follow.id()).unwrap_err();
    expect_code("video_live_url origin", "MCP", origin.code(), "unavailable");
    assert!(origin.to_string().contains("HTTP MCP origin"));
    let Err(trait_default) = mcp.open_video_follow(mcp_follow.id()).await else {
        diverge(
            "mcp trait default open_video_follow",
            "MCP",
            "opened a stream",
        )
    };
    expect_code(
        "mcp trait default open_video_follow",
        "MCP",
        trait_default.code(),
        "unavailable",
    );
    assert!(trait_default.availability().is_none());

    let anonymous = reqwest::Client::new().get(&a2a_url).send().await.unwrap();
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);
    let queried = reqwest::Client::new()
        .get(format!("{a2a_url}?access_token=lab-token"))
        .send()
        .await
        .unwrap();
    assert_eq!(queried.status(), reqwest::StatusCode::UNAUTHORIZED);

    let mut a2a_body = authed_get(&a2a_url).await;
    let mut mcp_body = reqwest::Client::new()
        .get(&mcp_url_live)
        .send()
        .await
        .unwrap();
    assert_unshared_stream(&a2a_body);
    assert_unshared_stream(&mcp_body);
    assert_idle(&mut a2a_body).await;
    assert_idle(&mut mcp_body).await;
    insert_frame(&videos, "cam", "2024-01-01T00:00:06Z").await;
    insert_frame(&videos, "cam", "2024-01-01T00:00:05Z").await;
    let expected = vec![
        "2024-01-01T00:00:05Z".to_owned(),
        "2024-01-01T00:00:06Z".to_owned(),
    ];
    let a2a_stamps = read_stamps(&mut a2a_body, 2).await;
    let mcp_stamps = read_stamps(&mut mcp_body, 2).await;
    if a2a_stamps != expected {
        diverge(
            "post-subscription frames",
            "A2A",
            &format!("{a2a_stamps:?}"),
        );
    }
    if mcp_stamps != expected {
        diverge(
            "post-subscription frames",
            "MCP",
            &format!("{mcp_stamps:?}"),
        );
    }
    drop(a2a_body);
    let later = follow_mcp(&mcp, "cam").await;
    let mut later_body = reqwest::Client::new()
        .get(mcp.video_live_url(later.id()).unwrap())
        .send()
        .await
        .unwrap();
    assert_unshared_stream(&later_body);
    assert_idle(&mut later_body).await;
    insert_frame(&videos, "cam", "2024-01-01T00:00:07Z").await;
    let mcp_next = read_stamps(&mut mcp_body, 1).await;
    let later_stamps = read_stamps(&mut later_body, 1).await;
    if mcp_next != ["2024-01-01T00:00:07Z"] {
        diverge("cancellation", "MCP", &format!("{mcp_next:?}"));
    }
    if later_stamps != ["2024-01-01T00:00:07Z"] {
        diverge("no shared cache", "MCP", &format!("{later_stamps:?}"));
    }

    let mut sila = SilaVideos::start(lab).await;
    let live = sila.timeline("cam").await;
    if !live.live {
        diverge("live metadata", "SiLA", &live.render());
    }
    assert_sila_has_no_live_follow();
}

#[tokio::test]
async fn media_security_covers_paths_cache_and_disconnect() {
    assert_sila_has_no_live_follow();
    assert!(MCP_MEDIA_IS_UNGATED.contains("no bearer gate"));
    assert_authenticated_media_paths().await;
    assert_mcp_media_is_outside_the_host_allowlist().await;
    assert_disconnect_cleanup().await;
}

#[allow(clippy::cognitive_complexity)]
async fn assert_authenticated_media_paths() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let lab = share(videos);
    let base = listen_a2a_auth(Arc::clone(&lab)).await;
    let client = A2aClient::new(&base)
        .unwrap()
        .with_bearer_token("lab-token");
    let follow = client
        .follow_video(FollowVideoRequest::new(cam()))
        .await
        .unwrap();
    assert_follow_handle(&follow);
    let url = client.video_live_url(follow.id());
    assert!(url.ends_with(follow.path()));
    assert!(!url.contains("lab-token") && !url.contains('?'));
    let anonymous = reqwest::Client::new().get(&url).send().await.unwrap();
    if anonymous.status() != reqwest::StatusCode::UNAUTHORIZED {
        diverge(
            "no public live route",
            "A2A",
            &anonymous.status().to_string(),
        );
    }
    let queried = reqwest::Client::new()
        .get(format!("{url}?access_token=lab-token"))
        .send()
        .await
        .unwrap();
    if queried.status() != reqwest::StatusCode::UNAUTHORIZED {
        diverge("bearer query", "A2A", &queried.status().to_string());
    }
    let first = authed_get(&url).await;
    assert_unshared_stream(&first);
    let second = authed_get(&url).await;
    let (status, code, message) = media_error(second).await;
    expect_media(
        "no shared cache",
        "A2A",
        status,
        &code,
        &message,
        400,
        "invalid",
    );
    drop(first);

    let (mcp_url, _) = listen_mcp(lab).await;
    let mcp = McpLab::connect(&mcp_url).await.unwrap();
    let follow = follow_mcp(&mcp, "cam").await;
    assert_follow_handle(&follow);
    let live = mcp.video_live_url(follow.id()).unwrap();
    assert!(!live.contains("access_token") && !live.contains('?'));
    let response = reqwest::Client::new().get(&live).send().await.unwrap();
    assert_unshared_stream(&response);
    drop(response);
}

async fn assert_mcp_media_is_outside_the_host_allowlist() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    let (_, address) = listen_mcp(share(videos)).await;
    let port = address.port();
    let media = raw_http(
        port,
        &format!(
            "GET /media/video-live/missing-follow HTTP/1.1\r\nHost: evil.example:{port}\r\nconnection: close\r\n\r\n"
        ),
    )
    .await;
    if !media.contains("404") || media.contains("403") || !media.contains("not_found") {
        diverge("mcp media host", "MCP", &media);
    }
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}"#;
    let rejected = raw_http(
        port,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: evil.example:{port}\r\ncontent-type: application/json\r\naccept: application/json, text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .await;
    if !rejected.contains("403") {
        diverge("mcp host allowlist", "MCP", &rejected);
    }
}

async fn assert_disconnect_cleanup() {
    for transport in ["A2A", "MCP"] {
        let probe = CleanupProbe::new();
        let lab = share(probe.clone());
        let url = if transport == "A2A" {
            let base = listen_a2a(lab).await;
            let client = A2aClient::new(&base).unwrap();
            let follow = client
                .follow_video(FollowVideoRequest::new(cam()))
                .await
                .unwrap();
            client.video_live_url(follow.id())
        } else {
            let (mcp_url, _) = listen_mcp(lab).await;
            let mcp = McpLab::connect(&mcp_url).await.unwrap();
            let follow = follow_mcp(&mcp, "cam").await;
            mcp.video_live_url(follow.id()).unwrap()
        };
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        assert_unshared_stream(&response);
        let opened = Arc::clone(&probe.opened);
        wait_until(move || opened.load(Ordering::SeqCst) == 1).await;
        if probe.retained.load(Ordering::SeqCst) != 1 {
            diverge(
                "disconnect cleanup",
                transport,
                "stream did not retain its frame",
            );
        }
        drop(response);
        let dropped = Arc::clone(&probe.dropped);
        wait_until(move || dropped.load(Ordering::SeqCst)).await;
        if probe.retained.load(Ordering::SeqCst) != 0 {
            diverge(
                "disconnect cleanup",
                transport,
                "retained frames after disconnect",
            );
        }
    }
}

#[derive(Clone)]
struct CleanupProbe {
    opened: Arc<AtomicUsize>,
    retained: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl CleanupProbe {
    fn new() -> Self {
        Self {
            opened: Arc::new(AtomicUsize::new(0)),
            retained: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct LiveHold(CleanupProbe);

impl Drop for LiveHold {
    fn drop(&mut self) {
        self.0.retained.store(0, Ordering::SeqCst);
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

impl VideoProvider for CleanupProbe {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("probe")))
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("probe")))
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("probe")))
    }

    fn open_video_clip(
        &self,
        _id: &a2a_lab_dev_kit::VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("probe")))
    }

    fn follow_video(
        &self,
        request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        std::future::ready(VideoFollow::new(
            VideoFollowId::new("follow-probe").unwrap(),
            request.source_id().clone(),
            MJPEG_MEDIA_TYPE,
        ))
    }

    fn open_video_follow(
        &self,
        id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        if id.as_str() != "follow-probe" {
            return std::future::ready(Err(A2aLabError::not_found("video follow", id.to_string())));
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        self.retained.store(1, Ordering::SeqCst);
        let hold = LiveHold(self.clone());
        let stream: VideoLiveStream =
            Box::pin(futures_util::stream::unfold(hold, |hold| async move {
                std::future::pending::<()>().await;
                Some((Err(A2aLabError::unavailable("unused")), hold))
            }));
        std::future::ready(Ok(stream))
    }
}
