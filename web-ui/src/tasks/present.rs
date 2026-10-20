//! Format a lab task result for the Tasks page.

use serde_json::{Map, Value};

use crate::workbench::report::{self, ImagePreview};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presented {
    pub kind: &'static str,
    pub text: String,
    pub image: Option<ImagePreview>,
}

pub fn present(raw: &str) -> Option<Presented> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(from_json(value));
    }
    Some(from_text(trimmed))
}

fn from_json(value: Value) -> Presented {
    if let Some(text) = value.as_str() {
        return from_text(text);
    }
    if let Some(inner) = unwrap_text(&value) {
        let shown = from_text(inner);
        if shown.kind != "Text" {
            return shown;
        }
    }
    let image = image_from(&value);
    let shown = collapse(&value);
    Presented {
        kind: if image.is_some() { "Image" } else { "JSON" },
        text: serde_json::to_string_pretty(&shown).unwrap_or_else(|_| shown.to_string()),
        image,
    }
}

fn unwrap_text(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let (key, inner) = object.iter().next()?;
    if !matches!(key.as_str(), "value" | "body" | "text" | "content" | "html" | "xml") {
        return None;
    }
    inner.as_str().filter(|text| !text.is_empty())
}

fn from_text(raw: &str) -> Presented {
    if let Some(image) = image_from_data_url(raw) {
        return Presented {
            kind: "Image",
            text: media_summary(&image),
            image: Some(image),
        };
    }
    if looks_like_html(raw) {
        return Presented {
            kind: "HTML",
            text: pretty_markup(raw),
            image: None,
        };
    }
    if looks_like_xml(raw) {
        return Presented {
            kind: "XML",
            text: pretty_markup(raw),
            image: None,
        };
    }
    if looks_like_csv(raw) {
        return Presented {
            kind: "CSV",
            text: raw.to_owned(),
            image: None,
        };
    }
    if looks_like_yaml(raw) {
        return Presented {
            kind: "YAML",
            text: raw.to_owned(),
            image: None,
        };
    }
    Presented {
        kind: "Text",
        text: raw.to_owned(),
        image: None,
    }
}

fn image_from(value: &Value) -> Option<ImagePreview> {
    match value {
        Value::Object(object) => image_from_object(object).or_else(|| {
            object
                .values()
                .find_map(|child| image_from(child).filter(|_| child.is_object()))
        }),
        Value::Array(items) => items.iter().find_map(image_from),
        Value::String(text) => image_from_data_url(text),
        _ => None,
    }
}

fn image_from_object(object: &Map<String, Value>) -> Option<ImagePreview> {
    let encoded = string_field(object, &["data", "bytes", "content", "image"])?;
    if let Some(image) = image_from_data_url(encoded) {
        return Some(image);
    }
    let media_type = string_field(object, &["media_type", "content_type", "mime_type"])
        .or_else(|| {
            object
                .get("descriptor")
                .and_then(Value::as_object)
                .and_then(|descriptor| string_field(descriptor, &["media_type", "content_type"]))
        })
        .or_else(|| sniff_media(encoded))?;
    Some(report::preview_for(media_type, encoded))
}

fn image_from_data_url(raw: &str) -> Option<ImagePreview> {
    let rest = raw.strip_prefix("data:")?;
    let (media_type, payload) = rest.split_once(";base64,")?;
    let media_type = media_type.trim();
    if !media_type.to_ascii_lowercase().starts_with("image/") {
        return None;
    }
    Some(report::preview_for(media_type, payload.trim()))
}

fn collapse(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut next = Map::new();
            for (key, child) in object {
                next.insert(key.clone(), collapse_field(key, child));
            }
            Value::Object(next)
        }
        Value::Array(items) => Value::Array(items.iter().map(collapse).collect()),
        other => other.clone(),
    }
}

fn collapse_field(key: &str, value: &Value) -> Value {
    if let Some(text) = value.as_str()
        && is_payload_key(key)
        && is_base64(text)
        && text.len() > 64
    {
        return payload_note(text);
    }
    collapse(value)
}

fn payload_note(encoded: &str) -> Value {
    let mut note = Map::new();
    note.insert("encoding".to_owned(), Value::String("base64".to_owned()));
    note.insert(
        "characters".to_owned(),
        Value::from(encoded.chars().count() as u64),
    );
    if let Some(bytes) = decoded_len(encoded) {
        note.insert("decoded_bytes".to_owned(), Value::from(bytes as u64));
    }
    Value::Object(note)
}

fn media_summary(image: &ImagePreview) -> String {
    let mut lines = vec![format!("media_type {}", image.media_type)];
    lines.push(format!("encoded_characters {}", image.encoded_chars));
    if let Some(bytes) = image.decoded_bytes {
        lines.push(format!("decoded_bytes {bytes}"));
    }
    if !image.withheld.is_empty() {
        lines.push(format!("preview {}", image.withheld));
    }
    lines.join("\n")
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    })
}

fn sniff_media(encoded: &str) -> Option<&'static str> {
    let compact: String = encoded.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.starts_with("iVBORw0KGgo") {
        return Some("image/png");
    }
    if compact.starts_with("/9j/") {
        return Some("image/jpeg");
    }
    if compact.starts_with("R0lGOD") {
        return Some("image/gif");
    }
    if compact.starts_with("UklGR") {
        return Some("image/webp");
    }
    None
}

