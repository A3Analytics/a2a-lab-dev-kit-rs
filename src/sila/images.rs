//! `LabImages` commands over [`A2aLabApi`](crate::service::A2aLabApi).
//!
//! Pixel bytes are never embedded. Each image is stored for binary download and
//! the response carries the transfer identifier.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId};
use crate::images::{GetCurrentImageRequest, GetImageRequest, Image, ListImageSourcesRequest};
use crate::page::PageRequest;
use crate::service::{A2aLabApi, A2aLabCommand, A2aLabResult};
use crate::sila::binary::BinaryStore;
use crate::sila::errors::{defined, reject_metadata, undefined, validation};
use crate::sila::values::{sila_bool, sila_string, timestamp_of};
use crate::sila::wire::sila2::com::a3analytics::lab::labimages::v1::data_type_image_source::ImageSourceStruct;
use crate::sila::wire::sila2::com::a3analytics::lab::labimages::v1::lab_images_server::LabImages;
use crate::sila::wire::sila2::com::a3analytics::lab::labimages::v1::{
    DataTypeImageSource, DataTypePageRequest, GetCurrentImageParameters, GetCurrentImageResponses,
    GetImageParameters, GetImageResponses, ListImageSourcesParameters, ListImageSourcesResponses,
};
use crate::sila::wire::sila2::org::silastandard::binary::Union;
use crate::sila::wire::sila2::org::silastandard::{Binary, Boolean, Integer, String as SilaString};

const LAB_IMAGES: &str = "com.a3analytics/lab/LabImages/v1";

#[derive(Clone)]
pub(crate) struct ImageFeature {
    pub lab: Arc<dyn A2aLabApi>,
    pub binaries: BinaryStore,
}

