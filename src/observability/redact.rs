//! Redaction helpers for structured application logs.

/// Returns `true` when field names commonly carry secrets or media payloads.
#[must_use]
pub fn is_sensitive_field(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "authorization"
            | "token"
            | "access_token"
            | "refresh_token"
            | "api_key"
            | "apikey"
            | "password"
            | "secret"
            | "credential"
            | "credentials"
            | "bearer"
            | "embedding"
            | "embeddings"
            | "bytes"
            | "payload"
            | "image"
            | "frame"
            | "video"
            | "body"
            | "description"
            | "semantic_text"
    ) || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("credential")
        || normalized.contains("embedding")
}

/// Replaces sensitive values with a stable placeholder.
#[must_use]
pub fn redact_field(name: &str, value: &str) -> String {
    if is_sensitive_field(name) || looks_like_secret(value) {
        "<redacted>".to_owned()
    } else {
        value.to_owned()
    }
}

/// Replaces bearer tokens and long base64-like blobs.
#[must_use]
pub fn redact_free_text(value: &str) -> String {
    if looks_like_secret(value) {
        "<redacted>".to_owned()
    } else {
        value.to_owned()
    }
}

fn looks_like_secret(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .is_some_and(|token| token.chars().any(|character| !character.is_whitespace()))
    {
        return true;
    }
    if trimmed.len() >= 64 && trimmed.chars().all(is_secret_char) {
        return true;
    }
    false
}

fn is_secret_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=' | '-' | '_' | '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_prefix() {
        assert_eq!(
            redact_free_text("Bearer eyJhbGciOiJIUzI1NiJ9.payload"),
            "<redacted>"
        );
    }

    #[test]
    fn redacts_sensitive_field_names() {
        assert_eq!(redact_field("api_key", "sk-live-abc"), "<redacted>");
        assert_eq!(redact_field("operation", "list_tasks"), "list_tasks");
    }
}
