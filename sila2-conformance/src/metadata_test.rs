//! Metadata provider and consumer test features.

use tonic::{Request, Response, Status};

use crate::error::framework;
use crate::metadata::{self, CallMetadata};
use crate::support::{integer, string, RpcStream};
use crate::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::wire::sila2::org::silastandard::test::metadataconsumertest::v1::metadata_consumer_test_server::MetadataConsumerTest;
use crate::wire::sila2::org::silastandard::test::metadataconsumertest::v1::{
    EchoStringMetadataParameters, EchoStringMetadataResponses, GetReceivedStringMetadataParameters,
    GetReceivedStringMetadataResponses, SubscribeReceivedStringMetadataAsCharactersParameters,
    SubscribeReceivedStringMetadataAsCharactersResponses, UnpackMetadataParameters, UnpackMetadataResponses,
};
use crate::wire::sila2::org::silastandard::test::metadataprovider::v1::metadata_provider_server::MetadataProvider;
use crate::wire::sila2::org::silastandard::test::metadataprovider::v1::{
    GetFcpAffectedByMetadataStringMetadataParameters, GetFcpAffectedByMetadataStringMetadataResponses,
    GetFcpAffectedByMetadataTwoIntegersMetadataParameters, GetFcpAffectedByMetadataTwoIntegersMetadataResponses,
};

const STRING_METADATA: &str = "org.silastandard/test/MetadataProvider/v1/Metadata/StringMetadata";
const BOTH_METADATA: &str = "Missing metadata, expected 'org.silastandard/test/MetadataProvider/v1/Metadata/StringMetadata' and 'org.silastandard/test/MetadataProvider/v1/Metadata/TwoIntegersMetadata'";

#[derive(Clone, Copy, Default)]
pub struct Provider;

#[derive(Clone, Copy, Default)]
pub struct Consumer;

#[tonic::async_trait]
impl MetadataProvider for Provider {
    async fn get_fcp_affected_by_metadata_string_metadata(
        &self,
        _request: Request<GetFcpAffectedByMetadataStringMetadataParameters>,
    ) -> Result<Response<GetFcpAffectedByMetadataStringMetadataResponses>, Status> {
        Ok(Response::new(
            GetFcpAffectedByMetadataStringMetadataResponses {
                affected_calls: vec![string("org.silastandard/test/MetadataConsumerTest/v1")],
            },
        ))
    }

    async fn get_fcp_affected_by_metadata_two_integers_metadata(
        &self,
        _request: Request<GetFcpAffectedByMetadataTwoIntegersMetadataParameters>,
    ) -> Result<Response<GetFcpAffectedByMetadataTwoIntegersMetadataResponses>, Status> {
        Ok(Response::new(
            GetFcpAffectedByMetadataTwoIntegersMetadataResponses {
                affected_calls: vec![string(
                    "org.silastandard/test/MetadataConsumerTest/v1/Command/UnpackMetadata",
                )],
            },
        ))
    }
}

#[tonic::async_trait]
impl MetadataConsumerTest for Consumer {
    type Subscribe_ReceivedStringMetadataAsCharactersStream =
        RpcStream<SubscribeReceivedStringMetadataAsCharactersResponses>;

    async fn echo_string_metadata(
        &self,
        request: Request<EchoStringMetadataParameters>,
    ) -> Result<Response<EchoStringMetadataResponses>, Status> {
        let value = require_string(&metadata::extract(request.metadata()), false)?;
        Ok(Response::new(EchoStringMetadataResponses {
            received_string_metadata: Some(string(value)),
        }))
    }

    async fn unpack_metadata(
        &self,
        request: Request<UnpackMetadataParameters>,
    ) -> Result<Response<UnpackMetadataResponses>, Status> {
        let parsed = metadata::extract(request.metadata());
        let received = require_string(&parsed, true)?;
        let pair = parsed
            .two_integers
            .as_ref()
            .and_then(|item| item.two_integers_metadata.as_ref());
        let Some(pair) = pair else {
            return Err(invalid(if parsed.two_integers.is_none() {
                BOTH_METADATA
            } else {
                "Received TwoIntegersMetadata message was empty"
            }));
        };
        let Some(first) = pair.first_integer else {
            return Err(invalid(
                "Received TwoIntegersMetadata has empty field FirstInteger",
            ));
        };
        let Some(second) = pair.second_integer else {
            return Err(invalid(
                "Received TwoIntegersMetadata has empty field SecondInteger",
            ));
        };
        Ok(Response::new(UnpackMetadataResponses {
            received_string: Some(string(received)),
            first_received_integer: Some(integer(first.value)),
            second_received_integer: Some(integer(second.value)),
        }))
    }

    async fn get_received_string_metadata(
        &self,
        request: Request<GetReceivedStringMetadataParameters>,
    ) -> Result<Response<GetReceivedStringMetadataResponses>, Status> {
        let value = require_string(&metadata::extract(request.metadata()), false)?;
        Ok(Response::new(GetReceivedStringMetadataResponses {
            received_string_metadata: Some(string(value)),
        }))
    }

    async fn subscribe_received_string_metadata_as_characters(
        &self,
        request: Request<SubscribeReceivedStringMetadataAsCharactersParameters>,
    ) -> Result<Response<Self::Subscribe_ReceivedStringMetadataAsCharactersStream>, Status> {
        let value = require_string(&metadata::extract(request.metadata()), true)?;
        let items = value
            .chars()
            .map(|character| {
                Ok(SubscribeReceivedStringMetadataAsCharactersResponses {
                    received_string_metadata_as_characters: Some(string(character.to_string())),
                })
            })
            .collect::<Vec<_>>();
        Ok(Response::new(Box::pin(futures_util::stream::iter(items))))
    }
}

fn require_string(parsed: &CallMetadata, subscribe_message: bool) -> Result<String, Status> {
    let Some(metadata) = &parsed.string_metadata else {
        let message = if subscribe_message {
            BOTH_METADATA
        } else {
            "Missing metadata, expected 'org.silastandard/test/MetadataProvider/v1/Metadata/StringMetadata'"
        };
        return Err(invalid(message));
    };
    let Some(value) = &metadata.string_metadata else {
        return Err(invalid("Received StringMetadata message was empty"));
    };
    let _ = STRING_METADATA;
    Ok(value.value.clone())
}

fn invalid(message: &str) -> Status {
    framework(ErrorType::InvalidMetadata, message)
}
