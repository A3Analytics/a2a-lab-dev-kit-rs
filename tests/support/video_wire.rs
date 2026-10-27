const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn source(id: &str) -> VideoSource {
    VideoSource::new(
        VideoSourceId::new(id).unwrap(),
        id,
        format!("{id} recording"),
        None,
        None,
        1,
        1,
        1,
        MJPEG_MEDIA_TYPE,
    )
    .unwrap()
}

fn cam() -> VideoSourceId {
    VideoSourceId::new("cam").unwrap()
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn share(videos: impl VideoProvider + 'static) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

async fn listen_a2a(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&lab).listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn listen_mcp(lab: Arc<dyn A2aLabApi>) -> (String, std::net::SocketAddr) {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(listener).await.unwrap();
    });
    (format!("http://{address}/mcp"), address)
}

fn assert_no_media_bytes(value: &Value) {
    let text = value.to_string();
    assert!(value.get("data").is_none(), "{text}");
    assert!(value.get("jpeg").is_none(), "{text}");
    assert!(value.get("frames").is_none(), "{text}");
    assert!(!text.contains("/9j/"), "{text}");
    assert!(!text.contains("data:video"), "{text}");
}

fn assert_relative_media_path(path: &str) {
    assert!(path.starts_with("/media/"));
    assert!(!path.contains("://"));
    assert!(!path.contains('?'));
    assert!(!path.contains('#'));
    assert!(!path.contains('@'));
    assert!(!path.contains("lab-token"));
    assert!(!path.contains("access_token"));
}

fn diverge(case: &str, transport: &str, observed: &str) -> ! {
    panic!("case {case}\ntransport {transport}\nobserved {observed}");
}

fn expect_code(case: &str, transport: &str, observed: &str, expected: &str) {
    if observed != expected {
        diverge(case, transport, observed);
    }
}

#[derive(Clone, PartialEq, Eq)]
struct TimelineView {
    source: String,
    intervals: Vec<(String, String)>,
    newest: Option<String>,
    live: bool,
}

impl TimelineView {
    fn render(&self) -> String {
        format!(
            "source={} intervals={:?} newest={:?} live={}",
            self.source, self.intervals, self.newest, self.live
        )
    }
}

async fn timeline_catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    for id in ["empty", "one", "adjacent", "gap"] {
        videos.insert_source(source(id)).await.unwrap();
    }
    insert_frame(&videos, "one", "2024-01-01T00:00:00Z").await;
    for at in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"] {
        insert_frame(&videos, "adjacent", at).await;
    }
    for at in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:10Z"] {
        insert_frame(&videos, "gap", at).await;
    }
    videos
        .set_live_available(&source_id("adjacent"), true)
        .await
        .unwrap();
    videos
}

fn view_timeline(timeline: &VideoTimeline) -> TimelineView {
    let encoded = serde_json::to_value(timeline).unwrap();
    assert_no_media_bytes(&encoded);
    if timeline.newest_at().is_none() {
        assert!(encoded.get("newest_at").is_none(), "{encoded}");
    }
    TimelineView {
        source: timeline.source_id().as_str().to_owned(),
        intervals: timeline
            .intervals()
            .iter()
            .map(|interval| (interval.start().to_rfc3339(), interval.end().to_rfc3339()))
            .collect(),
        newest: timeline.newest_at().map(|stamp| stamp.to_rfc3339()),
        live: timeline.live_available(),
    }
}

async fn insert_frame(videos: &MemoryVideos, id: &str, at: &str) {
    videos
        .insert_frame(source_id(id), timestamp(at), JPEG.to_vec())
        .await
        .unwrap();
}

fn same_timeline(case: &str, a2a: &TimelineView, mcp: &TimelineView, sila: &TimelineView) {
    assert!(
        a2a == mcp && a2a == sila,
        "case {case}\ntransport A2A {}\ntransport MCP {}\ntransport SiLA {}",
        a2a.render(),
        mcp.render(),
        sila.render()
    );
}

async fn send_lab(base: &str, command: &A2aLabCommand) -> Value {
    let response = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": "video-parity",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": LAB_MEDIA_TYPE,
                    "data": command
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    serde_json::from_str(&body).unwrap_or_else(|_| panic!("case a2a\nobserved {body}"))
}

fn metadata_code(body: &Value) -> Option<&str> {
    body.pointer("/error/details")?
        .as_array()?
        .iter()
        .find_map(|detail| detail.pointer("/metadata/code")?.as_str())
}

