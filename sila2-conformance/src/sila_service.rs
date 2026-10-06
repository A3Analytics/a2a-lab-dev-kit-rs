//! SiLAService feature.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::{self, framework};
use crate::metadata::has_sila_metadata;
use crate::shared::Shared;
use crate::support::{self, string};
use crate::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaService;
use crate::wire::sila2::org::silastandard::core::silaservice::v1::{
    GetFeatureDefinitionParameters, GetFeatureDefinitionResponses,
    GetImplementedFeaturesParameters, GetImplementedFeaturesResponses,
    GetServerDescriptionParameters, GetServerDescriptionResponses, GetServerNameParameters,
    GetServerNameResponses, GetServerTypeParameters, GetServerTypeResponses,
    GetServerUuidParameters, GetServerUuidResponses, GetServerVendorUrlParameters,
    GetServerVendorUrlResponses, GetServerVersionParameters, GetServerVersionResponses,
    SetServerNameParameters, SetServerNameResponses,
};
use crate::wire::sila2::org::silastandard::framework_error::ErrorType;

const FEATURE_PARAMETER: &str =
    "org.silastandard/core/SiLAService/v1/Command/GetFeatureDefinition/Parameter/FeatureIdentifier";
const NAME_PARAMETER: &str =
    "org.silastandard/core/SiLAService/v1/Command/SetServerName/Parameter/ServerName";

pub struct Service {
    shared: Arc<Shared>,
}

impl Service {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[tonic::async_trait]
impl SiLaService for Service {
    async fn get_feature_definition(
        &self,
        request: Request<GetFeatureDefinitionParameters>,
    ) -> Result<Response<GetFeatureDefinitionResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.GetFeatureDefinition received metadata",
        )?;
        let identifier = request.into_inner().feature_identifier;
        let Some(feature) = identifier else {
            return Err(error::validation(FEATURE_PARAMETER, "Missing parameter"));
        };
        if !support::is_feature_identifier(&feature.value) {
            return Err(error::validation(
                FEATURE_PARAMETER,
                &format!(
                    "Not a fully qualified feature identifier: '{}'",
                    feature.value
                ),
            ));
        }
        let Some(definition) = self.shared.feature_definition(&feature.value) else {
            return Err(error::defined(
                "org.silastandard/core/SiLAService/v1/DefinedExecutionError/UnimplementedFeature",
                &format!("Feature is not implemented: '{}'", feature.value),
            ));
        };
        Ok(Response::new(GetFeatureDefinitionResponses {
            feature_definition: Some(string(definition)),
        }))
    }

    async fn set_server_name(
        &self,
        request: Request<SetServerNameParameters>,
    ) -> Result<Response<SetServerNameResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.SetServerName received metadata",
        )?;
        let Some(name) = request.into_inner().server_name else {
            return Err(error::validation(NAME_PARAMETER, "Missing parameter"));
        };
        if name.value.is_empty() || name.value.chars().count() > 255 {
            return Err(error::validation(
                NAME_PARAMETER,
                "Invalid name, must be non-empty and <= 255 characters long",
            ));
        }
        *support::guard(&self.shared.name) = name.value;
        Ok(Response::new(SetServerNameResponses {}))
    }

    async fn get_server_name(
        &self,
        request: Request<GetServerNameParameters>,
    ) -> Result<Response<GetServerNameResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerName requested with metadata",
        )?;
        let name = support::guard(&self.shared.name).clone();
        Ok(Response::new(GetServerNameResponses {
            server_name: Some(string(name)),
        }))
    }

    async fn get_server_type(
        &self,
        request: Request<GetServerTypeParameters>,
    ) -> Result<Response<GetServerTypeResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerType requested with metadata",
        )?;
        Ok(Response::new(GetServerTypeResponses {
            server_type: Some(string("TestServer")),
        }))
    }

    async fn get_server_uuid(
        &self,
        request: Request<GetServerUuidParameters>,
    ) -> Result<Response<GetServerUuidResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerUUID requested with metadata",
        )?;
        Ok(Response::new(GetServerUuidResponses {
            server_uuid: Some(string(self.shared.server_uuid.to_string())),
        }))
    }

    async fn get_server_description(
        &self,
        request: Request<GetServerDescriptionParameters>,
    ) -> Result<Response<GetServerDescriptionResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerDescription requested with metadata",
        )?;
        Ok(Response::new(GetServerDescriptionResponses {
            server_description: Some(string("This is a test server")),
        }))
    }

    async fn get_server_version(
        &self,
        request: Request<GetServerVersionParameters>,
    ) -> Result<Response<GetServerVersionResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerVersion requested with metadata",
        )?;
        Ok(Response::new(GetServerVersionResponses {
            server_version: Some(string("0.1")),
        }))
    }

    async fn get_server_vendor_url(
        &self,
        request: Request<GetServerVendorUrlParameters>,
    ) -> Result<Response<GetServerVendorUrlResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ServerVendorURL requested with metadata",
        )?;
        Ok(Response::new(GetServerVendorUrlResponses {
            server_vendor_url: Some(string("https://gitlab.com/SiLA2/sila_interoperability")),
        }))
    }

    async fn get_implemented_features(
        &self,
        request: Request<GetImplementedFeaturesParameters>,
    ) -> Result<Response<GetImplementedFeaturesResponses>, Status> {
        reject(
            request.metadata(),
            "SiLAService.ImplementedFeatures requested with metadata",
        )?;
        let implemented_features = self
            .shared
            .features
            .iter()
            .map(|(identifier, _)| string(identifier.clone()))
            .collect();
        Ok(Response::new(GetImplementedFeaturesResponses {
            implemented_features,
        }))
    }
}

fn reject(metadata: &tonic::metadata::MetadataMap, message: &str) -> Result<(), Status> {
    if has_sila_metadata(metadata) {
        Err(framework(ErrorType::NoMetadataAllowed, message))
    } else {
        Ok(())
    }
}
