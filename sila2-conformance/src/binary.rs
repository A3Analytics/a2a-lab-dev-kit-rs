//! Binary upload, download, and BinaryTransferTest.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::stream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::auth;
use crate::error::{self, framework};
use crate::metadata::{self, CallMetadata};
use crate::shared::{Shared, Upload};
use crate::support::{
    self, binary_bytes, binary_transfer_status, confirmation, execution, guard, lifetime, string, RpcStream, TWO_MIB,
};
use crate::wire::sila2::org::silastandard::binary_download_server::BinaryDownload;
use crate::wire::sila2::org::silastandard::binary_transfer_error::ErrorType as BinaryError;
use crate::wire::sila2::org::silastandard::binary_upload_server::BinaryUpload;
use crate::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::wire::sila2::org::silastandard::test::binarytransfertest::v1::binary_transfer_test_server::BinaryTransferTest;
use crate::wire::sila2::org::silastandard::test::binarytransfertest::v1::{
    EchoBinariesObservablyIntermediateResponses, EchoBinariesObservablyParameters, EchoBinariesObservablyResponses,
    EchoBinaryAndMetadataStringParameters, EchoBinaryAndMetadataStringResponses, EchoBinaryValueParameters,
    EchoBinaryValueResponses, GetBinaryValueDirectlyParameters, GetBinaryValueDirectlyResponses,
    GetBinaryValueDownloadParameters, GetBinaryValueDownloadResponses, GetFcpAffectedByMetadataStringParameters,
    GetFcpAffectedByMetadataStringResponses,
};
use crate::wire::sila2::org::silastandard::{
    self as fw, CreateBinaryRequest, CreateBinaryResponse, DeleteBinaryRequest, DeleteBinaryResponse, GetBinaryInfoRequest,
    GetBinaryInfoResponse, GetChunkRequest, GetChunkResponse, UploadChunkRequest, UploadChunkResponse,
};
use crate::wire::sila2::org::silastandard::execution_info::CommandStatus;

const ECHO_VALUE: &str =
    "org.silastandard/test/BinaryTransferTest/v1/Command/EchoBinaryValue/Parameter/BinaryValue";
const ECHO_LIST: &str =
    "org.silastandard/test/BinaryTransferTest/v1/Command/EchoBinariesObservably/Parameter/Binaries";
const ECHO_META: &str = "org.silastandard/test/BinaryTransferTest/v1/Command/EchoBinaryAndMetadataString/Parameter/Binary";
const AUTH_UPLOAD: &str = "org.silastandard/test/AuthenticationTest/v1/Command/RequiresTokenForBinaryUpload/Parameter/BinaryToUpload";
const DOWNLOAD_SAMPLE: &str =
    "A_slightly_longer_SiLA2_Test_String_Value_used_to_demonstrate_the_binary_download";

pub struct UploadService {
    shared: Arc<Shared>,
}

pub struct DownloadService {
    shared: Arc<Shared>,
}

pub struct TestFeature {
    shared: Arc<Shared>,
    echoes: Mutex<HashMap<Uuid, Arc<EchoBinaries>>>,
}

struct EchoBinaries {
    parameters: Vec<Vec<u8>>,
    start: Instant,
    index: Mutex<u32>,
}

impl UploadService {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl DownloadService {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl TestFeature {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            echoes: Mutex::new(HashMap::new()),
        }
    }
}

#[tonic::async_trait]
impl BinaryUpload for UploadService {
    type UploadChunkStream = RpcStream<UploadChunkResponse>;

