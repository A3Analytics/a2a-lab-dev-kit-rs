//! SiLA 2 Feature Provider for the equipment represented by this devkit.
//!
//! `SilaServer` serves `SiLAService`, `LabOperations`, and `CancelController` over the same
//! [`A2aLabApi`](crate::service::A2aLabApi) used by A2A and MCP.

mod cancel;
mod cert;
mod cloud;
mod connection;
mod constraints;
mod core;
mod discover;
mod errors;
mod executions;
mod identity;
mod lab;
mod server;
mod values;
mod wire;

pub use cert::{SilaCertificate, certificate_contains_uuid, certificate_matches_profile};
pub use identity::SilaIdentity;
pub use server::{SilaServer, SilaServerHandle};

/// Generated SiLA service clients and messages.
pub mod api {
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
    pub use crate::sila::wire::sila2::org::silastandard::{
        Boolean, CommandExecutionUuid, Integer, String as SilaString, Timestamp,
    };
}
