//! Process-wide tester state: server identity, tokens, and binaries.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::support::{self, TWO_MIB, binary_bytes, binary_transfer_uuid, guard};
use crate::wire::sila2::org::silastandard::{self as fw, binary};

pub struct Shared {
    pub server_uuid: Uuid,
    pub name: Mutex<String>,
    pub features: Vec<(String, &'static str)>,
    pub tokens: Mutex<HashMap<String, Instant>>,
    pub uploads: Mutex<HashMap<Uuid, Upload>>,
    pub downloads: Mutex<HashMap<Uuid, Vec<u8>>>,
}

pub struct Upload {
    pub chunks: u32,
    pub size: u64,
    pub parts: HashMap<u32, Vec<u8>>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            server_uuid: process_uuid(),
            name: Mutex::new("Test Server".to_owned()),
            features: implemented_features(),
            tokens: Mutex::new(HashMap::new()),
            uploads: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
        }
    }

    pub fn feature_definition(&self, identifier: &str) -> Option<&'static str> {
        self.features
            .iter()
            .find(|(id, _)| id == identifier)
            .map(|(_, xml)| *xml)
    }

    pub fn issue_token(&self) -> (String, i64) {
        let token = Uuid::new_v4().to_string();
        let lifetime = 600_i64;
        guard(&self.tokens).insert(token.clone(), Instant::now() + Duration::from_secs(600));
        (token, lifetime)
    }

    pub fn token_known(&self, token: &str) -> bool {
        guard(&self.tokens).contains_key(token)
    }

    pub fn token_valid(&self, token: &str) -> bool {
        guard(&self.tokens)
            .get(token)
            .is_some_and(|expiry| Instant::now() <= *expiry)
    }

    pub fn revoke_token(&self, token: &str) {
        guard(&self.tokens).remove(token);
    }

    pub fn get_uploaded(&self, identifier: Uuid) -> Option<Vec<u8>> {
        let uploads = guard(&self.uploads);
        let binary = uploads.get(&identifier)?;
        if binary.parts.len() != usize::try_from(binary.chunks).unwrap_or(usize::MAX) {
            return None;
        }
        let mut bytes = Vec::new();
        for index in 0..binary.chunks {
            bytes.extend(binary.parts.get(&index)?);
        }
        Some(bytes)
    }

    pub fn pack_binary(&self, bytes: &[u8]) -> fw::Binary {
        if bytes.len() < TWO_MIB {
            return binary_bytes(bytes.to_vec());
        }
        let identifier = Uuid::new_v4();
        guard(&self.downloads).insert(identifier, bytes.to_vec());
        binary_transfer_uuid(&identifier.to_string())
    }

    pub fn read_binary(
        &self,
        message: &fw::Binary,
        parameter: &str,
    ) -> Result<Vec<u8>, tonic::Status> {
        match &message.union {
            Some(binary::Union::Value(bytes)) => Ok(bytes.clone()),
            Some(binary::Union::BinaryTransferUuid(identifier)) => {
                self.binary_by_id(identifier, parameter)
            }
            None => Err(crate::error::validation(
                parameter,
                "Received empty Binary message. Either 'value' or 'binaryTransferUUID' is required.",
            )),
        }
    }

    fn binary_by_id(&self, identifier: &str, parameter: &str) -> Result<Vec<u8>, tonic::Status> {
        let Some(parsed) = support::parse_uuid(identifier) else {
            return Err(crate::error::validation(
                parameter,
                &format!("Not a valid UUID: {identifier}"),
            ));
        };
        self.get_uploaded(parsed).ok_or_else(|| {
            crate::error::validation(
                parameter,
                &format!("Failed to get binary with UUID {identifier}"),
            )
        })
    }
}

fn process_uuid() -> Uuid {
    static SERVER_UUID: OnceLock<Uuid> = OnceLock::new();
    *SERVER_UUID.get_or_init(Uuid::new_v4)
}

fn implemented_features() -> Vec<(String, &'static str)> {
    const FILES: &[&str] = &[
        include_str!("../fdl/AnyTypeTest.sila.xml"),
        include_str!("../fdl/AuthenticationService.sila.xml"),
        include_str!("../fdl/AuthenticationTest.sila.xml"),
        include_str!("../fdl/AuthorizationService.sila.xml"),
        include_str!("../fdl/BasicDataTypesTest.sila.xml"),
        include_str!("../fdl/BinaryTransferTest.sila.xml"),
        include_str!("../fdl/ErrorHandlingTest.sila.xml"),
        include_str!("../fdl/ListDataTypeTest.sila.xml"),
        include_str!("../fdl/MetadataConsumerTest.sila.xml"),
        include_str!("../fdl/MetadataProvider.sila.xml"),
        include_str!("../fdl/MultiClientTest.sila.xml"),
        include_str!("../fdl/ObservableCommandTest.sila.xml"),
        include_str!("../fdl/ObservablePropertyTest.sila.xml"),
        include_str!("../fdl/SiLAService.sila.xml"),
        include_str!("../fdl/StructureDataTypeTest.sila.xml"),
        include_str!("../fdl/UnobservableCommandTest.sila.xml"),
        include_str!("../fdl/UnobservablePropertyTest.sila.xml"),
    ];
    FILES.iter().filter_map(|xml| feature_entry(xml)).collect()
}

fn feature_entry(xml: &'static str) -> Option<(String, &'static str)> {
    let originator = xml_attr(xml, "Originator")?;
    let category = xml_attr(xml, "Category")?;
    let version = xml_attr(xml, "FeatureVersion")?;
    let identifier = first_identifier(xml)?;
    let major = version.split('.').next().unwrap_or(version);
    Some((
        format!("{originator}/{category}/{identifier}/v{major}"),
        xml,
    ))
}

fn xml_attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = xml.find(&key)? + key.len();
    let rest = xml.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end)
}

fn first_identifier(xml: &str) -> Option<&str> {
    let start = xml.find("<Identifier>")? + "<Identifier>".len();
    let rest = xml.get(start..)?;
    let end = rest.find("</Identifier>")?;
    Some(rest.get(..end)?.trim())
}
