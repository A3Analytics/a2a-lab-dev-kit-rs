//! Mandatory `SiLAService` implementation.

use std::sync::{Arc, Mutex};

use tonic::{Request, Response, Status};

use crate::sila::constraints::feature_identifier;
use crate::sila::errors::{reject_metadata, undefined, validation};
use crate::sila::identity::SilaIdentity;
use crate::sila::values::sila_string;
use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaService;
use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::{
    GetFeatureDefinitionParameters, GetFeatureDefinitionResponses,
    GetImplementedFeaturesParameters, GetImplementedFeaturesResponses,
    GetServerDescriptionParameters, GetServerDescriptionResponses, GetServerNameParameters,
    GetServerNameResponses, GetServerTypeParameters, GetServerTypeResponses,
    GetServerUuidParameters, GetServerUuidResponses, GetServerVendorUrlParameters,
    GetServerVendorUrlResponses, GetServerVersionParameters, GetServerVersionResponses,
    SetServerNameParameters, SetServerNameResponses,
};
pub(crate) const SILA_SERVICE: &str = "org.silastandard/core/SiLAService/v1";
pub(crate) const LAB_OPERATIONS: &str = "com.a3analytics/lab/LabOperations/v1";
pub(crate) const LAB_IMAGES: &str = "com.a3analytics/lab/LabImages/v1";
pub(crate) const CANCEL_CONTROLLER: &str = "org.silastandard/core/commands/CancelController/v1";
pub(crate) const CONNECTION_CONFIGURATION: &str =
    "org.silastandard/core/ConnectionConfigurationService/v1";
const FEATURE_IDENTIFIER: &str =
    "org.silastandard/core/SiLAService/v1/Command/GetFeatureDefinition/Parameter/FeatureIdentifier";

const SILA_SERVICE_XML: &str = include_str!("standard/SiLAService.sila.xml");
const LAB_OPERATIONS_XML: &str = include_str!("standard/LabOperations.sila.xml");
const LAB_IMAGES_XML: &str = include_str!("standard/LabImages.sila.xml");
const CANCEL_CONTROLLER_XML: &str = include_str!("standard/CancelController.sila.xml");
const CONNECTION_CONFIGURATION_XML: &str =
    include_str!("standard/ConnectionConfigurationService.sila.xml");

