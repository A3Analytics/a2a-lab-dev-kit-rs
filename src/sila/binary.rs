//! SiLA binary download for lab images.
//!
//! Every image is stored here and returned as a transfer identifier. The command
//! response never embeds pixel bytes.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::stream::{self, Stream};
use tonic::{Request, Response, Status, Streaming};

use crate::sila::constraints::EXECUTION_LIFETIME_SECONDS;
use crate::sila::errors::binary_transfer;
use crate::sila::wire::sila2::org::silastandard::binary_download_server::BinaryDownload;
use crate::sila::wire::sila2::org::silastandard::binary_transfer_error::ErrorType as BinaryError;
use crate::sila::wire::sila2::org::silastandard::{
    DeleteBinaryRequest, DeleteBinaryResponse, Duration as SilaDuration, GetBinaryInfoRequest,
    GetBinaryInfoResponse, GetChunkRequest, GetChunkResponse,
};

const TWO_MIB: u32 = 2 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct BinaryStore {
    inner: Arc<Mutex<HashMap<String, Stored>>>,
}

struct Stored {
    bytes: Vec<u8>,
    expires: Instant,
}

impl BinaryStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&self, bytes: &[u8]) -> Result<String, Status> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut binaries = self.lock()?;
        binaries.insert(
            id.clone(),
            Stored {
                bytes: bytes.to_vec(),
                expires: Instant::now() + Duration::from_secs(lifetime_seconds()),
            },
        );
        Ok(id)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Stored>>, Status> {
        self.inner
            .lock()
            .map_err(|_| download_failed("binary store is unavailable"))
    }
}

#[derive(Clone)]
pub(crate) struct DownloadFeature {
    pub binaries: BinaryStore,
}

#[tonic::async_trait]
impl BinaryDownload for DownloadFeature {
    type GetChunkStream = Pin<Box<dyn Stream<Item = Result<GetChunkResponse, Status>> + Send>>;

    async fn get_binary_info(
        &self,
        request: Request<GetBinaryInfoRequest>,
    ) -> Result<Response<GetBinaryInfoResponse>, Status> {
        let id = request.into_inner().binary_transfer_uuid;
        let bytes = require(&self.binaries, &id)?;
        Ok(Response::new(GetBinaryInfoResponse {
            binary_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            lifetime_of_binary: Some(lifetime()),
        }))
    }

    async fn get_chunk(
        &self,
        request: Request<Streaming<GetChunkRequest>>,
    ) -> Result<Response<Self::GetChunkStream>, Status> {
        let binaries = self.binaries.clone();
        let inbound = request.into_inner();
        Ok(Response::new(Box::pin(stream::unfold(
            inbound,
            move |mut inbound| {
                let binaries = binaries.clone();
                async move {
                    match inbound.message().await {
                        Ok(Some(request)) => Some((read_chunk(&binaries, &request), inbound)),
                        Ok(None) => None,
                        Err(status) => Some((Err(status), inbound)),
                    }
                }
            },
        ))))
    }

    async fn delete_binary(
        &self,
        request: Request<DeleteBinaryRequest>,
    ) -> Result<Response<DeleteBinaryResponse>, Status> {
        let id = request.into_inner().binary_transfer_uuid;
        remove(&self.binaries, &id)?;
        Ok(Response::new(DeleteBinaryResponse {}))
    }
}

fn require(store: &BinaryStore, id: &str) -> Result<Vec<u8>, Status> {
    let _ = parsed_id(id)?;
    let mut binaries = store.lock()?;
    let Some(stored) = binaries.get(id) else {
        return Err(unknown_transfer(id));
    };
    if Instant::now() >= stored.expires {
        binaries.remove(id);
        return Err(unknown_transfer(id));
    }
    Ok(stored.bytes.clone())
}

fn remove(store: &BinaryStore, id: &str) -> Result<(), Status> {
    let _ = parsed_id(id)?;
    let mut binaries = store.lock()?;
    if binaries.remove(id).is_some() {
        Ok(())
    } else {
        Err(unknown_transfer(id))
    }
}

fn read_chunk(store: &BinaryStore, request: &GetChunkRequest) -> Result<GetChunkResponse, Status> {
    let bytes = require(store, &request.binary_transfer_uuid)?;
    if request.length > TWO_MIB {
        return Err(download_failed("Requested chunk size is > 2 MiB"));
    }
    let end = u128::from(request.offset) + u128::from(request.length);
    if end > u128::try_from(bytes.len()).unwrap_or(u128::MAX) {
        return Err(download_failed(&format!(
            "Requested range is out of range for binary {} of length {} (offset: {}, length: {})",
            request.binary_transfer_uuid,
            bytes.len(),
            request.offset,
            request.length
        )));
    }
    let start = usize::try_from(request.offset).unwrap_or(usize::MAX);
    let length = usize::try_from(request.length).unwrap_or(0);
    Ok(GetChunkResponse {
        binary_transfer_uuid: request.binary_transfer_uuid.clone(),
        offset: request.offset,
        payload: bytes
            .get(start..start.saturating_add(length))
            .unwrap_or_default()
            .to_vec(),
        lifetime_of_binary: Some(lifetime()),
    })
}

fn parsed_id(id: &str) -> Result<uuid::Uuid, Status> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| invalid_transfer(&format!("Given id is not a valid UUID: {id}")))
}

fn unknown_transfer(id: &str) -> Status {
    invalid_transfer(&format!("No binary exists with UUID {id}"))
}

fn invalid_transfer(message: &str) -> Status {
    binary_transfer(BinaryError::InvalidBinaryTransferUuid, message)
}

fn download_failed(message: &str) -> Status {
    binary_transfer(BinaryError::BinaryDownloadFailed, message)
}

fn lifetime() -> SilaDuration {
    SilaDuration {
        seconds: EXECUTION_LIFETIME_SECONDS,
        nanos: 0,
    }
}

fn lifetime_seconds() -> u64 {
    u64::try_from(EXECUTION_LIFETIME_SECONDS).unwrap_or(60)
}