fn looks_like_html(raw: &str) -> bool {
    let lower = raw.trim_start().to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || (lower.starts_with('<')
            && ["<head", "<body", "<div", "<span", "<p", "<table"]
                .iter()
                .any(|tag| lower.contains(tag)))
}

fn looks_like_xml(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    (trimmed.starts_with("<?xml") || (trimmed.starts_with('<') && trimmed.contains('>')))
        && !looks_like_html(raw)
}

fn looks_like_csv(raw: &str) -> bool {
    let lines: Vec<_> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    lines.len() >= 2
        && lines
            .iter()
            .all(|line| line.contains(',') && !line.contains('<'))
}

fn looks_like_yaml(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    if trimmed.starts_with("---") {
        return true;
    }
    let lines: Vec<_> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    !lines.is_empty()
        && lines.iter().all(|line| {
            let line = line.trim();
            line.starts_with('-') || line.contains(": ")
        })
}

fn pretty_markup(raw: &str) -> String {
    let mut out = String::new();
    let mut indent = 0usize;
    let mut rest = raw.trim();
    while !rest.is_empty() {
        if let Some(end) = rest.find('<') {
            let text = rest[..end].trim();
            if !text.is_empty() {
                push_line(&mut out, indent, text);
            }
            rest = &rest[end..];
        }
        let Some(close) = rest.find('>') else {
            push_line(&mut out, indent, rest.trim());
            break;
        };
        let tag = rest[..=close].trim();
        rest = rest[close + 1..].trim_start();
        let name = tag
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        let closing = tag.starts_with("</");
        let empty = tag.ends_with("/>") || is_void(name);
        if closing && indent > 0 {
            indent -= 1;
        }
        push_line(&mut out, indent, tag);
        if !closing && !empty {
            indent += 1;
        }
    }
    out
}

fn push_line(out: &mut String, indent: usize, line: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(line);
}

fn is_void(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_payload_key(key: &str) -> bool {
    matches!(key, "data" | "bytes" | "content" | "image")
}

fn is_base64(value: &str) -> bool {
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    decoded_len(&compact).is_some()
}

fn decoded_len(encoded: &str) -> Option<usize> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return None;
    }
    if !encoded
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'='))
    {
        return None;
    }
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    if padding > 2 {
        return None;
    }
    Some(encoded.len() / 4 * 3 - padding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_object_is_pretty_printed() {
        let shown = present(r#"{"cameraEnabled":true,"liveStreamEnabled":false}"#).unwrap();
        assert_eq!(shown.kind, "JSON");
        assert!(shown.text.contains("\n"));
        assert!(shown.text.contains("\"cameraEnabled\": true"));
        assert!(shown.image.is_none());
        let wrapped = present(r#"{"value":"<html><body>ok</body></html>"}"#).unwrap();
        assert_eq!(wrapped.kind, "HTML");
    }

    #[test]
    fn html_and_xml_are_labeled_and_indented() {
        let html = present("<html><body><p>ready</p></body></html>").unwrap();
        assert_eq!(html.kind, "HTML");
        assert!(html.text.contains("\n  <body>"));
        let xml = present(r#"<?xml version="1.0"?><lab><ok>true</ok></lab>"#).unwrap();
        assert_eq!(xml.kind, "XML");
        assert!(xml.text.contains("<ok>"));
        assert!(xml.text.contains("true"));
        assert!(xml.text.contains("</lab>"));
    }

    #[test]
    fn csv_and_yaml_keep_their_text() {
        let csv = present("id,name\n1,heat").unwrap();
        assert_eq!(csv.kind, "CSV");
        let yaml = present("---\nid: heat\nstate: completed").unwrap();
        assert_eq!(yaml.kind, "YAML");
    }

    #[test]
    fn image_json_and_data_url_create_a_preview() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let shown = present(&format!(r#"{{"media_type":"image/png","data":"{png}"}}"#)).unwrap();
        assert_eq!(shown.kind, "Image");
        assert!(shown.image.as_ref().unwrap().preview_url.is_some());
        assert!(!shown.text.contains(png));
        assert!(shown.text.contains("\"encoding\": \"base64\""));

        let url = present(&format!("data:image/png;base64,{png}")).unwrap();
        assert_eq!(url.kind, "Image");
        assert!(url.image.as_ref().unwrap().preview_url.is_some());
    }

    #[test]
    fn sniffed_jpeg_and_nested_descriptor_are_images() {
        let jpeg = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wAAA==";
        let sniffed = present(&format!(r#"{{"data":"{jpeg}"}}"#)).unwrap();
        assert_eq!(sniffed.kind, "Image");
        assert_eq!(sniffed.image.as_ref().unwrap().media_type, "image/jpeg");

        let nested = present(&format!(
            r#"{{"value":{{"descriptor":{{"media_type":"image/png"}},"data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="}}}}"#
        ))
        .unwrap();
        assert_eq!(nested.kind, "Image");
    }

    #[test]
    fn empty_result_is_absent() {
        assert!(present("").is_none());
        assert!(present("   ").is_none());
    }
}
