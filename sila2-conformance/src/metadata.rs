//! SiLA client metadata carried in gRPC binary headers.

use prost::Message;
use tonic::metadata::{KeyRef, MetadataMap};

use crate::wire::sila2::org::silastandard::core::authorizationservice::v1::MetadataAccessToken;
use crate::wire::sila2::org::silastandard::test::binarytransfertest::v1::MetadataString;
use crate::wire::sila2::org::silastandard::test::metadataprovider::v1::{
    MetadataStringMetadata, MetadataTwoIntegersMetadata,
};

#[derive(Clone, Debug, Default)]
pub struct CallMetadata {
    pub string_metadata: Option<MetadataStringMetadata>,
    pub two_integers: Option<MetadataTwoIntegersMetadata>,
    pub access_token: Option<MetadataAccessToken>,
    pub binary_string: Option<MetadataString>,
}

pub fn extract(map: &MetadataMap) -> CallMetadata {
    let mut parsed = CallMetadata::default();
    for key in map.keys() {
        let name = key_name(&key);
        let Some((feature, metadata)) = sila_parts(name) else {
            continue;
        };
        let Some(bytes) = map.get_bin(name).and_then(|value| value.to_bytes().ok()) else {
            continue;
        };
        decode_known(&mut parsed, feature, metadata, &bytes);
    }
    parsed
}

pub fn has_sila_metadata(map: &MetadataMap) -> bool {
    map.keys().any(|key| key_name(&key).starts_with("sila-"))
}

fn decode_known(parsed: &mut CallMetadata, feature: &str, metadata: &str, bytes: &[u8]) {
    match (feature, metadata) {
        ("metadataprovider", "stringmetadata") => {
            parsed.string_metadata = decode(bytes);
        }
        ("metadataprovider", "twointegersmetadata") => {
            parsed.two_integers = decode(bytes);
        }
        ("authorizationservice", "accesstoken") => {
            parsed.access_token = decode(bytes);
        }
        ("binarytransfertest", "string") => {
            parsed.binary_string = decode(bytes);
        }
        _ => {}
    }
}

fn decode<T: Message + Default>(bytes: &[u8]) -> Option<T> {
    T::decode(bytes).ok()
}

fn sila_parts(key: &str) -> Option<(&str, &str)> {
    let mut parts = key.split('-');
    let collected = [
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ];
    if parts.next().is_some() {
        return None;
    }
    let [
        Some("sila"),
        Some(_),
        Some(_),
        Some(feature),
        Some(_),
        Some("metadata"),
        Some(metadata),
        Some("bin"),
    ] = collected
    else {
        return None;
    };
    Some((feature, metadata))
}

fn key_name<'a>(key: &KeyRef<'a>) -> &'a str {
    match key {
        KeyRef::Ascii(value) => value.as_str(),
        KeyRef::Binary(value) => value.as_str(),
    }
}