fn error_reason(body: &Value) -> Option<&str> {
    let details = body.pointer("/error/details")?.as_array()?;
    details.iter().find_map(|detail| {
        detail
            .get("reason")
            .and_then(Value::as_str)
            .or_else(|| detail.pointer("/value/reason").and_then(Value::as_str))
    })
}

fn a2a_video_code(case: &str, body: &Value) -> String {
    let Some(code) = metadata_code(body) else {
        diverge(case, "A2A", &body.to_string());
    };
    let reason = error_reason(body).unwrap_or("");
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("");
    if reason != "INTERNAL_ERROR" || code == "INTERNAL_ERROR" || message == code || reason == code {
        diverge(
            case,
            "A2A",
            &format!("metadata.code={code} reason={reason} message={message} body={body}"),
        );
    }
    code.to_owned()
}

async fn mcp_data_code(url: &str, command: &A2aLabCommand) -> (String, String) {
    use rmcp::ServiceExt;
    use rmcp::model::{
        CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
    };
    use rmcp::service::ServiceError;
    use rmcp::transport::StreamableHttpClientTransport;

    let value = serde_json::to_value(command).unwrap();
    let tool = value["operation"].as_str().unwrap().to_owned();
    let params = value["params"].as_object().cloned().unwrap_or_default();
    let mut last = String::new();
    for _ in 0..50 {
        let config = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("a2a-lab-parity", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2026_07_28);
        match config
            .serve(StreamableHttpClientTransport::from_uri(url))
            .await
        {
            Ok(client) => {
                let error = client
                    .call_tool(CallToolRequestParams::new(tool.clone()).with_arguments(params))
                    .await
                    .expect_err("tool error");
                let ServiceError::McpError(data) = error else {
                    diverge("mcp data.code", "MCP", &error.to_string());
                };
                let code = data
                    .data
                    .as_ref()
                    .and_then(|value| value.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                return (code, data.message.to_string());
            }
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    diverge("mcp connect", "MCP", &last);
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

fn take_stamp(buf: &mut Vec<u8>) -> Option<String> {
    let marker = b"X-Captured-At: ";
    let start = buf
        .windows(marker.len())
        .position(|window| window == marker)?;
    let value_at = start + marker.len();
    let end = buf[value_at..]
        .windows(2)
        .position(|window| window == b"\r\n")?;
    let stamp = String::from_utf8(buf[value_at..value_at + end].to_vec()).ok()?;
    buf.drain(..value_at + end + 2);
    Some(stamp)
}

fn stamps_in(body: &[u8]) -> Vec<String> {
    let mut buf = body.to_vec();
    let mut stamps = Vec::new();
    while let Some(stamp) = take_stamp(&mut buf) {
        stamps.push(stamp);
    }
    stamps
}

async fn clip_stamps(stream: VideoClipStream) -> Vec<String> {
    let mut stream = stream;
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    stamps
}

#[cfg(feature = "sila2")]
struct SilaVideos {
    _server: a2a_lab_dev_kit::sila::SilaServerHandle,
    channel: tonic::transport::Channel,
    client: a2a_lab_dev_kit::sila::api::LabVideosClient<tonic::transport::Channel>,
}

#[cfg(feature = "sila2")]
impl SilaVideos {
    async fn start(lab: Arc<dyn A2aLabApi>) -> Self {
        use a2a_lab_dev_kit::sila::api::LabVideosClient;
        use a2a_lab_dev_kit::sila::{SilaIdentity, SilaServer};

        let server = SilaServer::new(
            SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
            lab,
        )
        .plaintext()
        .serve("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
        let channel =
            tonic::transport::Channel::from_shared(format!("http://{}", server.local_addr()))
                .unwrap()
                .connect()
                .await
                .unwrap();
        Self {
            client: LabVideosClient::new(channel.clone()),
            channel,
            _server: server,
        }
    }

    async fn timeline(&mut self, source: &str) -> TimelineView {
        use a2a_lab_dev_kit::sila::api::{GetVideoTimelineParameters, SilaString};

        let body = self
            .client
            .get_video_timeline(GetVideoTimelineParameters {
                source_id: Some(SilaString {
                    value: source.to_owned(),
                }),
            })
            .await
            .unwrap_or_else(|error| diverge("timeline", "SiLA", &decoded(&error)))
            .into_inner();
        let newest = if body.has_newest_at.as_ref().is_some_and(|flag| flag.value) {
            Some(stamp_text(body.newest_at.as_ref().unwrap()))
        } else {
            assert!(
                body.newest_at.is_none(),
                "case empty retention\ntransport SiLA\nobserved newest timestamp present"
            );
            None
        };
        TimelineView {
            source: body.source_id.unwrap().value,
            intervals: body.intervals.iter().map(range_text).collect(),
            newest,
            live: body.live_available.unwrap().value,
        }
    }

    async fn defined_timeline(&mut self, source: &str) -> String {
        use a2a_lab_dev_kit::sila::api::{GetVideoTimelineParameters, SilaString};

        let status = self
            .client
            .get_video_timeline(GetVideoTimelineParameters {
                source_id: Some(SilaString {
                    value: source.to_owned(),
                }),
            })
            .await
            .err()
            .unwrap_or_else(|| diverge("timeline", "SiLA", "success"));
        sila_defined_name(&status)
    }

    async fn defined_clip(
        &mut self,
        source: &str,
        start: a2a_lab_dev_kit::sila::api::Timestamp,
        end: a2a_lab_dev_kit::sila::api::Timestamp,
    ) -> String {
        let status = self
            .client
            .get_video_clip(clip_params(source, start, end))
            .await
            .err()
            .unwrap_or_else(|| diverge("clip", "SiLA", "success"));
        sila_defined_name(&status)
    }

    async fn defined_frame(
        &mut self,
        source: &str,
        at: a2a_lab_dev_kit::sila::api::Timestamp,
    ) -> String {
        let status = self
            .client
            .get_video_frame_at(frame_params(source, at))
            .await
            .err()
            .unwrap_or_else(|| diverge("frame", "SiLA", "success"));
        sila_defined_name(&status)
    }

    async fn clip_bytes(
        &mut self,
        source: &str,
        start: a2a_lab_dev_kit::sila::api::Timestamp,
        end: a2a_lab_dev_kit::sila::api::Timestamp,
    ) -> (String, Vec<u8>) {
        use a2a_lab_dev_kit::sila::api::{
            BinaryDownloadClient, BinaryUnion, GetBinaryInfoRequest, GetChunkRequest,
        };

        let clip = self
            .client
            .get_video_clip(clip_params(source, start, end))
            .await
            .unwrap_or_else(|error| diverge("clip", "SiLA", &decoded(&error)))
            .into_inner();
        let path = clip.path.unwrap().value;
        let transfer = match clip.clip.unwrap().union {
            Some(BinaryUnion::BinaryTransferUuid(id)) => id,
            other => diverge("clip", "SiLA", &format!("{other:?}")),
        };
        let mut downloads = BinaryDownloadClient::new(self.channel.clone());
        let info = downloads
            .get_binary_info(GetBinaryInfoRequest {
                binary_transfer_uuid: transfer.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        let mut stream = downloads
            .get_chunk(futures_util::stream::iter([GetChunkRequest {
                binary_transfer_uuid: transfer,
                offset: 0,
                length: u32::try_from(info.binary_size).unwrap_or(1024),
            }]))
            .await
            .unwrap()
            .into_inner();
        let chunk = stream.next().await.unwrap().unwrap();
        (path, chunk.payload)
    }
}

#[cfg(feature = "sila2")]
fn stamp_text(stamp: &a2a_lab_dev_kit::sila::api::Timestamp) -> String {
    assert_eq!(stamp.millisecond, 0, "{stamp:?}");
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        stamp.year, stamp.month, stamp.day, stamp.hour, stamp.minute, stamp.second
    )
}

#[cfg(feature = "sila2")]
fn range_text(range: &a2a_lab_dev_kit::sila::api::VideoTimeRange) -> (String, String) {
    let inner = range.time_range.as_ref().unwrap();
    (
        stamp_text(inner.start.as_ref().unwrap()),
        stamp_text(inner.end.as_ref().unwrap()),
    )
}

#[cfg(feature = "sila2")]
fn decoded(status: &tonic::Status) -> String {
    String::from_utf8_lossy(
        &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, status.message())
            .unwrap_or_default(),
    )
    .into_owned()
}

#[cfg(feature = "sila2")]
fn sila_defined_name(status: &tonic::Status) -> String {
    let text = decoded(status);
    let names = [
        "NoRecording",
        "RecordingGap",
        "ClipExpired",
        "LiveUnavailable",
        "VideoSourceNotFound",
        "VideoFrameNotFound",
        "VideoClipNotFound",
        "ProviderUnavailable",
    ];
    let found: Vec<_> = names
        .into_iter()
        .filter(|name| text.contains(&format!("DefinedExecutionError/{name}")))
        .collect();
    if found.len() != 1 {
        diverge("defined error", "SiLA", &text);
    }
    found[0].to_owned()
}

#[cfg(feature = "sila2")]
fn clip_params(
    source: &str,
    start: a2a_lab_dev_kit::sila::api::Timestamp,
    end: a2a_lab_dev_kit::sila::api::Timestamp,
) -> a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
    use a2a_lab_dev_kit::sila::api::{
        GetVideoClipParameters, SilaString, VideoTimeRange, VideoTimeStruct,
    };

    GetVideoClipParameters {
        source_id: Some(SilaString {
            value: source.to_owned(),
        }),
        range: Some(VideoTimeRange {
            time_range: Some(VideoTimeStruct {
                start: Some(start),
                end: Some(end),
            }),
        }),
    }
}

#[cfg(feature = "sila2")]
fn frame_params(
    source: &str,
    at: a2a_lab_dev_kit::sila::api::Timestamp,
) -> a2a_lab_dev_kit::sila::api::GetVideoFrameAtParameters {
    use a2a_lab_dev_kit::sila::api::{Boolean, GetVideoFrameAtParameters, Integer, SilaString};

    GetVideoFrameAtParameters {
        source_id: Some(SilaString {
            value: source.to_owned(),
        }),
        at: Some(at),
        has_max_delta_us: Some(Boolean { value: false }),
        max_delta_us: Some(Integer { value: 0 }),
    }
}

#[cfg(feature = "sila2")]
fn sila_utc(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> a2a_lab_dev_kit::sila::api::Timestamp {
    a2a_lab_dev_kit::sila::api::Timestamp {
        year,
        month,
        day,
        hour,
        minute,
        second,
        millisecond: 0,
        timezone: Some(a2a_lab_dev_kit::sila::api::Timezone {
            hours: 0,
            minutes: 0,
        }),
    }
}

#[cfg(feature = "sila2")]
fn sila_ms(minute: u32, second: u32, millisecond: u32) -> a2a_lab_dev_kit::sila::api::Timestamp {
    let mut stamp = sila_utc(2024, 1, 1, 0, minute, second);
    stamp.millisecond = millisecond;
    stamp
}

async fn follow_mcp(lab: &McpLab, source: &str) -> VideoFollow {
    let outcome = lab
        .execute(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            source_id(source),
        )))
        .await
        .unwrap();
    let A2aLabResult::FollowVideo(follow) = outcome.task.result else {
        diverge("follow_video", "MCP", "unexpected result");
    };
    follow
}

async fn a2a_result(client: &A2aClient, command: A2aLabCommand) -> Result<(), A2aLabError> {
    match command {
        A2aLabCommand::ListVideoSources(request) => {
            client.list_video_sources(request).await.map(|_| ())
        }
        A2aLabCommand::GetVideoFrameAt(request) => {
            client.get_video_frame_at(request).await.map(|_| ())
        }
        A2aLabCommand::GetVideoClip(request) => client.get_video_clip(request).await.map(|_| ()),
        A2aLabCommand::GetVideoTimeline(request) => {
            client.get_video_timeline(request).await.map(|_| ())
        }
        A2aLabCommand::FollowVideo(request) => client.follow_video(request).await.map(|_| ()),
        _ => diverge("command", "A2A", "unsupported"),
    }
}

async fn expect_a2a_video(
    case: &str,
    base: &str,
    client: &A2aClient,
    command: A2aLabCommand,
    expected: &str,
) {
    let typed = a2a_result(client, command.clone()).await.unwrap_err();
    expect_code(case, "A2A", typed.code(), expected);
    let wire = a2a_video_code(case, &send_lab(base, &command).await);
    expect_code(case, "A2A", &wire, expected);
}

async fn expect_a2a_status(
    case: &str,
    base: &str,
    client: &A2aClient,
    command: A2aLabCommand,
    expected: &str,
) {
    let typed = a2a_result(client, command.clone()).await.unwrap_err();
    expect_code(case, "A2A", typed.code(), expected);
    let body = send_lab(base, &command).await;
    if metadata_code(&body).is_some() {
        diverge(case, "A2A", &body.to_string());
    }
}

fn expect_media(
    case: &str,
    transport: &str,
    status: u16,
    code: &str,
    message: &str,
    expected_status: u16,
    expected_code: &str,
) {
    if status != expected_status || code != expected_code || message == code || code.is_empty() {
        diverge(
            case,
            transport,
            &format!("status={status} error.code={code} message={message}"),
        );
    }
}

async fn expect_mcp(case: &str, url: &str, lab: &McpLab, command: A2aLabCommand, expected: &str) {
    let typed = lab.execute(command.clone()).await.unwrap_err();
    expect_code(case, "MCP", typed.code(), expected);
    let (code, message) = mcp_data_code(url, &command).await;
    if code != expected || message == code || code.is_empty() {
        diverge(case, "MCP", &format!("data.code={code} message={message}"));
    }
}
