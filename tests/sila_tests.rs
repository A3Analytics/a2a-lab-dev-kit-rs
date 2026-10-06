#![cfg(feature = "sila2")]

use std::sync::Arc;

use a2a_lab_dev_kit::sila::api::{
    Boolean, CancelAllParameters, CancelCommandParameters, CancelControllerClient,
    CommandExecutionUuid, ConnectSiLaClientParameters, ConnectionConfigurationServiceClient,
    DataTypePageRequest, DataTypeTimeRange, DataTypeUuid,
    EnableServerInitiatedConnectionModeParameters, GetFeatureDefinitionParameters,
    GetImplementedFeaturesParameters, GetServerNameParameters, GetServerVersionParameters,
    GetTaskStatusParameters, Integer, LabOperationsClient, ListLogSourcesParameters,
    ListMetricsParameters, ListTasksParameters, PageRequestStruct, QueryLogsParameters,
    QueryMetricParameters, SetServerNameParameters, SiLaServiceClient, SilaString,
    StartTaskParameters, TimeRangeStruct, Timestamp,
};
use a2a_lab_dev_kit::sila::{
    SilaCertificate, SilaIdentity, SilaServer, certificate_matches_profile,
};
use a2a_lab_dev_kit::{
    A2aLabService, JsonObject, LogLevel, LogRecord, LogSource, MemoryLogs, MemoryMetrics,
    MemoryTasks, MetricDescriptor, MetricId, MetricPoint, SourceId, TaskDefinition, TaskId,
    UtcTimestamp,
};
use tonic::transport::Channel;

