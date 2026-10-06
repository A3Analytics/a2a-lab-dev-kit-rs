//! Shared SiLA value constructors and execution-info helpers.

use std::pin::Pin;
use std::time::Duration;

use futures_util::Stream;
use prost::Message;
use tonic::{Code, Status};
use uuid::Uuid;

use crate::wire::sila2::org::silastandard::{
    self as fw, binary, binary_transfer_error, execution_info, framework_error,
};

pub type RpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub const TWO_MIB: usize = 2 * 1024 * 1024;
pub const TEST_REAL: f64 = 31_415_926.0 / 10_000_000.0;
const TOKEN_SECONDS: i64 = 600;

pub fn string(value: impl Into<String>) -> fw::String {
    fw::String {
        value: value.into(),
    }
}

pub fn integer(value: i64) -> fw::Integer {
    fw::Integer { value }
}

pub fn real(value: f64) -> fw::Real {
    fw::Real { value }
}

pub fn boolean(value: bool) -> fw::Boolean {
    fw::Boolean { value }
}

pub fn timezone(hours: i32) -> fw::Timezone {
    fw::Timezone { hours, minutes: 0 }
}

pub fn date(year: u32, month: u32, day: u32, hours: i32) -> fw::Date {
    fw::Date {
        day,
        month,
        year,
        timezone: Some(timezone(hours)),
    }
}

pub fn time_value(hour: u32, minute: u32, second: u32, millisecond: u32, hours: i32) -> fw::Time {
    fw::Time {
        second,
        minute,
        hour,
        timezone: Some(timezone(hours)),
        millisecond,
    }
}

pub fn timestamp(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millisecond: u32,
    hours: i32,
) -> fw::Timestamp {
    fw::Timestamp {
        second,
        minute,
        hour,
        day,
        month,
        year,
        timezone: Some(timezone(hours)),
        millisecond,
    }
}

pub fn binary_bytes(bytes: impl Into<Vec<u8>>) -> fw::Binary {
    fw::Binary {
        union: Some(binary::Union::Value(bytes.into())),
    }
}

pub fn binary_transfer_uuid(identifier: &str) -> fw::Binary {
    fw::Binary {
        union: Some(binary::Union::BinaryTransferUuid(identifier.to_owned())),
    }
}

pub fn lifetime() -> fw::Duration {
    fw::Duration {
        seconds: TOKEN_SECONDS,
        nanos: 0,
    }
}

pub fn duration_from_seconds(seconds: f64) -> fw::Duration {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let whole = seconds.floor();
    let nanos = ((seconds - whole) * 1_000_000_000.0).floor();
    fw::Duration {
        seconds: float_to_i64(whole),
        nanos: float_to_i32(nanos),
    }
}

pub fn sleep_duration(seconds: f64) -> Duration {
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds.min(86_400.0))
    } else {
        Duration::ZERO
    }
}

pub fn confirmation(identifier: Uuid) -> fw::CommandConfirmation {
    fw::CommandConfirmation {
        command_execution_uuid: Some(fw::CommandExecutionUuid {
            value: identifier.to_string(),
        }),
        lifetime_of_execution: None,
    }
}

pub fn execution(
    status: execution_info::CommandStatus,
    progress: f64,
    remaining: f64,
) -> fw::ExecutionInfo {
    fw::ExecutionInfo {
        command_status: status as i32,
        progress_info: Some(real(progress)),
        estimated_remaining_time: Some(duration_from_seconds(remaining)),
        updated_lifetime_of_execution: None,
    }
}

pub fn status_only(status: execution_info::CommandStatus) -> fw::ExecutionInfo {
    fw::ExecutionInfo {
        command_status: status as i32,
        progress_info: None,
        estimated_remaining_time: None,
        updated_lifetime_of_execution: None,
    }
}

pub fn parse_uuid(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value).ok()
}

pub fn invalid_execution_uuid(value: &str) -> Status {
    crate::error::framework(
        framework_error::ErrorType::InvalidCommandExecutionUuid,
        &format!("String is not a valid UUID: '{value}'"),
    )
}