#[tonic::async_trait]
impl LabImages for ImageFeature {
    async fn list_image_sources(
        &self,
        request: Request<ListImageSourcesParameters>,
    ) -> Result<Response<ListImageSourcesResponses>, Status> {
        reject_metadata(&request)?;
        let page = page_from(request.into_inner().page.as_ref())?;
        let result = self
            .call(A2aLabCommand::ListImageSources(
                ListImageSourcesRequest::new(page).map_err(image_error)?,
            ))
            .await?;
        let A2aLabResult::ListImageSources(page) = result else {
            return Err(undefined("unexpected list image sources result"));
        };
        Ok(Response::new(ListImageSourcesResponses {
            sources: page.items().iter().map(source_message).collect(),
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn get_image(
        &self,
        request: Request<GetImageParameters>,
    ) -> Result<Response<GetImageResponses>, Status> {
        reject_metadata(&request)?;
        let id = read_image_id(request.into_inner().image_id.as_ref())?;
        let result = self
            .call(A2aLabCommand::GetImage(GetImageRequest::new(id)))
            .await?;
        let A2aLabResult::GetImage(image) = result else {
            return Err(undefined("unexpected get image result"));
        };
        Ok(Response::new(self.image_response(&image)?))
    }

    async fn get_current_image(
        &self,
        request: Request<GetCurrentImageParameters>,
    ) -> Result<Response<GetCurrentImageResponses>, Status> {
        reject_metadata(&request)?;
        let source_id = read_source_id(request.into_inner().source_id.as_ref())?;
        let result = self
            .call(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
                source_id,
            )))
            .await?;
        let A2aLabResult::GetCurrentImage(image) = result else {
            return Err(undefined("unexpected get current image result"));
        };
        let response = self.image_response(&image)?;
        Ok(Response::new(GetCurrentImageResponses {
            image_id: response.image_id,
            source_id: response.source_id,
            captured_at: response.captured_at,
            media_type: response.media_type,
            width: response.width,
            height: response.height,
            has_caption: response.has_caption,
            caption: response.caption,
            image: response.image,
        }))
    }
}

impl ImageFeature {
    async fn call(&self, command: A2aLabCommand) -> Result<A2aLabResult, Status> {
        self.lab
            .execute(command)
            .await
            .map(|outcome| outcome.task.result)
            .map_err(image_error)
    }

    fn image_response(&self, image: &Image) -> Result<GetImageResponses, Status> {
        let descriptor = image.descriptor();
        let transfer = self.binaries.insert(image.data())?;
        let (has_caption, caption) = caption_fields(descriptor.caption());
        let captured_at =
            timestamp_of(descriptor.captured_at()).map_err(|error| undefined(error.to_string()))?;
        Ok(GetImageResponses {
            image_id: Some(sila_string(descriptor.id().as_str())),
            source_id: Some(sila_string(descriptor.source_id().as_str())),
            captured_at: Some(captured_at),
            media_type: Some(sila_string(descriptor.media_type())),
            width: Some(Integer {
                value: i64::from(descriptor.width()),
            }),
            height: Some(Integer {
                value: i64::from(descriptor.height()),
            }),
            has_caption: Some(has_caption),
            caption: Some(caption),
            image: Some(Binary {
                union: Some(Union::BinaryTransferUuid(transfer)),
            }),
        })
    }
}

fn source_message(source: &crate::images::ImageSource) -> DataTypeImageSource {
    let (has_asset, asset) = optional_text(source.asset_id.as_deref());
    let (has_semantic, semantic) = optional_text(source.semantic_id.as_deref());
    DataTypeImageSource {
        image_source: Some(ImageSourceStruct {
            id: Some(sila_string(source.id.as_str())),
            name: Some(sila_string(&source.name)),
            description: Some(sila_string(&source.description)),
            has_asset_id: Some(has_asset),
            asset_id: Some(asset),
            has_semantic_id: Some(has_semantic),
            semantic_id: Some(semantic),
        }),
    }
}

fn optional_text(value: Option<&str>) -> (Boolean, SilaString) {
    (
        sila_bool(value.is_some()),
        sila_string(value.unwrap_or_default()),
    )
}

fn caption_fields(caption: Option<&str>) -> (Boolean, SilaString) {
    (
        sila_bool(caption.is_some()),
        sila_string(caption.unwrap_or_default()),
    )
}

fn page_from(page: Option<&DataTypePageRequest>) -> Result<PageRequest, Status> {
    let page =
        page.ok_or_else(|| validation(parameter("ListImageSources", "Page"), "is required"))?;
    let inner = page
        .page_request
        .as_ref()
        .ok_or_else(|| validation(parameter("ListImageSources", "Page"), "is required"))?;
    let cursor = if inner.has_cursor.as_ref().is_some_and(|flag| flag.value) {
        let cursor = inner
            .cursor
            .as_ref()
            .map(|item| item.value.clone())
            .unwrap_or_default();
        if cursor.is_empty() {
            return Err(validation(
                parameter("ListImageSources", "Page"),
                "is not a valid page cursor",
            ));
        }
        Some(cursor)
    } else {
        None
    };
    let limit = inner.limit.as_ref().map_or(0, |item| item.value);
    let limit = u32::try_from(limit).unwrap_or(0);
    PageRequest::new(cursor, limit)
        .map_err(|error| validation(parameter("ListImageSources", "Page"), error.to_string()))
}

fn read_image_id(value: Option<&SilaString>) -> Result<ImageId, Status> {
    let text = required_text(value, "GetImage", "ImageId")?;
    ImageId::new(text)
        .map_err(|error| validation(parameter("GetImage", "ImageId"), error.to_string()))
}

fn read_source_id(value: Option<&SilaString>) -> Result<ImageSourceId, Status> {
    let text = required_text(value, "GetCurrentImage", "SourceId")?;
    ImageSourceId::new(text)
        .map_err(|error| validation(parameter("GetCurrentImage", "SourceId"), error.to_string()))
}

fn required_text(value: Option<&SilaString>, command: &str, name: &str) -> Result<String, Status> {
    value
        .map(|item| item.value.clone())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| validation(parameter(command, name), "is required"))
}

fn parameter(command: &str, name: &str) -> String {
    format!("{LAB_IMAGES}/Command/{command}/Parameter/{name}")
}

fn image_error(error: A2aLabError) -> Status {
    match error {
        A2aLabError::Invalid { message, .. } => undefined(message),
        A2aLabError::NotFound { kind, id } => {
            let identifier = if kind == "image source" {
                "ImageSourceNotFound"
            } else {
                "ImageNotFound"
            };
            defined(
                LAB_IMAGES,
                identifier,
                format!("{kind} `{id}` was not found"),
            )
        }
        A2aLabError::Unavailable { message } => defined(LAB_IMAGES, "ProviderUnavailable", message),
        A2aLabError::Transport { message } | A2aLabError::Protocol { message } => {
            undefined(message)
        }
    }
}
