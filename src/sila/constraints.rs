//! Constraint checks shared by SiLAService, CancelController, and discovery.

pub(crate) const EXECUTION_LIFETIME_SECONDS: i64 = 60;

pub(crate) fn feature_identifier(value: &str) -> bool {
    let mut parts = value.split('/');
    let Some(origin) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    if rest.len() < 3 {
        return false;
    }
    let version = rest[rest.len() - 1];
    let feature = rest[rest.len() - 2];
    dotted_identifier(origin)
        && rest[..rest.len() - 2]
            .iter()
            .all(|segment| identifier(segment))
        && feature_token(feature)
        && major_version(version)
}

pub(crate) fn execution_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let groups = [8, 4, 4, 4, 12];
    let mut index = 0;
    for (group, size) in groups.iter().enumerate() {
        if group > 0 {
            if bytes.get(index) != Some(&b'-') {
                return false;
            }
            index += 1;
        }
        for _ in 0..*size {
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

pub(crate) fn server_version(value: &str) -> bool {
    let (numbers, suffix) = match value.split_once('_') {
        Some((numbers, suffix)) => (numbers, Some(suffix)),
        None => (value, None),
    };
    if suffix.is_some_and(|suffix| {
        suffix.is_empty()
            || !suffix
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    }) {
        return false;
    }
    let mut parts = numbers.split('.');
    let major = parts.next();
    let minor = parts.next();
    let patch = parts.next();
    parts.next().is_none()
        && major.is_some_and(digits)
        && minor.is_some_and(digits)
        && patch.is_none_or(digits)
}

pub(crate) fn vendor_url(value: &str) -> bool {
    value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty())
}

pub(crate) fn bounded_characters(value: &str, limit: usize) -> bool {
    value.chars().count() <= limit
}

fn dotted_identifier(value: &str) -> bool {
    !value.is_empty() && value.split('.').all(identifier)
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

fn feature_token(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
}

fn major_version(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(digits)
}

fn digits(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_characters, execution_uuid, feature_identifier, server_version, vendor_url,
    };

    #[test]
    fn identifiers_match_the_sila_service_patterns() {
        assert!(feature_identifier("org.silastandard/core/SiLAService/v1"));
        assert!(!feature_identifier("SiLAService"));
        assert!(execution_uuid("11111111-1111-1111-1111-111111111111"));
        assert!(!execution_uuid("11111111-1111-1111-1111-11111111111G"));
        assert!(server_version("0.1.0"));
        assert!(server_version("1.2.3_lab"));
        assert!(!server_version("01.2"));
        assert!(vendor_url("http://a"));
        assert!(!vendor_url("ftp://a"));
        assert!(bounded_characters("héllo", 5));
        assert!(!bounded_characters("héllo", 4));
    }
}
