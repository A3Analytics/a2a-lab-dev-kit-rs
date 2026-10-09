use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabResult, A2aLabService, GetCurrentImageRequest, GetImageRequest,
    GetTaskStatusRequest, Image, ImageDescriptor, ImageId, ImageProvider, ImageSource,
    ImageSourceId, ImageTransportConfig, JsonObject, ListImageSourcesRequest, ListImagesRequest,
    ListLogSourcesRequest, ListMetricsRequest, ListTasksRequest, LogLevel, LogProvider, LogRecord,
    LogSource, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricId,
    MetricPoint, MetricProvider, PageRequest, QueryLogsRequest, QueryMetricRequest,
    SearchImagesRequest, SourceId, StartTaskRequest, TaskDefinition, TaskId, TaskProvider,
    TaskState, TimeRange, UtcTimestamp,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn source(id: &str) -> LogSource {
    LogSource {
        id: SourceId::new(id).unwrap(),
        name: id.to_owned(),
        description: format!("{id} logs"),
        asset_id: None,
        semantic_id: None,
    }
}

#[tokio::test]
async fn paginates_log_sources_and_queries_a_half_open_range() {
    let logs = MemoryLogs::new();
    logs.insert_source(source("beta")).await;
    logs.insert_source(source("alpha")).await;
    let first = logs
        .list_sources(ListLogSourcesRequest { page: page(1) })
        .await
        .unwrap();
    assert_eq!(first.items()[0].id.as_str(), "alpha");
    let second = logs
        .list_sources(ListLogSourcesRequest {
            page: PageRequest::new(first.next_cursor().map(str::to_owned), 1).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(second.items()[0].id.as_str(), "beta");
    assert!(second.next_cursor().is_none());

    let source_id = SourceId::new("alpha").unwrap();
    for (stamp, message) in [
        ("2024-01-01T00:00:00Z", "start"),
        ("2024-01-01T00:30:00Z", "middle"),
        ("2024-01-01T01:00:00Z", "end"),
    ] {
        logs.insert_record(LogRecord {
            source_id: source_id.clone(),
            timestamp: timestamp(stamp),
            level: LogLevel::Info,
            message: message.to_owned(),
            attributes: JsonObject::empty(),
        })
        .await
        .unwrap();
    }
    let queried = logs
        .query(QueryLogsRequest {
            source_id: source_id.clone(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert_eq!(
        queried
            .items()
            .iter()
            .map(|record| record.message.as_str())
            .collect::<Vec<_>>(),
        ["start", "middle"]
    );
    let missing = logs
        .query(QueryLogsRequest {
            source_id: SourceId::new("missing").unwrap(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}

#[tokio::test]
async fn queries_metric_samples_and_reports_provider_failure() {
    let metrics = MemoryMetrics::new();
    let metric_id = MetricId::new("latency").unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: metric_id.clone(),
            name: "Latency".to_owned(),
            description: "Request latency".to_owned(),
            unit: "ms".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            metric_id.clone(),
            MetricPoint::new(timestamp("2024-01-01T00:00:00Z"), 5.0).unwrap(),
        )
        .await
        .unwrap();
    let listed = metrics
        .list_metrics(ListMetricsRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(listed.items().len(), 1);
    let queried = metrics
        .query(QueryMetricRequest {
            metric_id: metric_id.clone(),
            range: range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"),
            page: page(10),
        })
        .await
        .unwrap();
    assert!((queried.items()[0].value - 5.0).abs() < f64::EPSILON);
    metrics.set_unavailable("metrics offline").await;
    let error = metrics
        .list_metrics(ListMetricsRequest { page: page(10) })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unavailable");
}

#[tokio::test]
async fn starts_a_task_and_tracks_its_status() {
    let tasks = MemoryTasks::new();
    let task_id = TaskId::new("build").unwrap();
    tasks
        .insert(TaskDefinition {
            id: task_id.clone(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let listed = tasks
        .list_tasks(ListTasksRequest { page: page(10) })
        .await
        .unwrap();
    assert_eq!(listed.items()[0].id, task_id);
    let run = tasks
        .start(StartTaskRequest::new(
            task_id.clone(),
            JsonObject::parse(r#"{"branch":"main"}"#).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(run.state, TaskState::Submitted);
    tasks
        .transition(&run.id, TaskState::Completed, Some("done".to_owned()))
        .await
        .unwrap();
    let status = tasks
        .status(GetTaskStatusRequest { id: run.id.clone() })
        .await
        .unwrap();
    assert_eq!(status.state, TaskState::Completed);
    assert_eq!(status.message.as_deref(), Some("done"));
    let missing = tasks
        .start(StartTaskRequest::new(
            TaskId::new("missing").unwrap(),
            JsonObject::empty(),
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}

#[tokio::test]
async fn three_provider_constructor_leaves_images_unavailable_until_with_images() {
    let logs = MemoryLogs::new();
    logs.insert_source(source("app")).await;
    let service = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new());
    let listed = service
        .execute(A2aLabCommand::ListLogSources(ListLogSourcesRequest {
            page: page(10),
        }))
        .await
        .unwrap();
    let A2aLabResult::ListLogSources(sources) = listed.task.result else {
        panic!("log listing must keep working");
    };
    assert_eq!(sources.items()[0].id.as_str(), "app");

    for command in image_commands() {
        let error = service.execute(command).await.unwrap_err();
        assert_eq!(error.code(), "unavailable");
    }

    let images = MemoryImages::new();
    images.insert_source(image_source("cam")).await.unwrap();
    let enabled = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_image_transport(ImageTransportConfig::new(8).unwrap())
        .with_images(images);
    let outcome = enabled
        .execute(A2aLabCommand::ListImageSources(
            ListImageSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::ListImageSources(page) = outcome.task.result else {
        panic!("with_images must enable source listing");
    };
    assert_eq!(page.items()[0].id.as_str(), "cam");
}

#[tokio::test]
async fn service_enforces_payload_limit_and_retains_image_snapshots() {
    let images = sample_images().await;
    let tight = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images.clone())
        .with_image_transport(ImageTransportConfig::new(3).unwrap());
    let rejected = tight
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(image_id("a"))))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), "invalid");
    let rejected_current = tight
        .execute(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
            image_source_id("alpha"),
        )))
        .await
        .unwrap_err();
    assert_eq!(rejected_current.code(), "invalid");
    assert_eq!(tight.task("task-1").await.unwrap_err().code(), "not_found");

    let listed = tight
        .execute(A2aLabCommand::ListImages(
            ListImagesRequest::new(image_source_id("alpha"), page(10)).unwrap(),
        ))
        .await
        .unwrap();
    assert_metadata_only(&serde_json::to_value(&listed.task.result).unwrap());
    let searched = tight
        .execute(A2aLabCommand::SearchImages(
            SearchImagesRequest::new(None, None, Some("plate".to_owned()), page(10)).unwrap(),
        ))
        .await
        .unwrap();
    assert_metadata_only(&serde_json::to_value(&searched.task.result).unwrap());

    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_image_transport(ImageTransportConfig::new(4).unwrap())
        .with_images(images);
    let image = service
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(image_id("a"))))
        .await
        .unwrap();
    let A2aLabResult::GetImage(frame) = &image.task.result else {
        panic!("get_image must return an image result");
    };
    assert_eq!(frame.data(), [1, 2, 3, 4]);
    assert_eq!(service.task(&image.task.id).await.unwrap(), image.task);

    let current = service
        .execute(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
            image_source_id("alpha"),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetCurrentImage(frame) = &current.task.result else {
        panic!("get_current_image must return an image result");
    };
    assert_eq!(frame.descriptor().id().as_str(), "c");
    assert_eq!(service.task(&current.task.id).await.unwrap(), current.task);
}

#[tokio::test]
async fn memory_images_page_filter_and_select_the_current_frame() {
    let images = sample_images().await;
    let first = images
        .list_image_sources(ListImageSourcesRequest::new(page(1)).unwrap())
        .await
        .unwrap();
    assert_eq!(first.items()[0].id.as_str(), "alpha");
    let second = images
        .list_image_sources(
            ListImageSourcesRequest::new(
                PageRequest::new(first.next_cursor().map(str::to_owned), 1).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.items()[0].id.as_str(), "zeta");
    assert!(second.next_cursor().is_none());

    let listed = images
        .list_images(ListImagesRequest::new(image_source_id("alpha"), page(10)).unwrap())
        .await
        .unwrap();
    assert_eq!(ids(listed.items()), ["a", "b", "c"]);
    let paged = images
        .list_images(ListImagesRequest::new(image_source_id("alpha"), page(1)).unwrap())
        .await
        .unwrap();
    assert_eq!(paged.items()[0].id().as_str(), "a");
    assert!(paged.next_cursor().is_some());

    let range_only = images
        .search_images(
            SearchImagesRequest::new(
                Some(image_source_id("alpha")),
                Some(range("2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z")),
                None,
                page(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(range_only.items()), ["a", "b"]);

    let text_only = images
        .search_images(
            SearchImagesRequest::new(None, None, Some("WELL".to_owned()), page(10)).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(text_only.items()), ["b", "c"]);

    let combined = images
        .search_images(
            SearchImagesRequest::new(
                Some(image_source_id("alpha")),
                Some(range("2024-01-01T00:00:00Z", "2024-01-01T00:30:00Z")),
                Some("plat".to_owned()),
                page(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(combined.items()), ["a"]);

    let tied = images
        .search_images(
            SearchImagesRequest::new(
                Some(image_source_id("zeta")),
                Some(range("2024-01-01T03:00:00Z", "2024-01-01T04:00:00Z")),
                None,
                page(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(tied.items()), ["m", "z"]);

    let current = images
        .get_current_image(GetCurrentImageRequest::new(image_source_id("zeta")))
        .await
        .unwrap();
    assert_eq!(current.descriptor().id().as_str(), "z");
    let latest = images
        .get_current_image(GetCurrentImageRequest::new(image_source_id("alpha")))
        .await
        .unwrap();
    assert_eq!(latest.descriptor().id().as_str(), "c");
    images
        .set_current(image_source_id("alpha"), image_id("a"))
        .await
        .unwrap();
    let selected = images
        .get_current_image(GetCurrentImageRequest::new(image_source_id("alpha")))
        .await
        .unwrap();
    assert_eq!(selected.descriptor().id().as_str(), "a");
    assert_eq!(selected.data(), [1, 2, 3, 4]);
    let specific = images
        .get_image(GetImageRequest::new(image_id("b")))
        .await
        .unwrap();
    assert_eq!(specific.descriptor().source_id().as_str(), "alpha");
    assert_eq!(specific.data(), [1, 2, 3, 4]);
}

#[tokio::test]
async fn memory_images_report_missing_resources_and_unavailability() {
    let images = sample_images().await;
    let missing_image = images
        .get_image(GetImageRequest::new(image_id("missing")))
        .await
        .unwrap_err();
    assert_eq!(missing_image.code(), "not_found");
    let missing_source = images
        .list_images(ListImagesRequest::new(image_source_id("missing"), page(10)).unwrap())
        .await
        .unwrap_err();
    assert_eq!(missing_source.code(), "not_found");
    let missing_search = images
        .search_images(
            SearchImagesRequest::new(
                Some(image_source_id("missing")),
                None,
                Some("plate".to_owned()),
                page(10),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing_search.code(), "not_found");
    let missing_current = images
        .get_current_image(GetCurrentImageRequest::new(image_source_id("missing")))
        .await
        .unwrap_err();
    assert_eq!(missing_current.code(), "not_found");
    let unrelated = images
        .set_current(image_source_id("alpha"), image_id("z"))
        .await
        .unwrap_err();
    assert_eq!(unrelated.code(), "invalid");
    let orphan = Image::new(
        ImageDescriptor::new(
            image_id("orphan"),
            image_source_id("missing"),
            timestamp("2024-01-01T00:00:00Z"),
            "image/png",
            1,
            1,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        vec![1, 2, 3, 4],
    )
    .unwrap();
    assert_eq!(
        images.insert_image(orphan).await.unwrap_err().code(),
        "not_found"
    );
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images.clone());
    let missing = service
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(image_id(
            "missing",
        ))))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");

    images.set_unavailable("images offline").await;
    for command in image_commands() {
        let error = dispatch(&images, command).await.unwrap_err();
        assert_eq!(error.code(), "unavailable");
        assert!(error.to_string().contains("images offline"));
    }
    for command in image_commands() {
        let error = service.execute(command).await.unwrap_err();
        assert_eq!(error.code(), "unavailable");
    }
    images.clear_unavailable().await;
    assert!(
        images
            .list_image_sources(ListImageSourcesRequest::new(page(10)).unwrap())
            .await
            .is_ok()
    );
}

fn image_commands() -> Vec<A2aLabCommand> {
    vec![
        A2aLabCommand::ListImageSources(ListImageSourcesRequest::new(page(10)).unwrap()),
        A2aLabCommand::ListImages(
            ListImagesRequest::new(image_source_id("alpha"), page(10)).unwrap(),
        ),
        A2aLabCommand::SearchImages(
            SearchImagesRequest::new(None, None, Some("plate".to_owned()), page(10)).unwrap(),
        ),
        A2aLabCommand::GetImage(GetImageRequest::new(image_id("a"))),
        A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(image_source_id("alpha"))),
    ]
}

async fn dispatch(
    images: &MemoryImages,
    command: A2aLabCommand,
) -> Result<(), a2a_lab_dev_kit::A2aLabError> {
    match command {
        A2aLabCommand::ListImageSources(request) => {
            images.list_image_sources(request).await.map(|_| ())
        }
        A2aLabCommand::ListImages(request) => images.list_images(request).await.map(|_| ()),
        A2aLabCommand::SearchImages(request) => images.search_images(request).await.map(|_| ()),
        A2aLabCommand::GetImage(request) => images.get_image(request).await.map(|_| ()),
        A2aLabCommand::GetCurrentImage(request) => {
            images.get_current_image(request).await.map(|_| ())
        }
        _ => unreachable!(),
    }
}

async fn sample_images() -> MemoryImages {
    let images = MemoryImages::new();
    images.insert_source(image_source("zeta")).await.unwrap();
    images.insert_source(image_source("alpha")).await.unwrap();
    for (id, source, stamp, caption) in [
        ("c", "alpha", "2024-01-01T01:00:00Z", Some("Well")),
        ("a", "alpha", "2024-01-01T00:00:00Z", Some("Plate")),
        ("b", "alpha", "2024-01-01T00:30:00Z", Some("plate well")),
        ("z", "zeta", "2024-01-01T03:00:00Z", Some("Other plate")),
        ("m", "zeta", "2024-01-01T03:00:00Z", Some("other")),
    ] {
        let frame = stored_image(id, source, stamp, caption);
        let decoded: Image = serde_json::from_str(&serde_json::to_string(&frame).unwrap()).unwrap();
        images.insert_image(decoded).await.unwrap();
    }
    images
}

fn stored_image(id: &str, source: &str, stamp: &str, caption: Option<&str>) -> Image {
    Image::new(
        ImageDescriptor::new(
            image_id(id),
            image_source_id(source),
            timestamp(stamp),
            "image/png",
            1,
            1,
            caption.map(str::to_owned),
            JsonObject::empty(),
        )
        .unwrap(),
        vec![1, 2, 3, 4],
    )
    .unwrap()
}

fn image_source(id: &str) -> ImageSource {
    ImageSource {
        id: image_source_id(id),
        name: id.to_owned(),
        description: format!("{id} frames"),
        asset_id: None,
        semantic_id: None,
    }
}

fn image_id(id: &str) -> ImageId {
    ImageId::new(id).unwrap()
}

fn image_source_id(id: &str) -> ImageSourceId {
    ImageSourceId::new(id).unwrap()
}

fn ids(descriptors: &[ImageDescriptor]) -> Vec<&str> {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id().as_str())
        .collect()
}

fn assert_metadata_only(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            assert!(!map.contains_key("data"), "{value}");
            for child in map.values() {
                assert_metadata_only(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                assert_metadata_only(item);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}
