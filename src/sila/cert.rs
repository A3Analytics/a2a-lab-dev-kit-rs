//! SiLA TLS material. The certificate common name is `SiLA2` and the server UUID is an extension.

use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, DnType, IsCa, KeyPair, KeyUsagePurpose,
    SanType,
};

use crate::error::A2aLabError;

const UUID_OID: &[u64] = &[1, 3, 6, 1, 4, 1, 58_583];

/// PEM material for the server listener and discovery trust anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilaCertificate {
    /// Server certificate PEM.
    pub cert_pem: String,
    /// Server private key PEM.
    pub key_pem: String,
    /// PEM clients use as the trust anchor.
    pub ca_pem: String,
}

impl SilaCertificate {
    /// Creates a self-signed development certificate for `server_uuid`.
    pub fn self_signed(server_uuid: &str) -> Result<Self, A2aLabError> {
        let key = KeyPair::generate().map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let mut params = CertificateParams::new(vec!["SiLA2".to_owned()])
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        params.distinguished_name.push(DnType::CommonName, "SiLA2");
        params.subject_alt_names =
            vec![SanType::DnsName("SiLA2".try_into().map_err(
                |error: rcgen::Error| A2aLabError::protocol(error.to_string()),
            )?)];
        params
            .custom_extensions
            .push(CustomExtension::from_oid_content(
                UUID_OID,
                uuid_extension(server_uuid),
            ));
        let cert = params
            .self_signed(&key)
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let cert_pem = cert.pem();
        let certificate = Self {
            key_pem: key.serialize_pem(),
            ca_pem: cert_pem.clone(),
            cert_pem,
        };
        certificate.check_profile(server_uuid)?;
        Ok(certificate)
    }

    /// Loads PEM files supplied by the operator and checks the SiLA certificate profile.
    pub fn from_pem(
        cert_pem: impl Into<String>,
        key_pem: impl Into<String>,
        ca_pem: impl Into<String>,
        server_uuid: &str,
    ) -> Result<Self, A2aLabError> {
        let certificate = Self {
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            ca_pem: ca_pem.into(),
        };
        certificate.check_profile(server_uuid)?;
        Ok(certificate)
    }

    fn check_profile(&self, server_uuid: &str) -> Result<(), A2aLabError> {
        if self.key_pem.is_empty() || self.ca_pem.is_empty() {
            return Err(A2aLabError::invalid(
                "certificate",
                "certificate, key, and CA PEM are required",
            ));
        }
        if !certificate_matches_profile(&self.cert_pem, server_uuid) {
            return Err(A2aLabError::invalid(
                "certificate",
                "must use CN SiLA2, a CA constraint, and the server UUID extension",
            ));
        }
        Ok(())
    }
}

pub(crate) fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[must_use]
pub fn certificate_contains_uuid(cert_pem: &str, server_uuid: &str) -> bool {
    der_contains(cert_pem, &uuid_extension(server_uuid))
}

/// Reports whether `cert_pem` carries the SiLA server certificate profile.
#[must_use]
pub fn certificate_matches_profile(cert_pem: &str, server_uuid: &str) -> bool {
    certificate_contains_uuid(cert_pem, server_uuid)
        && der_contains(cert_pem, b"SiLA2")
        && der_contains(cert_pem, &[0x01, 0x01, 0xff])
}

fn uuid_extension(server_uuid: &str) -> Vec<u8> {
    let bytes = server_uuid.as_bytes();
    let mut encoded = Vec::with_capacity(bytes.len() + 2);
    encoded.push(0x0c);
    encoded.push(u8::try_from(bytes.len()).unwrap_or(0));
    encoded.extend(bytes);
    encoded
}

fn der_contains(cert_pem: &str, needle: &[u8]) -> bool {
    let der = pem_body(cert_pem);
    let Some(bytes) = standard_base64_decode(&der) else {
        return false;
    };
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn pem_body(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("-----"))
        .collect()
}

fn standard_base64_decode(text: &str) -> Option<Vec<u8>> {
    let text: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut bytes = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(4) {
        let values = chunk
            .iter()
            .map(|character| decode_digit(*character))
            .collect::<Option<Vec<_>>>()?;
        let value = (values[0] << 18) | (values[1] << 12) | (values[2] << 6) | values[3];
        bytes.push(u8::try_from((value >> 16) & 0xff).unwrap_or(0));
        if chunk[2] != '=' {
            bytes.push(u8::try_from((value >> 8) & 0xff).unwrap_or(0));
        }
        if chunk[3] != '=' {
            bytes.push(u8::try_from(value & 0xff).unwrap_or(0));
        }
    }
    Some(bytes)
}

fn decode_digit(character: char) -> Option<u32> {
    match character {
        'A'..='Z' => Some(character as u32 - 'A' as u32),
        'a'..='z' => Some(character as u32 - 'a' as u32 + 26),
        '0'..='9' => Some(character as u32 - '0' as u32 + 52),
        '+' => Some(62),
        '/' => Some(63),
        '=' => Some(0),
        _ => None,
    }
}