impl FeatureCatalog {
    pub(crate) fn ids(self) -> Vec<&'static str> {
        let mut ids = vec![SILA_SERVICE];
        if self.connection {
            ids.push(CONNECTION_CONFIGURATION);
        }
        ids.push(LAB_OPERATIONS);
        ids.push(LAB_IMAGES);
        ids.push(CANCEL_CONTROLLER);
        ids
    }

    fn definition(self, identifier: &str) -> Option<&'static str> {
        match identifier {
            SILA_SERVICE => Some(SILA_SERVICE_XML),
            LAB_OPERATIONS => Some(LAB_OPERATIONS_XML),
            LAB_IMAGES => Some(LAB_IMAGES_XML),
            CANCEL_CONTROLLER => Some(CANCEL_CONTROLLER_XML),
            CONNECTION_CONFIGURATION if self.connection => Some(CONNECTION_CONFIGURATION_XML),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FeatureCatalog {
    pub connection: bool,
}

#[derive(Clone)]
pub(crate) struct CoreService {
    pub identity: Arc<Mutex<SilaIdentity>>,
    pub on_rename: Arc<dyn Fn() + Send + Sync>,
    pub features: FeatureCatalog,
}

#[tonic::async_trait]
impl SiLaService for CoreService {
    async fn get_feature_definition(
        &self,
        request: Request<GetFeatureDefinitionParameters>,
    ) -> Result<Response<GetFeatureDefinitionResponses>, Status> {
        reject_metadata(&request)?;
        let identifier = request
            .into_inner()
            .feature_identifier
            .map(|item| item.value)
            .unwrap_or_default();
        if !feature_identifier(&identifier) {
            return Err(validation(
                FEATURE_IDENTIFIER,
                "must be a fully qualified feature identifier",
            ));
        }
        let xml = self.features.definition(&identifier).ok_or_else(|| {
            crate::sila::errors::defined(
                SILA_SERVICE,
                "UnimplementedFeature",
                format!("feature `{identifier}` is not implemented"),
            )
        })?;
        Ok(Response::new(GetFeatureDefinitionResponses {
            feature_definition: Some(sila_string(xml)),
        }))
    }

    async fn set_server_name(
        &self,
        request: Request<SetServerNameParameters>,
    ) -> Result<Response<SetServerNameResponses>, Status> {
        reject_metadata(&request)?;
        let name = request
            .into_inner()
            .server_name
            .map(|item| item.value)
            .unwrap_or_default();
        let renamed = {
            let mut identity = self
                .identity
                .lock()
                .map_err(|_| undefined("server identity is unavailable"))?;
            identity.set_name(name).map_err(|error| {
                validation(
                    "org.silastandard/core/SiLAService/v1/Command/SetServerName/Parameter/ServerName",
                    error.to_string(),
                )
            })?;
            true
        };
        if renamed {
            (self.on_rename)();
        }
        Ok(Response::new(SetServerNameResponses {}))
    }

    async fn get_server_name(
        &self,
        request: Request<GetServerNameParameters>,
    ) -> Result<Response<GetServerNameResponses>, Status> {
        reject_metadata(&request)?;
        let name = self
            .identity
            .lock()
            .map_err(|_| undefined("server identity is unavailable"))?
            .server_name
            .clone();
        Ok(Response::new(GetServerNameResponses {
            server_name: Some(sila_string(name)),
        }))
    }

    async fn get_server_type(
        &self,
        request: Request<GetServerTypeParameters>,
    ) -> Result<Response<GetServerTypeResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetServerTypeResponses {
            server_type: Some(sila_string(
                self.field(|identity| identity.server_type.clone())?,
            )),
        }))
    }

    async fn get_server_uuid(
        &self,
        request: Request<GetServerUuidParameters>,
    ) -> Result<Response<GetServerUuidResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetServerUuidResponses {
            server_uuid: Some(sila_string(
                self.field(|identity| identity.server_uuid.clone())?,
            )),
        }))
    }

    async fn get_server_description(
        &self,
        request: Request<GetServerDescriptionParameters>,
    ) -> Result<Response<GetServerDescriptionResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetServerDescriptionResponses {
            server_description: Some(sila_string(
                self.field(|identity| identity.description.clone())?,
            )),
        }))
    }

    async fn get_server_version(
        &self,
        request: Request<GetServerVersionParameters>,
    ) -> Result<Response<GetServerVersionResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetServerVersionResponses {
            server_version: Some(sila_string(
                self.field(|identity| identity.version.clone())?,
            )),
        }))
    }

    async fn get_server_vendor_url(
        &self,
        request: Request<GetServerVendorUrlParameters>,
    ) -> Result<Response<GetServerVendorUrlResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetServerVendorUrlResponses {
            server_vendor_url: Some(sila_string(
                self.field(|identity| identity.vendor_url.clone())?,
            )),
        }))
    }

    async fn get_implemented_features(
        &self,
        request: Request<GetImplementedFeaturesParameters>,
    ) -> Result<Response<GetImplementedFeaturesResponses>, Status> {
        reject_metadata(&request)?;
        Ok(Response::new(GetImplementedFeaturesResponses {
            implemented_features: self.features.ids().into_iter().map(sila_string).collect(),
        }))
    }
}

impl CoreService {
    fn field(&self, read: impl FnOnce(&SilaIdentity) -> String) -> Result<String, Status> {
        let identity = self
            .identity
            .lock()
            .map_err(|_| undefined("server identity is unavailable"))?;
        Ok(read(&identity))
    }
}