    async fn create_binary(
        &self,
        request: Request<CreateBinaryRequest>,
    ) -> Result<Response<CreateBinaryResponse>, Status> {
        let parsed = metadata::extract(request.metadata());
        let request = request.into_inner();
        if !allowed(&request.parameter_identifier) {
            return Err(upload_failed(&format!(
                "Binary upload not allowed for parameter '{}'",
                request.parameter_identifier
            )));
        }
        validate_create(&request.parameter_identifier, &parsed, &self.shared)?;
        let identifier = Uuid::new_v4();
        guard(&self.shared.uploads).insert(
            identifier,
            Upload {
                chunks: request.chunk_count,
                size: request.binary_size,
                parts: HashMap::new(),
            },
        );
        Ok(Response::new(CreateBinaryResponse {
            binary_transfer_uuid: identifier.to_string(),
            lifetime_of_binary: Some(lifetime()),
        }))
    }

    async fn upload_chunk(
        &self,
        request: Request<Streaming<UploadChunkRequest>>,
    ) -> Result<Response<Self::UploadChunkStream>, Status> {
        Ok(Response::new(chunk_stream(
            request.into_inner(),
            Arc::clone(&self.shared),
        )))
    }

    async fn delete_binary(
        &self,
        request: Request<DeleteBinaryRequest>,
    ) -> Result<Response<DeleteBinaryResponse>, Status> {
        let identifier = request.into_inner().binary_transfer_uuid;
        remove_upload(&self.shared, &identifier)?;
        Ok(Response::new(DeleteBinaryResponse {}))
    }
}

#[tonic::async_trait]
impl BinaryDownload for DownloadService {
    type GetChunkStream = RpcStream<GetChunkResponse>;

    async fn get_binary_info(
        &self,
        request: Request<GetBinaryInfoRequest>,
    ) -> Result<Response<GetBinaryInfoResponse>, Status> {
        let identifier = request.into_inner().binary_transfer_uuid;
        let bytes = require_download(&self.shared, &identifier)?;
        Ok(Response::new(GetBinaryInfoResponse {
            binary_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            lifetime_of_binary: Some(lifetime()),
        }))
    }

    async fn get_chunk(
        &self,
        request: Request<Streaming<GetChunkRequest>>,
    ) -> Result<Response<Self::GetChunkStream>, Status> {
        Ok(Response::new(download_stream(
            request.into_inner(),
            Arc::clone(&self.shared),
        )))
    }

    async fn delete_binary(
        &self,
        request: Request<DeleteBinaryRequest>,
    ) -> Result<Response<DeleteBinaryResponse>, Status> {
        let identifier = request.into_inner().binary_transfer_uuid;
        remove_download(&self.shared, &identifier)?;
        Ok(Response::new(DeleteBinaryResponse {}))
    }
}

#[tonic::async_trait]
impl BinaryTransferTest for TestFeature {
    type EchoBinariesObservably_InfoStream = RpcStream<fw::ExecutionInfo>;
    type EchoBinariesObservably_IntermediateStream =
        RpcStream<EchoBinariesObservablyIntermediateResponses>;

    async fn echo_binary_value(
        &self,
        request: Request<EchoBinaryValueParameters>,
    ) -> Result<Response<EchoBinaryValueResponses>, Status> {
        let Some(binary) = request.into_inner().binary_value else {
            return Err(error::validation(
                ECHO_VALUE,
                "Missing parameter 'BinaryValue'",
            ));
        };
        let bytes = self.shared.read_binary(&binary, ECHO_VALUE)?;
        Ok(Response::new(EchoBinaryValueResponses {
            received_value: Some(self.shared.pack_binary(&bytes)),
        }))
    }

    async fn echo_binaries_observably(
        &self,
        request: Request<EchoBinariesObservablyParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let mut parameters = Vec::new();
        for item in request.into_inner().binaries {
            parameters.push(self.shared.read_binary(&item, ECHO_LIST)?);
        }
        let exec = Arc::new(EchoBinaries {
            parameters,
            start: Instant::now(),
            index: Mutex::new(0),
        });
        spawn_echo(Arc::clone(&exec));
        let identifier = Uuid::new_v4();
        guard(&self.echoes).insert(identifier, exec);
        Ok(Response::new(confirmation(identifier)))
    }