pub fn unknown_execution(value: &str) -> Status {
    crate::error::framework(
        framework_error::ErrorType::InvalidCommandExecutionUuid,
        &format!("No command instance with UUID {value}"),
    )
}

pub fn not_finished() -> Status {
    crate::error::framework(
        framework_error::ErrorType::CommandExecutionNotFinished,
        "Command is still running",
    )
}

pub fn lookup_uuid(value: &str) -> Result<Uuid, Status> {
    parse_uuid(value).ok_or_else(|| invalid_execution_uuid(value))
}

pub fn binary_transfer_status(kind: binary_transfer_error::ErrorType, message: &str) -> Status {
    let error = fw::BinaryTransferError {
        error_type: kind as i32,
        message: message.to_owned(),
    };
    let mut bytes = Vec::new();
    let _ = error.encode(&mut bytes);
    Status::new(Code::Aborted, standard_base64(&bytes))
}

pub fn embed_message(message: &impl Message) -> Vec<u8> {
    let mut inner = Vec::new();
    let _ = message.encode(&mut inner);
    embed_bytes(&inner)
}

pub fn embed_bytes(inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x0a);
    write_varint(&mut out, inner.len());
    out.extend_from_slice(inner);
    out
}

pub fn embed_repeated(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend(embed_bytes(part));
    }
    out
}

pub fn any_message(type_xml: impl Into<String>, payload: Vec<u8>) -> fw::Any {
    fw::Any {
        r#type: type_xml.into(),
        payload,
    }
}

pub fn basic_any(basic: &str, payload: Vec<u8>) -> fw::Any {
    any_message(
        format!(
            r#"<DataType xmlns="http://www.sila-standard.org"><Basic>{basic}</Basic></DataType>"#
        ),
        payload,
    )
}

pub fn is_feature_identifier(value: &str) -> bool {
    let mut parts = value.split('/');
    let (Some(originator), Some(category), Some(identifier), Some(version)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    parts.next().is_none()
        && dotted_lowercase(originator)
        && dotted_lowercase(category)
        && pascal_identifier(identifier)
        && version.strip_prefix('v').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub fn is_lowercase_server_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let groups = [8, 4, 4, 4, 12];
    let mut index = 0;
    for (group, length) in groups.into_iter().enumerate() {
        if group > 0 {
            if bytes.get(index) != Some(&b'-') {
                return false;
            }
            index += 1;
        }
        for _ in 0..length {
            let Some(byte) = bytes.get(index) else {
                return false;
            };
            if !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase() {
                return false;
            }
            index += 1;
        }
    }
    index == bytes.len()
}

pub fn python_list(items: &[String]) -> String {
    let body = items
        .iter()
        .map(|item| format!("'{item}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

pub fn text_of(value: Option<&fw::String>) -> &str {
    value.map_or("", |item| item.value.as_str())
}

pub fn guard<'a, T>(mutex: &'a std::sync::Mutex<T>) -> std::sync::MutexGuard<'a, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn dotted_lowercase(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|character| character.is_ascii_lowercase() || character == '.')
}

fn pascal_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
}

fn write_varint(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

fn standard_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = match chunk {
            [first, second, third] => {
                (u32::from(*first) << 16) | (u32::from(*second) << 8) | u32::from(*third)
            }
            [first, second] => (u32::from(*first) << 16) | (u32::from(*second) << 8),
            [first] => u32::from(*first) << 16,
            _ => 0,
        };
        out.push(TABLE[usize::try_from((value >> 18) & 63).unwrap_or(0)] as char);
        out.push(TABLE[usize::try_from((value >> 12) & 63).unwrap_or(0)] as char);
        out.push(if chunk.len() > 1 {
            TABLE[usize::try_from((value >> 6) & 63).unwrap_or(0)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[usize::try_from(value & 63).unwrap_or(0)] as char
        } else {
            '='
        });
    }
    out
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn float_to_i64(value: f64) -> i64 {
    if value >= i64::MAX as f64 {
        i64::MAX
    } else if value <= i64::MIN as f64 {
        i64::MIN
    } else {
        value as i64
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn float_to_i32(value: f64) -> i32 {
    if value >= f64::from(i32::MAX) {
        i32::MAX
    } else if value <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}
