//! SiLA error statuses for the communication-tester server.

use prost::Message;
use tonic::{Code, Status};

use crate::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::wire::sila2::org::silastandard::si_la_error::Error as Choice;
use crate::wire::sila2::org::silastandard::{
    DefinedExecutionError, FrameworkError, SiLaError, UndefinedExecutionError, ValidationError,
};

pub fn validation(parameter: &str, message: &str) -> Status {
    fail(Choice::ValidationError(ValidationError {
        parameter: parameter.to_owned(),
        message: message.to_owned(),
    }))
}

pub fn defined(identifier: &str, message: &str) -> Status {
    fail(Choice::DefinedExecutionError(DefinedExecutionError {
        error_identifier: identifier.to_owned(),
        message: message.to_owned(),
    }))
}

pub fn undefined(message: &str) -> Status {
    fail(Choice::UndefinedExecutionError(UndefinedExecutionError {
        message: message.to_owned(),
    }))
}

pub fn framework(kind: ErrorType, message: &str) -> Status {
    fail(Choice::FrameworkError(FrameworkError {
        error_type: kind as i32,
        message: message.to_owned(),
    }))
}

pub fn metadata_rejected() -> Status {
    framework(
        ErrorType::NoMetadataAllowed,
        "this call does not accept SiLA client metadata",
    )
}

fn fail(error: Choice) -> Status {
    let mut bytes = Vec::new();
    let _ = SiLaError { error: Some(error) }.encode(&mut bytes);
    Status::new(Code::Aborted, base64(&bytes))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = match chunk {
            [a, b, c] => (u32::from(*a) << 16) | (u32::from(*b) << 8) | u32::from(*c),
            [a, b] => (u32::from(*a) << 16) | (u32::from(*b) << 8),
            [a] => u32::from(*a) << 16,
            _ => 0,
        };
        out.push(TABLE[((value >> 18) & 63) as usize] as char);
        out.push(TABLE[((value >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
