//! Plaintext gRPC server for the communication tester.

use std::net::SocketAddr;
use std::sync::Arc;

use tonic::transport::Server;

use crate::auth::{Authentication, Authorization, Test as AuthenticationTests};
use crate::basic::Feature as Basic;
use crate::binary::{DownloadService, TestFeature as BinaryTests, UploadService};
use crate::error_handling::Feature as ErrorHandling;
use crate::lists::Feature as Lists;
use crate::metadata_test::{Consumer, Provider};
use crate::multi_client::Feature as MultiClient;
use crate::observable_command::Feature as ObservableCommands;
use crate::observable_property::Feature as ObservableProperties;
use crate::shared::Shared;
use crate::sila_service::Service as SilaService;
use crate::structures::Feature as Structures;
use crate::unobservable::{Commands, Properties};
use crate::any_type::Feature as AnyTypes;
use crate::wire::sila2::org::silastandard::binary_download_server::BinaryDownloadServer;
use crate::wire::sila2::org::silastandard::binary_upload_server::BinaryUploadServer;
use crate::wire::sila2::org::silastandard::core::authenticationservice::v1::authentication_service_server::AuthenticationServiceServer;
use crate::wire::sila2::org::silastandard::core::authorizationservice::v1::authorization_service_server::AuthorizationServiceServer;
use crate::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaServiceServer;
use crate::wire::sila2::org::silastandard::test::anytypetest::v1::any_type_test_server::AnyTypeTestServer;
use crate::wire::sila2::org::silastandard::test::authenticationtest::v1::authentication_test_server::AuthenticationTestServer;
use crate::wire::sila2::org::silastandard::test::basicdatatypestest::v1::basic_data_types_test_server::BasicDataTypesTestServer;
use crate::wire::sila2::org::silastandard::test::binarytransfertest::v1::binary_transfer_test_server::BinaryTransferTestServer;
use crate::wire::sila2::org::silastandard::test::errorhandlingtest::v1::error_handling_test_server::ErrorHandlingTestServer;
use crate::wire::sila2::org::silastandard::test::listdatatypetest::v1::list_data_type_test_server::ListDataTypeTestServer;
use crate::wire::sila2::org::silastandard::test::metadataconsumertest::v1::metadata_consumer_test_server::MetadataConsumerTestServer;
use crate::wire::sila2::org::silastandard::test::metadataprovider::v1::metadata_provider_server::MetadataProviderServer;
use crate::wire::sila2::org::silastandard::test::multiclienttest::v1::multi_client_test_server::MultiClientTestServer;
use crate::wire::sila2::org::silastandard::test::observablecommandtest::v1::observable_command_test_server::ObservableCommandTestServer;
use crate::wire::sila2::org::silastandard::test::observablepropertytest::v1::observable_property_test_server::ObservablePropertyTestServer;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::structure_data_type_test_server::StructureDataTypeTestServer;
use crate::wire::sila2::org::silastandard::test::unobservablecommandtest::v1::unobservable_command_test_server::UnobservableCommandTestServer;
use crate::wire::sila2::org::silastandard::test::unobservablepropertytest::v1::unobservable_property_test_server::UnobservablePropertyTestServer;

pub async fn serve(address: SocketAddr) -> Result<(), tonic::transport::Error> {
    let _ = crate::error::metadata_rejected;
    let shared = Arc::new(Shared::new());
    Server::builder()
        .add_service(BinaryUploadServer::new(UploadService::new(Arc::clone(
            &shared,
        ))))
        .add_service(BinaryDownloadServer::new(DownloadService::new(Arc::clone(
            &shared,
        ))))
        .add_service(SiLaServiceServer::new(SilaService::new(Arc::clone(
            &shared,
        ))))
        .add_service(UnobservablePropertyTestServer::new(Properties))
        .add_service(UnobservableCommandTestServer::new(Commands))
        .add_service(MetadataProviderServer::new(Provider))
        .add_service(MetadataConsumerTestServer::new(Consumer))
        .add_service(ErrorHandlingTestServer::new(ErrorHandling::default()))
        .add_service(ObservablePropertyTestServer::new(
            ObservableProperties::default(),
        ))
        .add_service(ObservableCommandTestServer::new(
            ObservableCommands::default(),
        ))
        .add_service(BinaryTransferTestServer::new(BinaryTests::new(Arc::clone(
            &shared,
        ))))
        .add_service(AuthorizationServiceServer::new(Authorization))
        .add_service(AuthenticationServiceServer::new(Authentication::new(
            Arc::clone(&shared),
        )))
        .add_service(AuthenticationTestServer::new(AuthenticationTests::new(
            Arc::clone(&shared),
        )))
        .add_service(MultiClientTestServer::new(MultiClient::default()))
        .add_service(BasicDataTypesTestServer::new(Basic))
        .add_service(StructureDataTypeTestServer::new(Structures))
        .add_service(ListDataTypeTestServer::new(Lists))
        .add_service(AnyTypeTestServer::new(AnyTypes))
        .serve(address)
        .await
}
