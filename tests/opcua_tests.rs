#![cfg(feature = "opcua")]

use a2a_lab_sdk::opcua::{filter_half_open, namespace_index};
use a2a_lab_sdk::{MetricPoint, TimeRange, UtcTimestamp};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

#[test]
fn resolves_namespace_uris_and_drops_the_exclusive_end() {
    let index = namespace_index(
        &[
            "http://opcfoundation.org/UA/".to_owned(),
            "urn:lab:equipment".to_owned(),
        ],
        "urn:lab:equipment",
    )
    .unwrap();
    assert_eq!(index, 1);
    assert_eq!(
        namespace_index(&["http://opcfoundation.org/UA/".to_owned()], "urn:missing")
            .unwrap_err()
            .code(),
        "not_found"
    );
    let range = TimeRange::new(
        timestamp("2024-01-01T00:00:00Z"),
        timestamp("2024-01-01T01:00:00Z"),
    )
    .unwrap();
    let points = filter_half_open(
        vec![
            MetricPoint::new(timestamp("2024-01-01T00:00:00Z"), 1.0).unwrap(),
            MetricPoint::new(timestamp("2024-01-01T01:00:00Z"), 2.0).unwrap(),
        ],
        range,
    );
    assert_eq!(points.len(), 1);
    assert!((points[0].value - 1.0).abs() < f64::EPSILON);
}
