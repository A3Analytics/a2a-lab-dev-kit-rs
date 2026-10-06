//! SiLA error encoding. Part B carries a standard-base64 `SiLAError` in an aborted status.

use prost::Message;
use tonic::{Code, Status};

use crate::error::A2aLabError;
use crate::sila::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::sila::wire::sila2::org::silastandard::si_la_error::Error as SiLaChoice;
use crate::sila::wire::sila2::org::silastandard::{
    DefinedExecutionError, FrameworkError, SiLaError, UndefinedExecutionError, ValidationError,
};

const LAB_FEATURE: &str = "com.a3analytics/lab/LabOperations/v1";
const CANCEL_FEATURE: &str = "org.silastandard/core/commands/CancelController/v1";

pub(crate) fn reject_metadata<T>(request: &tonic::Request<T>) -> Result<(), Status> {
    let blocked = request.metadata().keys().any(|key| {
        let name = match key {
            tonic::metadata::KeyRef::Ascii(key) => key.as_str(),
            tonic::metadata::KeyRef::Binary(key) => key.as_str(),
        };
        name.starts_with("sila-")
    });
    if blocked {
        return Err(framework(
            ErrorType::NoMetadataAllowed,
            "this server does not accept SiLA client metadata",
        ));
    }
    Ok(())
}

pub(crate) fn validation(parameter: impl Into<String>, message: impl Into<String>) -> Status {
    aborted(SiLaChoice::ValidationError(ValidationError {
        parameter: parameter.into(),
        message: message.into(),
    }))
}

pub(crate) fn defined(feature: &str, identifier: &str, message: impl Into<String>) -> Status {
    aborted(SiLaChoice::DefinedExecutionError(DefinedExecutionError {
        error_identifier: format!("{feature}/DefinedExecutionError/{identifier}"),
        message: message.into(),
    }))
}

pub(crate) fn lab_defined(identifier: &str, message: impl Into<String>) -> Status {
    defined(LAB_FEATURE, identifier, message)
}

pub(crate) fn cancel_defined(identifier: &str, message: impl Into<String>) -> Status {
    defined(CANCEL_FEATURE, identifier, message)
}

pub(crate) fn undefined(message: impl Into<String>) -> Status {
    aborted(SiLaChoice::UndefinedExecutionError(
        UndefinedExecutionError {
            message: message.into(),
        },
    ))
}

pub(crate) fn framework(kind: ErrorType, message: impl Into<String>) -> Status {
    aborted(SiLaChoice::FrameworkError(FrameworkError {
        error_type: kind as i32,
        message: message.into(),
    }))
}

pub(crate) fn lab_error(error: A2aLabError, not_found: &str) -> Status {
    match error {
        A2aLabError::Invalid { message, .. } => undefined(message),
        A2aLabError::NotFound { kind, id } => {
            lab_defined(not_found, format!("{kind} `{id}` was not found"))
        }
        A2aLabError::Unavailable { message } => lab_defined("ProviderUnavailable", message),
        A2aLabError::Transport { message } | A2aLabError::Protocol { message } => {
            undefined(message)
        }
    }
}

pub(crate) fn error_of(status: &Status) -> SiLaError {
    if status.code() == Code::Aborted
        && let Some(bytes) = standard_base64_decode(status.message())
        && let Ok(error) = SiLaError::decode(bytes.as_slice())
    {
        return error;
    }
    SiLaError {
        error: Some(SiLaChoice::UndefinedExecutionError(
            UndefinedExecutionError {
                message: status.message().to_owned(),
            },
        )),
    }
}

fn aborted(error: SiLaChoice) -> Status {
    let body = SiLaError { error: Some(error) };
    let mut bytes = Vec::new();
    if body.encode(&mut bytes).is_err() {
        return Status::internal("SiLA error encoding failed");
    }
    Status::new(Code::Aborted, standard_base64(&bytes))
}

fn standard_base64_decode(text: &str) -> Option<Vec<u8>> {
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

fn standard_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    let (chunks, rest) = bytes.as_chunks::<3>();
    for chunk in chunks {
        let value = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 6) & 63) as usize] as char);
        encoded.push(TABLE[(value & 63) as usize] as char);
    }
    if !rest.is_empty() {
        let mut value = u32::from(rest[0]) << 16;
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        if rest.len() == 2 {
            value |= u32::from(rest[1]) << 8;
            encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
            encoded.push(TABLE[((value >> 6) & 63) as usize] as char);
            encoded.push('=');
        } else {
            encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
            encoded.push('=');
            encoded.push('=');
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use tonic::Request;

    use super::{error_of, reject_metadata, validation};
    use crate::sila::wire::sila2::org::silastandard::si_la_error::Error as SiLaChoice;

    #[test]
    fn validation_errors_name_the_parameter() {
        let status = validation(
            "org.silastandard/core/SiLAService/v1/Command/GetFeatureDefinition/Parameter/FeatureIdentifier",
            "must be a fully qualified feature identifier",
        );
        let Some(SiLaChoice::ValidationError(error)) = error_of(&status).error else {
            panic!("expected a validation error");
        };
        assert!(error.parameter.ends_with("FeatureIdentifier"));
    }

    #[test]
    fn sila_metadata_is_rejected() {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("sila-token", "x".parse().unwrap());
        assert!(reject_metadata(&request).is_err());
        let mut binary = Request::new(());
        binary.metadata_mut().insert_bin(
            "sila-token-bin",
            tonic::metadata::MetadataValue::from_bytes(b"x"),
        );
        assert!(reject_metadata(&binary).is_err());
    }
}
