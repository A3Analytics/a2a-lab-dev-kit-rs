use a2a_lab_dev_kit::{
    A2aLabError, JsonObject, MAX_PAGE_LIMIT, MetricPoint, PageRequest, SourceId, TimeRange,
    UtcTimestamp, version,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

#[test]
fn reports_package_version() {
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn rejects_malformed_identifiers() {
    assert!(SourceId::new("").is_err());
    assert!(SourceId::new("has space").is_err());
    assert!(SourceId::new("a/b").is_err());
    assert!(SourceId::new("x".repeat(129)).is_err());
    assert_eq!(SourceId::new("app.log_1").unwrap().as_str(), "app.log_1");
}

#[test]
fn accepts_only_utc_timestamps() {
    assert!(UtcTimestamp::parse("2024-01-01T00:00:00+01:00").is_err());
    assert!(UtcTimestamp::parse("2024-01-01T00:00:00").is_err());
    let parsed = timestamp("2024-01-01T00:00:00Z");
    assert_eq!(timestamp("2024-01-01T00:00:00+00:00"), parsed);
    assert_eq!(parsed.to_rfc3339(), "2024-01-01T00:00:00Z");
}

#[test]
fn rejects_empty_or_reversed_ranges_and_uses_half_open_bounds() {
    let start = timestamp("2024-01-01T00:00:00Z");
    let end = timestamp("2024-01-01T01:00:00Z");
    assert!(TimeRange::new(start, start).is_err());
    assert!(TimeRange::new(end, start).is_err());
    let range = TimeRange::new(start, end).unwrap();
    assert!(range.contains(start));
    assert!(!range.contains(end));
}

#[test]
fn rejects_invalid_pages_and_omits_the_last_cursor() {
    assert!(PageRequest::new(None, 0).is_err());
    assert!(PageRequest::new(None, MAX_PAGE_LIMIT + 1).is_err());
    assert!(PageRequest::new(Some("next".to_owned()), 10).is_err());
    let request = PageRequest::new(Some("2".to_owned()), 2).unwrap();
    assert_eq!(request.cursor(), Some("2"));
    assert_eq!(request.limit(), 2);
}

#[test]
fn rejects_non_object_task_input() {
    assert!(JsonObject::parse("[1]").is_err());
    assert!(JsonObject::parse("\"text\"").is_err());
    assert!(JsonObject::parse("{").is_err());
    let object = JsonObject::parse(r#"{"branch":"main"}"#).unwrap();
    assert_eq!(
        object
            .as_map()
            .get("branch")
            .and_then(serde_json::Value::as_str),
        Some("main")
    );
}

#[test]
fn rejects_non_finite_metric_values() {
    let instant = timestamp("2024-01-01T00:00:00Z");
    assert!(MetricPoint::new(instant, f64::NAN).is_err());
    let error =
        serde_json::from_str::<MetricPoint>(r#"{"timestamp":"2024-01-01T00:00:00Z","value":null}"#);
    assert!(error.is_err());
    let point = MetricPoint::new(instant, 1.5).unwrap();
    let decoded: MetricPoint =
        serde_json::from_str(&serde_json::to_string(&point).unwrap()).unwrap();
    assert_eq!(decoded, point);
}

#[test]
fn error_codes_round_trip() {
    let error = A2aLabError::not_found("log source", "app");
    assert_eq!(error.code(), "not_found");
    assert_eq!(
        A2aLabError::from_code(error.code(), error.to_string()).code(),
        "not_found"
    );
}