#[test]
fn identity_rejects_a_non_uuid_and_certificates_carry_the_uuid() {
    assert!(SilaIdentity::lab_dev_kit("not-a-uuid").is_err());
    let uuid = "11111111-1111-1111-1111-111111111111";
    let identity = SilaIdentity::lab_dev_kit(uuid).unwrap();
    assert_eq!(identity.server_type, "LabDevKit");
    let certificate = SilaCertificate::self_signed(uuid).unwrap();
    assert!(certificate_matches_profile(&certificate.cert_pem, uuid));
    assert!(!certificate_matches_profile(
        &certificate.cert_pem,
        "22222222-2222-2222-2222-222222222222"
    ));
    assert!(SilaIdentity::new(uuid, "", "LabDevKit", "desc", "1.2.3_lab", "http://a",).is_ok());
    assert!(SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-11111111111g").is_err());
}

#[tokio::test]
async fn plaintext_server_lists_starts_and_cancels_a_task() {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("mix").unwrap(),
            name: "Mix".to_owned(),
            description: "Mix the plate".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let server = SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        Arc::clone(&lab),
    )
    .plaintext()
    .serve("127.0.0.1:0".parse().unwrap())
    .await
    .unwrap();
    let channel = Channel::from_shared(format!("http://{}", server.local_addr()))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = LabOperationsClient::new(channel.clone());
    let listed = client
        .list_tasks(ListTasksParameters {
            page: Some(page(10)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.tasks.len(), 1);

    let started = client
        .start_task(StartTaskParameters {
            task_id: Some(SilaString {
                value: "mix".to_owned(),
            }),
            input: Some(SilaString {
                value: r#"{"complete":false}"#.to_owned(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let execution = started.command_execution_uuid.unwrap().value;
    let mut info = client
        .start_task_info(CommandExecutionUuid {
            value: execution.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let first = info.message().await.unwrap().unwrap();
    assert_eq!(first.command_status, 0);
    assert_eq!(first.updated_lifetime_of_execution.unwrap().seconds, 60);

    let mut cancel = CancelControllerClient::new(channel);
    cancel
        .cancel_command(CancelCommandParameters {
            command_execution_uuid: Some(DataTypeUuid {
                uuid: Some(SilaString {
                    value: execution.clone(),
                }),
            }),
        })
        .await
        .unwrap();
    let finished = info.message().await.unwrap().unwrap();
    assert_eq!(finished.command_status, 3);
    assert!(
        client
            .start_task_result(CommandExecutionUuid { value: execution })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn service_identity_and_feature_definition_follow_the_constraints() {
    let server = plain_server(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let mut service = SiLaServiceClient::new(channel(&server).await);
    let version = service
        .get_server_version(GetServerVersionParameters {})
        .await
        .unwrap()
        .into_inner()
        .server_version
        .unwrap()
        .value;
    assert_eq!(version, "0.1.0");
    service
        .set_server_name(SetServerNameParameters {
            server_name: Some(SilaString {
                value: "bench".to_owned(),
            }),
        })
        .await
        .unwrap();
    let name = service
        .get_server_name(GetServerNameParameters {})
        .await
        .unwrap()
        .into_inner()
        .server_name
        .unwrap()
        .value;
    assert_eq!(name, "bench");
    let malformed = service
        .get_feature_definition(GetFeatureDefinitionParameters {
            feature_identifier: Some(SilaString {
                value: "SiLAService".to_owned(),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(malformed.code(), tonic::Code::Aborted);
    let missing = service
        .get_feature_definition(GetFeatureDefinitionParameters {
            feature_identifier: Some(SilaString {
                value: "org.silastandard/core/MissingFeature/v1".to_owned(),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), tonic::Code::Aborted);
    let features = service
        .get_implemented_features(GetImplementedFeaturesParameters {})
        .await
        .unwrap()
        .into_inner()
        .implemented_features;
    assert!(
        features
            .iter()
            .any(|feature| feature.value.contains("SiLAService"))
    );
    assert!(
        features
            .iter()
            .all(|feature| !feature.value.contains("ConnectionConfigurationService"))
    );
}

#[tokio::test]
async fn lab_queries_return_records_and_reject_missing_ids() {
    let (logs, metrics) = demo_observations().await;
    let server = plain_server(A2aLabService::new(logs, metrics, MemoryTasks::new()).share()).await;
    let mut lab = LabOperationsClient::new(channel(&server).await);
    assert_eq!(
        lab.list_log_sources(ListLogSourcesParameters {
            page: Some(page(10))
        })
        .await
        .unwrap()
        .into_inner()
        .sources
        .len(),
        1
    );
    assert_eq!(
        lab.query_logs(QueryLogsParameters {
            source_id: Some(SilaString {
                value: "journal".to_owned()
            }),
            range: Some(span()),
            page: Some(page(10)),
        })
        .await
        .unwrap()
        .into_inner()
        .records
        .len(),
        1
    );
    assert_eq!(
        lab.list_metrics(ListMetricsParameters {
            page: Some(page(10))
        })
        .await
        .unwrap()
        .into_inner()
        .metrics
        .len(),
        1
    );
    assert_eq!(
        lab.query_metric(QueryMetricParameters {
            metric_id: Some(SilaString {
                value: "temp".to_owned()
            }),
            range: Some(span()),
            page: Some(page(10)),
        })
        .await
        .unwrap()
        .into_inner()
        .points
        .len(),
        1
    );
    assert!(
        lab.get_task_status(GetTaskStatusParameters {
            run_id: Some(SilaString {
                value: "missing".to_owned()
            }),
        })
        .await
        .is_err()
    );
    assert!(
        lab.query_logs(QueryLogsParameters {
            source_id: Some(SilaString {
                value: "missing".to_owned()
            }),
            range: Some(span()),
            page: Some(page(10)),
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn connection_configuration_rejects_an_unusable_client() {
    let path = std::env::temp_dir().join(format!("sila-reject-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let server = SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .plaintext()
    .cloud_plaintext()
    .connection_store(path.clone())
    .serve("127.0.0.1:0".parse().unwrap())
    .await
    .unwrap();
    let mut service = SiLaServiceClient::new(channel(&server).await);
    let features = service
        .get_implemented_features(GetImplementedFeaturesParameters {})
        .await
        .unwrap()
        .into_inner()
        .implemented_features;
    assert!(
        features
            .iter()
            .any(|feature| feature.value.contains("ConnectionConfigurationService"))
    );
    let mut connection = ConnectionConfigurationServiceClient::new(channel(&server).await);
    connection
        .enable_server_initiated_connection_mode(EnableServerInitiatedConnectionModeParameters {})
        .await
        .unwrap();
    let rejected = connection
        .connect_si_la_client(ConnectSiLaClientParameters {
            client_name: Some(SilaString {
                value: String::new(),
            }),
            si_la_client_host: Some(SilaString {
                value: "127.0.0.1".to_owned(),
            }),
            si_la_client_port: Some(Integer { value: 1 }),
            persist: Some(Boolean { value: false }),
        })
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), tonic::Code::Aborted);
    let mut cancel = CancelControllerClient::new(channel(&server).await);
    cancel.cancel_all(CancelAllParameters {}).await.unwrap();
    let malformed = cancel
        .cancel_command(CancelCommandParameters {
            command_execution_uuid: Some(DataTypeUuid {
                uuid: Some(SilaString {
                    value: "not-a-uuid".to_owned(),
                }),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(malformed.code(), tonic::Code::Aborted);
    drop(server);
    let _ = std::fs::remove_file(path);
}

async fn demo_observations() -> (MemoryLogs, MemoryMetrics) {
    let logs = MemoryLogs::new();
    let metrics = MemoryMetrics::new();
    logs.insert_source(LogSource {
        id: SourceId::new("journal").unwrap(),
        name: "Journal".to_owned(),
        description: "Demo".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_record(LogRecord {
        source_id: SourceId::new("journal").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap(),
        level: LogLevel::Info,
        message: "ready".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await
    .unwrap();
    metrics
        .insert_metric(MetricDescriptor {
            id: MetricId::new("temp").unwrap(),
            name: "Temperature".to_owned(),
            description: "Demo".to_owned(),
            unit: "C".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            MetricId::new("temp").unwrap(),
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap(), 21.5).unwrap(),
        )
        .await
        .unwrap();
    (logs, metrics)
}

async fn plain_server(
    lab: Arc<dyn a2a_lab_dev_kit::A2aLabApi>,
) -> a2a_lab_dev_kit::sila::SilaServerHandle {
    SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        lab,
    )
    .plaintext()
    .serve("127.0.0.1:0".parse().unwrap())
    .await
    .unwrap()
}

async fn channel(server: &a2a_lab_dev_kit::sila::SilaServerHandle) -> Channel {
    Channel::from_shared(format!("http://{}", server.local_addr()))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

fn span() -> DataTypeTimeRange {
    DataTypeTimeRange {
        time_range: Some(TimeRangeStruct {
            start: Some(stamp(2024, 1, 1)),
            end: Some(stamp(2024, 1, 2)),
        }),
    }
}

fn stamp(year: u32, month: u32, day: u32) -> Timestamp {
    Timestamp {
        second: 0,
        minute: 0,
        hour: 0,
        day,
        month,
        year,
        timezone: None,
        millisecond: 0,
    }
}

fn page(limit: i64) -> DataTypePageRequest {
    DataTypePageRequest {
        page_request: Some(PageRequestStruct {
            has_cursor: Some(Boolean { value: false }),
            cursor: Some(SilaString {
                value: String::new(),
            }),
            limit: Some(Integer { value: limit }),
        }),
    }
}
