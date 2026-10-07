//! Dynamic SiLA gRPC client.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use http::uri::PathAndQuery;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, ReflectMessage};
use tokio::sync::Mutex;
use tonic::client::Grpc;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Status};

use crate::error::A2aLabError;
use crate::json_object::JsonObject;
use crate::sila::consumer::codec::{
    self, EncodeStop, Uploads, collect_transfers, decode_fields, encode_base64, encode_fields,
    execution_uuid, parameters_without_metadata, read_execution_info, read_execution_uuid,
    replace_transfer,
};
use crate::sila::consumer::compile::{framework_pool, install_feature};
use crate::sila::consumer::fdl::parse_feature;
use crate::sila::consumer::model::{Element, FeatureModel};

const DOMAIN: &str = "SiLA2";
const CHUNK: usize = 1024 * 1024;
const SILA_SERVICE_XML: &str = include_str!("../standard/SiLAService.sila.xml");

/// A SiLA execution failure decoded from an aborted status.
#[derive(Debug, Clone)]
pub(crate) struct SilaFailure {
    pub kind: String,
    pub identifier: Option<String>,
    pub message: String,
}

/// A completed value or a SiLA execution error.
#[derive(Debug)]
pub(crate) enum Invoke {
    Done(JsonObject),
    Failed(SilaFailure),
}

enum RpcBody {
    Message(DynamicMessage),
    Failed(SilaFailure),
}

/// One connected server and the features it published.
pub(crate) struct Consumer {
    channel: Channel,
    pool: DescriptorPool,
    features: BTreeMap<String, FeatureModel>,
    intermediates: Arc<Mutex<BTreeMap<String, Vec<serde_json::Value>>>>,
}

impl Consumer {
    /// Opens TLS to `host` and loads every feature the server implements.
    pub(crate) async fn connect(
        host: &str,
        port: u16,
        expected_uuid: &str,
        authority_pem: &[u8],
    ) -> Result<Self, A2aLabError> {
        let channel = tls_channel(&format!("https://{host}:{port}"), authority_pem).await?;
        Self::load(channel, expected_uuid).await
    }

    /// Opens plaintext HTTP/2. Used for the local Caddy h2c gateway only.
    pub(crate) async fn connect_unencrypted(
        host: &str,
        port: u16,
        expected_uuid: &str,
    ) -> Result<Self, A2aLabError> {
        let channel = plain_channel(&format!("http://{host}:{port}")).await?;
        Self::load(channel, expected_uuid).await
    }

    async fn load(channel: Channel, expected_uuid: &str) -> Result<Self, A2aLabError> {
        let mut consumer = Self {
            channel,
            pool: framework_pool()?.clone(),
            features: BTreeMap::new(),
            intermediates: std::sync::Arc::new(Mutex::new(BTreeMap::new())),
        };
        let service = parse_feature(SILA_SERVICE_XML)?;
        install_feature(&mut consumer.pool, &service)?;
        let service_id = service.fqi();
        consumer.features.insert(service_id.clone(), service);
        let listed = consumer.feature_list(&service_id).await?;
        for feature_id in listed {
            if feature_id == service_id {
                continue;
            }
            let xml = consumer.feature_xml(&service_id, &feature_id).await?;
            let model = parse_feature(&xml)?;
            install_feature(&mut consumer.pool, &model)?;
            consumer.features.insert(model.fqi(), model);
        }
        let actual = consumer.string_property(&service_id, "ServerUUID").await?;
        if !actual.eq_ignore_ascii_case(expected_uuid) {
            return Err(A2aLabError::protocol(format!(
                "server UUID {actual} did not match {expected_uuid}"
            )));
        }
        Ok(consumer)
    }

    pub(crate) fn features(&self) -> &BTreeMap<String, FeatureModel> {
        &self.features
    }