    async fn echo_binaries_observably_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::EchoBinariesObservably_InfoStream>, Status> {
        let value = request.into_inner().value;
        let exec = self.echo(&value)?;
        Ok(Response::new(Box::pin(stream::unfold(
            false,
            move |sent| {
                let exec = Arc::clone(&exec);
                async move {
                    if sent {
                        return None;
                    }
                    let snapshot = exec.info();
                    let finished = exec.done();
                    if !finished {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    Some((Ok(snapshot), finished))
                }
            },
        ))))
    }

    async fn echo_binaries_observably_intermediate(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::EchoBinariesObservably_IntermediateStream>, Status> {
        let value = request.into_inner().value;
        let exec = self.echo(&value)?;
        if exec.done() {
            return Ok(Response::new(Box::pin(stream::empty())));
        }
        let shared = Arc::clone(&self.shared);
        Ok(Response::new(Box::pin(stream::unfold(None, move |seen| {
            let exec = Arc::clone(&exec);
            let shared = Arc::clone(&shared);
            async move { next_echo_binary(&exec, &shared, seen).await }
        }))))
    }

    async fn echo_binaries_observably_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<EchoBinariesObservablyResponses>, Status> {
        let value = request.into_inner().value;
        let exec = self.echo(&value)?;
        if !exec.done() {
            return Err(support::not_finished());
        }
        let mut joint = Vec::new();
        for parameter in &exec.parameters {
            joint.extend(parameter);
        }
        Ok(Response::new(EchoBinariesObservablyResponses {
            joint_binary: Some(self.shared.pack_binary(&joint)),
        }))
    }

    async fn echo_binary_and_metadata_string(
        &self,
        request: Request<EchoBinaryAndMetadataStringParameters>,
    ) -> Result<Response<EchoBinaryAndMetadataStringResponses>, Status> {
        let parsed = metadata::extract(request.metadata());
        let metadata_value = metadata_string(&parsed)?;
        let Some(binary) = request.into_inner().binary else {
            return Err(error::validation(ECHO_META, "Missing parameter 'Binary'"));
        };
        let bytes = self.shared.read_binary(&binary, ECHO_META)?;
        Ok(Response::new(EchoBinaryAndMetadataStringResponses {
            binary: Some(self.shared.pack_binary(&bytes)),
            string_metadata: Some(string(metadata_value)),
        }))
    }

    async fn get_binary_value_directly(
        &self,
        _request: Request<GetBinaryValueDirectlyParameters>,
    ) -> Result<Response<GetBinaryValueDirectlyResponses>, Status> {
        Ok(Response::new(GetBinaryValueDirectlyResponses {
            binary_value_directly: Some(binary_bytes(b"SiLA2_Test_String_Value".to_vec())),
        }))
    }

    async fn get_binary_value_download(
        &self,
        _request: Request<GetBinaryValueDownloadParameters>,
    ) -> Result<Response<GetBinaryValueDownloadResponses>, Status> {
        let bytes = DOWNLOAD_SAMPLE.repeat(100_000).into_bytes();
        Ok(Response::new(GetBinaryValueDownloadResponses {
            binary_value_download: Some(self.shared.pack_binary(&bytes)),
        }))
    }

    async fn get_fcp_affected_by_metadata_string(
        &self,
        _request: Request<GetFcpAffectedByMetadataStringParameters>,
    ) -> Result<Response<GetFcpAffectedByMetadataStringResponses>, Status> {
        Ok(Response::new(GetFcpAffectedByMetadataStringResponses {
            affected_calls: vec![string(
                "org.silastandard/test/BinaryTransferTest/v1/Command/EchoBinaryAndMetadataString",
            )],
        }))
    }
}

impl TestFeature {
    fn echo(&self, value: &str) -> Result<Arc<EchoBinaries>, Status> {
        let identifier = support::lookup_uuid(value)?;
        guard(&self.echoes)
            .get(&identifier)
            .cloned()
            .ok_or_else(|| support::unknown_execution(value))
    }
}

impl EchoBinaries {
    fn len_seconds(&self) -> f64 {
        u32_to_f64(u32::try_from(self.parameters.len()).unwrap_or(u32::MAX))
    }

    fn done(&self) -> bool {
        self.start.elapsed().as_secs_f64() >= self.len_seconds()
    }

    fn info(&self) -> fw::ExecutionInfo {
        let total = self.len_seconds();
        let remaining = (total - self.start.elapsed().as_secs_f64()).max(0.0);
        let progress = if total == 0.0 {
            1.0
        } else {
            (total - remaining) / total
        };
        let status = if self.done() {
            CommandStatus::FinishedSuccessfully
        } else {
            CommandStatus::Running
        };
        execution(status, progress, remaining)
    }
}

fn spawn_echo(exec: Arc<EchoBinaries>) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        for _ in 0..exec.parameters.len() {
            *guard(&exec.index) += 1;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

async fn next_echo_binary(
    exec: &EchoBinaries,
    shared: &Shared,
    seen: Option<u32>,
) -> Option<(
    Result<EchoBinariesObservablyIntermediateResponses, Status>,
    Option<u32>,
)> {
    loop {
        let index = *guard(&exec.index);
        if seen != Some(index)
            && usize::try_from(index).unwrap_or(usize::MAX) < exec.parameters.len()
        {
            let bytes = exec.parameters[usize::try_from(index).unwrap_or(0)].clone();
            let item = EchoBinariesObservablyIntermediateResponses {
                binary: Some(shared.pack_binary(&bytes)),
            };
            return Some((Ok(item), Some(index)));
        }
        if exec.done() {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn allowed(parameter: &str) -> bool {
    matches!(parameter, ECHO_VALUE | ECHO_LIST | ECHO_META | AUTH_UPLOAD)
}

fn validate_create(parameter: &str, parsed: &CallMetadata, shared: &Shared) -> Result<(), Status> {
    match parameter {
        ECHO_META if parsed.binary_string.is_none() => Err(framework(
            ErrorType::InvalidMetadata,
            "Missing metadata: 'org.silastandard/test/BinaryTransferTest/v1/Metadata/String'",
        )),
        AUTH_UPLOAD => auth::validate_upload(parsed, shared),
        _ => Ok(()),
    }
}

fn metadata_string(parsed: &CallMetadata) -> Result<String, Status> {
    let Some(metadata) = &parsed.binary_string else {
        return Err(framework(
            ErrorType::InvalidMetadata,
            "Missing metadata, expected 'org.silastandard/test/BinaryTransferTest/v1/Metadata/String'",
        ));
    };
    Ok(support::text_of(metadata.string.as_ref()).to_owned())
}

fn chunk_stream(
    inbound: Streaming<UploadChunkRequest>,
    shared: Arc<Shared>,
) -> RpcStream<UploadChunkResponse> {
    Box::pin(stream::unfold(
        (inbound, false),
        move |(mut inbound, closed)| {
            let shared = Arc::clone(&shared);
            async move {
                if closed {
                    return None;
                }
                match inbound.message().await {
                    Ok(Some(request)) => {
                        let response = accept_chunk(&shared, &request);
                        let failed = response.is_err();
                        Some((response, (inbound, failed)))
                    }
                    Ok(None) => None,
                    Err(status) => Some((Err(status), (inbound, true))),
                }
            }
        },
    ))
}

fn accept_chunk(
    shared: &Shared,
    request: &UploadChunkRequest,
) -> Result<UploadChunkResponse, Status> {
    let identifier = parsed_transfer_id(&request.binary_transfer_uuid)?;
    let mut uploads = guard(&shared.uploads);
    let Some(binary) = uploads.get_mut(&identifier) else {
        return Err(unknown_transfer(&request.binary_transfer_uuid));
    };
    if request.chunk_index >= binary.chunks {
        return Err(upload_failed(&format!(
            "Failed to upload chunk for {}:Chunk index {} out of range for binary with {} chunks",
            request.binary_transfer_uuid, request.chunk_index, binary.chunks
        )));
    }
    if request.payload.len() > TWO_MIB {
        return Err(upload_failed(&format!(
            "Failed to upload chunk for {}:Chunks must not be larger than 2 MiB",
            request.binary_transfer_uuid
        )));
    }
    let current = binary.parts.values().map(Vec::len).sum::<usize>();
    if current.saturating_add(request.payload.len())
        > usize::try_from(binary.size).unwrap_or(usize::MAX)
    {
        return Err(upload_failed(&format!(
            "Failed to upload chunk for {}:Tried to upload more bytes than originally announced",
            request.binary_transfer_uuid
        )));
    }
    if binary.parts.contains_key(&request.chunk_index) {
        return Err(upload_failed(&format!(
            "Tried to upload chunk {} twice for binary {}",
            request.chunk_index, request.binary_transfer_uuid
        )));
    }
    binary
        .parts
        .insert(request.chunk_index, request.payload.clone());
    Ok(UploadChunkResponse {
        binary_transfer_uuid: request.binary_transfer_uuid.clone(),
        chunk_index: request.chunk_index,
        lifetime_of_binary: Some(lifetime()),
    })
}

fn download_stream(
    inbound: Streaming<GetChunkRequest>,
    shared: Arc<Shared>,
) -> RpcStream<GetChunkResponse> {
    Box::pin(stream::unfold(inbound, move |mut inbound| {
        let shared = Arc::clone(&shared);
        async move {
            match inbound.message().await {
                Ok(Some(request)) => Some((read_chunk(&shared, &request), inbound)),
                Ok(None) => None,
                Err(status) => Some((Err(status), inbound)),
            }
        }
    }))
}

fn read_chunk(shared: &Shared, request: &GetChunkRequest) -> Result<GetChunkResponse, Status> {
    let bytes = require_download(shared, &request.binary_transfer_uuid)?;
    if request.length > u32::try_from(TWO_MIB).unwrap_or(u32::MAX) {
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

fn require_download(shared: &Shared, identifier: &str) -> Result<Vec<u8>, Status> {
    let parsed = parsed_transfer_id(identifier)?;
    guard(&shared.downloads)
        .get(&parsed)
        .cloned()
        .ok_or_else(|| unknown_transfer(identifier))
}

fn remove_upload(shared: &Shared, identifier: &str) -> Result<(), Status> {
    let parsed = parsed_transfer_id(identifier)?;
    if guard(&shared.uploads).remove(&parsed).is_some() {
        Ok(())
    } else {
        Err(unknown_transfer(identifier))
    }
}

fn remove_download(shared: &Shared, identifier: &str) -> Result<(), Status> {
    let parsed = parsed_transfer_id(identifier)?;
    if guard(&shared.downloads).remove(&parsed).is_some() {
        Ok(())
    } else {
        Err(unknown_transfer(identifier))
    }
}

fn parsed_transfer_id(identifier: &str) -> Result<Uuid, Status> {
    support::parse_uuid(identifier)
        .ok_or_else(|| invalid_transfer(&format!("Given id is not a valid UUID: {identifier}")))
}

fn unknown_transfer(identifier: &str) -> Status {
    invalid_transfer(&format!("No binary exists with UUID {identifier}"))
}

fn invalid_transfer(message: &str) -> Status {
    binary_transfer_status(BinaryError::InvalidBinaryTransferUuid, message)
}

fn upload_failed(message: &str) -> Status {
    binary_transfer_status(BinaryError::BinaryUploadFailed, message)
}

fn download_failed(message: &str) -> Status {
    binary_transfer_status(BinaryError::BinaryDownloadFailed, message)
}

fn u32_to_f64(value: u32) -> f64 {
    f64::from(value)
}
