//! SiLA 2 interface and remote provider.
//!
//! `SilaServer` serves `SiLAService`, `LabOperations`, `LabImages`, `BinaryDownload`, and
//! `CancelController` over the same [`A2aLabApi`](crate::service::A2aLabApi) used by A2A and MCP.
//! Image pixel bytes always use binary download. `SilaProvider` connects to a remote SiLA server
//! and fulfills logs, metrics, and tasks.

mod binary;
mod cancel;
mod cert;
mod cloud;
mod connection;
mod constraints;
mod consumer;
mod core;
mod discover;
mod errors;
mod executions;
mod identity;
mod images;
mod lab;
mod server;
mod values;
mod wire;

pub use cert::{SilaCertificate, certificate_contains_uuid, certificate_matches_profile};
pub use consumer::{
    LogBinding, MemberKind, MetricBinding, RequestBinding, SILA_SERVICE, SilaBinding, SilaDevice,
    SilaEndpoint, SilaMember, SilaProvider, SilaProviderConfig, SilaSession, SilaTasks, SilaTrust,
    TaskBinding,
};
pub use identity::SilaIdentity;
pub use server::{SilaServer, SilaServerHandle};

/// Generated SiLA service clients and messages.
pub mod api {
    pub use crate::sila::wire::sila2::com::a3analytics::lab::labimages::v1::{
        DataTypePageRequest as ImagePageRequest, GetCurrentImageParameters, GetImageParameters,
        ListImageSourcesParameters, data_type_page_request::PageRequestStruct as ImagePageStruct,
        lab_images_client::LabImagesClient,
    };
    pub use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::{
        DataTypePageRequest, DataTypeTimeRange, GetTaskStatusParameters, ListLogSourcesParameters,
        ListMetricsParameters, ListTasksParameters, QueryLogsParameters, QueryMetricParameters,
        StartTaskParameters, data_type_page_request::PageRequestStruct,
        data_type_time_range::TimeRangeStruct, lab_operations_client::LabOperationsClient,
    };
    pub use crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::{
        CancelAllParameters, CancelCommandParameters, DataTypeUuid,
        cancel_controller_client::CancelControllerClient,
    };
    pub use crate::sila::wire::sila2::org::silastandard::core::connectionconfigurationservice::v1::{
        ConnectSiLaClientParameters, EnableServerInitiatedConnectionModeParameters,
        GetServerInitiatedConnectionModeStatusParameters,
        connection_configuration_service_client::ConnectionConfigurationServiceClient,
    };
    pub use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::{
        GetFeatureDefinitionParameters, GetImplementedFeaturesParameters, GetServerNameParameters,
        GetServerVersionParameters, SetServerNameParameters,
        si_la_service_client::SiLaServiceClient,
    };
    pub use crate::sila::wire::sila2::org::silastandard::binary::Union as BinaryUnion;
    pub use crate::sila::wire::sila2::org::silastandard::binary_download_client::BinaryDownloadClient;
    pub use crate::sila::wire::sila2::org::silastandard::{
        Binary, Boolean, CommandExecutionUuid, DeleteBinaryRequest, GetBinaryInfoRequest,
        GetChunkRequest, Integer, String as SilaString, Timestamp,
    };
}