    pub(crate) async fn string_property(
        &self,
        feature_id: &str,
        name: &str,
    ) -> Result<String, A2aLabError> {
        match self
            .read_property(feature_id, name, &JsonObject::empty())
            .await?
        {
            Invoke::Done(object) => object
                .as_map()
                .get("value")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| A2aLabError::protocol(format!("{name} was not a string"))),
            Invoke::Failed(error) => Err(A2aLabError::protocol(error.message)),
        }
    }

    pub(crate) async fn call(
        &self,
        feature_id: &str,
        command: &str,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let model = feature
            .command(command)
            .filter(|command| !command.observable)
            .ok_or_else(|| A2aLabError::not_found("task", command))?
            .clone();
        let headers = self.metadata_headers(input)?;
        let message = self
            .encode_parameters(
                feature,
                &format!("{command}_Parameters"),
                &model.parameters,
                input,
                &headers,
            )
            .await?;
        let body = self
            .unary(&service_path(feature), command, message, &headers)
            .await?;
        self.finish(feature_id, &model.responses, body, false).await
    }

    pub(crate) async fn read_property(
        &self,
        feature_id: &str,
        name: &str,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let data_type = feature
            .property(name)
            .filter(|property| !property.observable)
            .ok_or_else(|| A2aLabError::not_found("task", name))?
            .data_type
            .clone();
        let headers = self.metadata_headers(input)?;
        let rpc = format!("Get_{name}");
        let message = self
            .encode_parameters(
                feature,
                &format!("{rpc}_Parameters"),
                &[],
                &JsonObject::empty(),
                &headers,
            )
            .await?;
        let body = self
            .unary(&service_path(feature), &rpc, message, &headers)
            .await?;
        let elements = [Element {
            identifier: name.to_owned(),
            data_type,
        }];
        self.finish(feature_id, &elements, body, true).await
    }

    pub(crate) async fn start_observable(
        &self,
        feature_id: &str,
        command: &str,
        input: &JsonObject,
    ) -> Result<String, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let parameters = feature
            .command(command)
            .filter(|command| command.observable)
            .ok_or_else(|| A2aLabError::not_found("task", command))?
            .parameters
            .clone();
        let headers = self.metadata_headers(input)?;
        let message = self
            .encode_parameters(
                feature,
                &format!("{command}_Parameters"),
                &parameters,
                input,
                &headers,
            )
            .await?;
        let body = self
            .unary(&service_path(feature), command, message, &headers)
            .await?;
        let RpcBody::Message(message) = body else {
            return Err(A2aLabError::protocol(format!("{command} was not accepted")));
        };
        let execution = read_execution_uuid(&message)?;
        if !model_has_intermediate(feature, command) {
            return Ok(execution);
        }
        self.watch_intermediates(feature, command, &execution);
        Ok(execution)
    }

    pub(crate) async fn poll(
        &self,
        feature_id: &str,
        command: &str,
        execution: &str,
    ) -> Result<Poll, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let responses = feature
            .command(command)
            .ok_or_else(|| A2aLabError::not_found("task", command))?
            .responses
            .clone();
        let request = execution_uuid(&self.pool, execution)?;
        let info = self
            .first_stream(
                &service_path(feature),
                &format!("{command}_Info"),
                request.clone(),
            )
            .await?;
        let snapshot = read_execution_info(&info);
        let mut poll = Poll {
            status: snapshot.status,
            progress: snapshot.progress,
            result: None,
            failure: None,
            intermediates: Vec::new(),
        };
        if snapshot.status >= 2 {
            let body = self
                .unary(
                    &service_path(feature),
                    &format!("{command}_Result"),
                    request,
                    &[],
                )
                .await?;
            match self.finish(feature_id, &responses, body, false).await? {
                Invoke::Done(object) => poll.result = Some(object),
                Invoke::Failed(error) => poll.failure = Some(error),
            }
            let stored = self.intermediates.lock().await.get(execution).cloned();
            if let Some(values) = stored.filter(|values| !values.is_empty()) {
                poll.intermediates = values;
            }
        }
        Ok(poll)
    }

    #[allow(dead_code)]
    pub(crate) async fn subscribe_property(
        &self,
        feature_id: &str,
        name: &str,
        count: usize,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let data_type = feature
            .property(name)
            .filter(|property| property.observable)
            .ok_or_else(|| A2aLabError::not_found("task", name))?
            .data_type
            .clone();
        let headers = self.metadata_headers(input)?;
        let rpc = format!("Subscribe_{name}");
        let message = self
            .encode_parameters(
                feature,
                &format!("{rpc}_Parameters"),
                &[],
                &JsonObject::empty(),
                &headers,
            )
            .await?;
        let messages = collect_server_stream(
            &self.channel,
            &self.pool,
            &service_path(feature),
            &rpc,
            message,
            count.max(1),
        )
        .await?;
        let elements = [Element {
            identifier: name.to_owned(),
            data_type,
        }];
        let mut values = Vec::new();
        for message in messages {
            let object = decode_fields(feature, &elements, &message)?;
            values.push(
                object
                    .into_values()
                    .next()
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        let body = if count <= 1 {
            serde_json::json!({"value": values.first().cloned().unwrap_or(serde_json::Value::Null)})
        } else {
            serde_json::json!({"values": values})
        };
        Ok(Invoke::Done(JsonObject::try_from_value(body)?))
    }

    pub(crate) async fn affected_calls(
        &self,
        feature_id: &str,
        metadata: &str,
    ) -> Result<Vec<String>, A2aLabError> {
        let feature = self.feature(feature_id)?;
        let rpc = format!("Get_FCPAffectedByMetadata_{metadata}");
        let message = self
            .encode_parameters(
                feature,
                &format!("{rpc}_Parameters"),
                &[],
                &JsonObject::empty(),
                &[],
            )
            .await?;
        let body = self
            .unary(&service_path(feature), &rpc, message, &[])
            .await?;
        let RpcBody::Message(message) = body else {
            return Err(A2aLabError::protocol(format!("{rpc} failed")));
        };
        let elements = [Element {
            identifier: "AffectedCalls".to_owned(),
            data_type: crate::sila::consumer::model::SilaType::List(Box::new(
                crate::sila::consumer::model::SilaType::Basic(
                    crate::sila::consumer::model::Basic::String,
                ),
            )),
        }];
        let object = decode_fields(feature, &elements, &message)?;
        Ok(object
            .get("AffectedCalls")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn watch_intermediates(&self, feature: &FeatureModel, command: &str, execution: &str) {
        let Some(command_model) = feature.command(command) else {
            return;
        };
        if command_model.intermediate.is_empty() {
            return;
        }
        let elements = command_model.intermediate.clone();
        let feature = feature.clone();
        let channel = self.channel.clone();
        let pool = self.pool.clone();
        let store = Arc::clone(&self.intermediates);
        let service = service_path(&feature);
        let method = format!("{command}_Intermediate");
        let execution = execution.to_owned();
        let request = execution_uuid(&pool, &execution).ok();
        tokio::spawn(async move {
            let Some(request) = request else {
                return;
            };
            let Ok(messages) =
                collect_server_stream(&channel, &pool, &service, &method, request, 8).await
            else {
                return;
            };
            let mut values = Vec::new();
            for message in messages {
                if let Ok(object) = decode_fields(&feature, &elements, &message) {
                    values.push(serde_json::Value::Object(object));
                }
            }
            store.lock().await.insert(execution, values);
        });
    }

    pub(crate) async fn cancel(&self, execution: &str) -> Result<(), A2aLabError> {
        let feature = self
            .features
            .values()
            .find(|feature| feature.identifier == "CancelController")
            .ok_or_else(|| {
                A2aLabError::protocol(
                    "feature `org.silastandard/core/commands/CancelController/v1` was not found",
                )
            })?;
        let command = ["Cancel", "CancelCommand"]
            .into_iter()
            .find(|command| feature.command(command).is_some())
            .ok_or_else(|| A2aLabError::unavailable("task is not cancelable"))?;
        let input = JsonObject::parse(&format!(r#"{{"CommandExecutionUUID":"{execution}"}}"#))?;
        let feature_id = feature.fqi();
        match self.call(&feature_id, command, &input).await? {
            Invoke::Done(_) => Ok(()),
            Invoke::Failed(error) => Err(A2aLabError::protocol(format!(
                "{} {}",
                error.kind, error.message
            ))),
        }
    }

    async fn feature_list(&self, service_id: &str) -> Result<Vec<String>, A2aLabError> {
        match self
            .read_property(service_id, "ImplementedFeatures", &JsonObject::empty())
            .await?
        {
            Invoke::Done(object) => Ok(object
                .as_map()
                .get("value")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default()),
            Invoke::Failed(error) => Err(A2aLabError::protocol(error.message)),
        }
    }

    async fn feature_xml(&self, service_id: &str, feature_id: &str) -> Result<String, A2aLabError> {
        let input = JsonObject::parse(&format!(r#"{{"FeatureIdentifier":"{feature_id}"}}"#))?;
        match self
            .call(service_id, "GetFeatureDefinition", &input)
            .await?
        {
            Invoke::Done(object) => object
                .as_map()
                .get("FeatureDefinition")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| A2aLabError::protocol("GetFeatureDefinition returned no XML")),
            Invoke::Failed(error) => Err(A2aLabError::protocol(error.message)),
        }
    }

    fn feature(&self, feature_id: &str) -> Result<&FeatureModel, A2aLabError> {
        self.features
            .get(feature_id)
            .ok_or_else(|| A2aLabError::not_found("feature", feature_id))
    }

    async fn encode_parameters(
        &self,
        feature: &FeatureModel,
        message: &str,
        elements: &[Element],
        input: &JsonObject,
        headers: &[(String, Vec<u8>)],
    ) -> Result<DynamicMessage, A2aLabError> {
        let values = parameters_without_metadata(input);
        let qualified = format!("{}.{message}", feature.package());
        let mut uploads = Uploads::default();
        loop {
            match encode_fields(
                &self.pool, feature, &qualified, elements, &values, &uploads, "",
            ) {
                Ok(message) => return Ok(message),
                Err(EncodeStop::Upload { parameter, bytes }) => {
                    let fqi = format!(
                        "{}/Command/{}/Parameter/{parameter}",
                        feature.fqi(),
                        message.trim_end_matches("_Parameters")
                    );
                    let uuid = self.upload(&fqi, &bytes, headers).await?;
                    uploads.ready.push((bytes, uuid));
                }
                Err(error) => return Err(error.into_error()),
            }
        }
    }

    fn metadata_headers(&self, input: &JsonObject) -> Result<Vec<(String, Vec<u8>)>, A2aLabError> {
        let Some(metadata) = input.as_map().get("metadata") else {
            return Ok(Vec::new());
        };
        let serde_json::Value::Object(entries) = metadata else {
            return Err(A2aLabError::invalid(
                "metadata",
                "must be an object of fully qualified metadata identifiers",
            ));
        };
        let mut headers = Vec::new();
        for (name, value) in entries {
            headers.push(self.one_metadata(name, value)?);
        }
        Ok(headers)
    }

    fn one_metadata(
        &self,
        fqi: &str,
        value: &serde_json::Value,
    ) -> Result<(String, Vec<u8>), A2aLabError> {
        let (feature_id, identifier) = fqi.rsplit_once("/Metadata/").ok_or_else(|| {
            A2aLabError::invalid(
                "metadata",
                "key must be a fully qualified metadata identifier",
            )
        })?;
        let feature = self.feature(feature_id)?;
        let metadata = feature.metadata(identifier).ok_or_else(|| {
            A2aLabError::invalid("metadata", format!("{identifier} is not declared"))
        })?;
        let mut values = std::collections::HashMap::new();
        values.insert(identifier.to_owned(), value.clone());
        let uploads = Uploads::default();
        let message = encode_fields(
            &self.pool,
            feature,
            &format!("{}.Metadata_{identifier}", feature.package()),
            &[Element {
                identifier: identifier.to_owned(),
                data_type: metadata.data_type.clone(),
            }],
            &values,
            &uploads,
            "",
        )
        .map_err(EncodeStop::into_error)?;
        let header = format!("sila-{}-bin", fqi.replace('/', "-")).to_ascii_lowercase();
        Ok((header, message.encode_to_vec()))
    }

    async fn finish(
        &self,
        feature_id: &str,
        elements: &[Element],
        body: RpcBody,
        property: bool,
    ) -> Result<Invoke, A2aLabError> {
        let message = match body {
            RpcBody::Failed(error) if error.kind == "validation" => {
                return Err(A2aLabError::invalid(
                    "input",
                    format!(
                        "{}: {}",
                        error.identifier.unwrap_or_default(),
                        error.message
                    ),
                ));
            }
            RpcBody::Failed(error) => return Ok(Invoke::Failed(error)),
            RpcBody::Message(message) => message,
        };
        let feature = self.feature(feature_id)?;
        let mut object = decode_fields(feature, elements, &message)?;
        let transfers = collect_transfers(&serde_json::Value::Object(object.clone()));
        for uuid in transfers {
            let bytes = self.download(&uuid).await?;
            let encoded = encode_base64(&bytes);
            let mut value = serde_json::Value::Object(object);
            replace_transfer(&mut value, &uuid, &encoded);
            object = match value {
                serde_json::Value::Object(map) => map,
                _ => {
                    return Err(A2aLabError::protocol(
                        "binary download replaced a non-object",
                    ));
                }
            };
        }
        let value = if property {
            let inner = object
                .into_values()
                .next()
                .unwrap_or(serde_json::Value::Null);
            JsonObject::try_from_value(serde_json::json!({"value": inner}))?
        } else {
            JsonObject::try_from_value(serde_json::Value::Object(object))?
        };
        Ok(Invoke::Done(value))
    }

    async fn upload(
        &self,
        parameter: &str,
        bytes: &[u8],
        headers: &[(String, Vec<u8>)],
    ) -> Result<String, A2aLabError> {
        let chunks = bytes.len().div_ceil(CHUNK).max(1);
        let mut request = empty_message(&self.pool, "sila2.org.silastandard.CreateBinaryRequest")?;
        set(
            &mut request,
            "binarySize",
            prost_reflect::Value::U64(bytes.len() as u64),
        )?;
        set(
            &mut request,
            "chunkCount",
            prost_reflect::Value::U32(u32::try_from(chunks).unwrap_or(u32::MAX)),
        )?;
        set(
            &mut request,
            "parameterIdentifier",
            prost_reflect::Value::String(parameter.to_owned()),
        )?;
        let created = self
            .unary(
                "/sila2.org.silastandard.BinaryUpload",
                "CreateBinary",
                request,
                headers,
            )
            .await?;
        let RpcBody::Message(created) = created else {
            return Err(A2aLabError::protocol("CreateBinary was rejected"));
        };
        let uuid = string_field(&created, "binaryTransferUUID")?;
        let mut chunks_out = Vec::new();
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            let mut message =
                empty_message(&self.pool, "sila2.org.silastandard.UploadChunkRequest")?;
            set(
                &mut message,
                "binaryTransferUUID",
                prost_reflect::Value::String(uuid.clone()),
            )?;
            set(
                &mut message,
                "chunkIndex",
                prost_reflect::Value::U32(u32::try_from(index).unwrap_or(u32::MAX)),
            )?;
            set(
                &mut message,
                "payload",
                prost_reflect::Value::Bytes(prost_reflect::bytes::Bytes::copy_from_slice(chunk)),
            )?;
            chunks_out.push(message);
        }
        self.bidi(
            "/sila2.org.silastandard.BinaryUpload",
            "UploadChunk",
            chunks_out,
            headers,
        )
        .await?;
        Ok(uuid)
    }

    async fn download(&self, uuid: &str) -> Result<Vec<u8>, A2aLabError> {
        let mut info = empty_message(&self.pool, "sila2.org.silastandard.GetBinaryInfoRequest")?;
        set(
            &mut info,
            "binaryTransferUUID",
            prost_reflect::Value::String(uuid.to_owned()),
        )?;
        let info = self
            .unary(
                "/sila2.org.silastandard.BinaryDownload",
                "GetBinaryInfo",
                info,
                &[],
            )
            .await?;
        let RpcBody::Message(info) = info else {
            return Err(A2aLabError::protocol("GetBinaryInfo was rejected"));
        };
        let size = u64_field(&info, "binarySize")?;
        let mut offset = 0u64;
        let mut collected = Vec::new();
        while offset < size {
            let length = u32::try_from((size - offset).min(CHUNK as u64)).unwrap_or(u32::MAX);
            let mut request = empty_message(&self.pool, "sila2.org.silastandard.GetChunkRequest")?;
            set(
                &mut request,
                "binaryTransferUUID",
                prost_reflect::Value::String(uuid.to_owned()),
            )?;
            set(&mut request, "offset", prost_reflect::Value::U64(offset))?;
            set(&mut request, "length", prost_reflect::Value::U32(length))?;
            let chunks = self
                .bidi(
                    "/sila2.org.silastandard.BinaryDownload",
                    "GetChunk",
                    vec![request],
                    &[],
                )
                .await?;
            if chunks.is_empty() {
                return Err(A2aLabError::protocol("binary download returned no chunk"));
            }
            let payload = bytes_field(&chunks[0], "payload")?;
            offset += payload.len() as u64;
            collected.extend_from_slice(&payload);
        }
        Ok(collected)
    }

    async fn unary(
        &self,
        service: &str,
        method: &str,
        message: DynamicMessage,
        headers: &[(String, Vec<u8>)],
    ) -> Result<RpcBody, A2aLabError> {
        let descriptor = response_descriptor(&self.pool, service, method)?;
        let path = PathAndQuery::try_from(format!("{service}/{method}"))
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let mut client = Grpc::new(self.channel.clone());
        client
            .ready()
            .await
            .map_err(|error| A2aLabError::transport(format!("{error:#}")))?;
        let response = client
            .unary(
                request_with(message, headers)?,
                path,
                DynamicCodec::new(descriptor),
            )
            .await;
        status_or_message(&self.pool, response)
    }

    async fn first_stream(
        &self,
        service: &str,
        method: &str,
        message: DynamicMessage,
    ) -> Result<DynamicMessage, A2aLabError> {
        let descriptor = response_descriptor(&self.pool, service, method)?;
        let path = PathAndQuery::try_from(format!("{service}/{method}"))
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let mut client = Grpc::new(self.channel.clone());
        client
            .ready()
            .await
            .map_err(|error| A2aLabError::transport(format!("{error:#}")))?;
        let response = client
            .server_streaming(Request::new(message), path, DynamicCodec::new(descriptor))
            .await
            .map_err(|status| map_status(&self.pool, &status))?;
        let mut stream = response.into_inner();
        tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .map_err(|_| A2aLabError::unavailable("SiLA execution status timed out"))?
            .ok_or_else(|| A2aLabError::protocol("SiLA execution stream ended"))?
            .map_err(|status| map_status(&self.pool, &status))
    }

    async fn bidi(
        &self,
        service: &str,
        method: &str,
        messages: Vec<DynamicMessage>,
        headers: &[(String, Vec<u8>)],
    ) -> Result<Vec<DynamicMessage>, A2aLabError> {
        let descriptor = response_descriptor(&self.pool, service, method)?;
        let path = PathAndQuery::try_from(format!("{service}/{method}"))
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let mut client = Grpc::new(self.channel.clone());
        client
            .ready()
            .await
            .map_err(|error| A2aLabError::transport(format!("{error:#}")))?;
        let response = client
            .streaming(
                request_with(futures_util::stream::iter(messages), headers)?,
                path,
                DynamicCodec::new(descriptor),
            )
            .await
            .map_err(|status| map_status(&self.pool, &status))?;
        let mut stream = response.into_inner();
        let mut collected = Vec::new();
        while let Some(item) = stream.next().await {
            collected.push(item.map_err(|status| map_status(&self.pool, &status))?);
        }
        Ok(collected)
    }
}

/// Observable command status after one info read.
#[derive(Debug)]
pub(crate) struct Poll {
    pub status: i32,
    pub progress: Option<f64>,
    pub result: Option<JsonObject>,
    pub failure: Option<SilaFailure>,
    pub intermediates: Vec<serde_json::Value>,
}

fn service_path(feature: &FeatureModel) -> String {
    format!("/{}.{}", feature.package(), feature.identifier)
}

async fn plain_channel(endpoint: &str) -> Result<Channel, A2aLabError> {
    Endpoint::from_shared(endpoint.to_owned())
        .map_err(|error| A2aLabError::transport(format!("{error:#}")))?
        .connect()
        .await
        .map_err(|error| A2aLabError::transport(format!("{error:#}")))
}

async fn collect_server_stream(
    channel: &Channel,
    pool: &DescriptorPool,
    service: &str,
    method: &str,
    message: DynamicMessage,
    limit: usize,
) -> Result<Vec<DynamicMessage>, A2aLabError> {
    let descriptor = response_descriptor(pool, service, method)?;
    let path = PathAndQuery::try_from(format!("{service}/{method}"))
        .map_err(|error| A2aLabError::protocol(error.to_string()))?;
    let mut client = Grpc::new(channel.clone());
    client
        .ready()
        .await
        .map_err(|error| A2aLabError::transport(format!("{error:#}")))?;
    let response = client
        .server_streaming(Request::new(message), path, DynamicCodec::new(descriptor))
        .await
        .map_err(|status| map_status(pool, &status))?;
    let mut stream = response.into_inner();
    let mut collected = Vec::new();
    while collected.len() < limit {
        let next = tokio::time::timeout(Duration::from_secs(3), stream.next()).await;
        let Ok(Some(item)) = next else {
            break;
        };
        collected.push(item.map_err(|status| map_status(pool, &status))?);
    }
    Ok(collected)
}

fn model_has_intermediate(feature: &FeatureModel, command: &str) -> bool {
    feature
        .command(command)
        .is_some_and(|command| !command.intermediate.is_empty())
}

async fn tls_channel(endpoint: &str, authority_pem: &[u8]) -> Result<Channel, A2aLabError> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(authority_pem))
        .domain_name(DOMAIN);
    Endpoint::from_shared(endpoint.to_owned())
        .map_err(|error| A2aLabError::transport(format!("{error:#}")))?
        .tls_config(tls)
        .map_err(|error| A2aLabError::transport(format!("{error:#}")))?
        .connect()
        .await
        .map_err(|error| A2aLabError::transport(format!("{error:?}")))
}

fn response_descriptor(
    pool: &DescriptorPool,
    service: &str,
    method: &str,
) -> Result<MessageDescriptor, A2aLabError> {
    let service_name = service.trim_start_matches('/');
    let service = pool
        .services()
        .find(|item| item.full_name() == service_name)
        .ok_or_else(|| A2aLabError::protocol(format!("missing service {service_name}")))?;
    let method = service
        .methods()
        .find(|item| item.name() == method)
        .ok_or_else(|| A2aLabError::protocol(format!("missing method {method}")))?;
    Ok(method.output())
}

fn status_or_message(
    pool: &DescriptorPool,
    response: Result<tonic::Response<DynamicMessage>, Status>,
) -> Result<RpcBody, A2aLabError> {
    match response {
        Ok(response) => Ok(RpcBody::Message(response.into_inner())),
        Err(status) => {
            if let Some(error) = codec::sila_error_from_status(pool, &status) {
                Ok(RpcBody::Failed(error))
            } else {
                Err(A2aLabError::protocol(status.to_string()))
            }
        }
    }
}

fn map_status(pool: &DescriptorPool, status: &Status) -> A2aLabError {
    if let Some(error) = codec::sila_error_from_status(pool, status) {
        A2aLabError::protocol(format!("{} {}", error.kind, error.message))
    } else {
        A2aLabError::protocol(status.to_string())
    }
}

fn request_with<T>(message: T, headers: &[(String, Vec<u8>)]) -> Result<Request<T>, A2aLabError> {
    let mut request = Request::new(message);
    for (name, bytes) in headers {
        let key = tonic::metadata::MetadataKey::from_bytes(name.as_bytes())
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        request
            .metadata_mut()
            .insert_bin(key, MetadataValue::from_bytes(bytes));
    }
    Ok(request)
}

fn empty_message(pool: &DescriptorPool, name: &str) -> Result<DynamicMessage, A2aLabError> {
    let descriptor = pool
        .get_message_by_name(name)
        .ok_or_else(|| A2aLabError::protocol(format!("missing message {name}")))?;
    Ok(DynamicMessage::new(descriptor))
}

fn set(
    message: &mut DynamicMessage,
    name: &str,
    value: prost_reflect::Value,
) -> Result<(), A2aLabError> {
    let descriptor = message.descriptor();
    let field = descriptor.get_field_by_name(name).ok_or_else(|| {
        A2aLabError::protocol(format!("{} has no {name}", descriptor.full_name()))
    })?;
    message.set_field(&field, value);
    Ok(())
}

fn string_field(message: &DynamicMessage, name: &str) -> Result<String, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not a string")))
}

fn u64_field(message: &DynamicMessage, name: &str) -> Result<u64, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_u64()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not an integer")))
}

fn bytes_field(
    message: &DynamicMessage,
    name: &str,
) -> Result<prost_reflect::bytes::Bytes, A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| {
            A2aLabError::protocol(format!(
                "{} has no {name}",
                message.descriptor().full_name()
            ))
        })?;
    message
        .get_field(&field)
        .as_bytes()
        .cloned()
        .ok_or_else(|| A2aLabError::protocol(format!("{name} is not bytes")))
}

pub(crate) struct DynamicCodec {
    decode: MessageDescriptor,
}

impl DynamicCodec {
    pub(crate) fn new(decode: MessageDescriptor) -> Self {
        Self { decode }
    }
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            descriptor: self.decode.clone(),
        }
    }
}

pub(crate) struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|error| Status::internal(error.to_string()))
    }
}

pub(crate) struct DynamicDecoder {
    descriptor: MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        use prost_reflect::bytes::Buf;
        let bytes = src.copy_to_bytes(src.remaining());
        DynamicMessage::decode(self.descriptor.clone(), bytes)
            .map(Some)
            .map_err(|error| Status::internal(error.to_string()))
    }
}
