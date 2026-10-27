//! UTC instants and half-open time ranges.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::A2aLabError;

/// An absolute UTC timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp(Timestamp);

impl UtcTimestamp {
    /// Returns the current UTC time.
    #[must_use]
    pub fn now() -> Self {
        Self(Timestamp::now())
    }

    /// Parses an RFC 3339 timestamp whose offset is UTC.
    pub fn parse(value: &str) -> Result<Self, A2aLabError> {
        let timestamp = Timestamp::from_str(value)
            .map_err(|_| A2aLabError::invalid("timestamp", "must be an RFC 3339 UTC timestamp"))?;
        let utc = value.ends_with('Z') || value.ends_with("+00:00") || value.ends_with("-00:00");
        if !utc {
            return Err(A2aLabError::invalid("timestamp", "must use a UTC offset"));
        }
        Ok(Self(timestamp))
    }

    /// Returns the timestamp in RFC 3339 form.
    #[must_use]
    pub fn to_rfc3339(&self) -> String {
        self.0.to_string()
    }

    /// Microseconds since the Unix epoch.
    #[must_use]
    pub fn as_micros(self) -> i64 {
        self.0.as_microsecond()
    }

    /// Adds `delta_us` microseconds, saturating at the representable range.
    #[must_use]
    pub fn saturating_add_micros(self, delta_us: i64) -> Self {
        let micros = self.as_micros().saturating_add(delta_us);
        Timestamp::from_microsecond(micros).map_or(self, Self)
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for UtcTimestamp {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "UtcTimestamp".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "format": "date-time"
        })
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_rfc3339())
    }
}

/// A half-open UTC interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimeRange {
    start: UtcTimestamp,
    end: UtcTimestamp,
}

impl TimeRange {
    /// Creates a range when `start` is strictly before `end`.
    pub fn new(start: UtcTimestamp, end: UtcTimestamp) -> Result<Self, A2aLabError> {
        let range = Self { start, end };
        range.check()?;
        Ok(range)
    }

    /// Rejects an empty or reversed range.
    pub fn check(self) -> Result<(), A2aLabError> {
        if self.start >= self.end {
            return Err(A2aLabError::invalid("range", "start must be before end"));
        }
        Ok(())
    }

    /// Returns the inclusive start instant.
    #[must_use]
    pub const fn start(self) -> UtcTimestamp {
        self.start
    }

    /// Returns the exclusive end instant.
    #[must_use]
    pub const fn end(self) -> UtcTimestamp {
        self.end
    }

    /// Reports whether `timestamp` lies in `[start, end)`.
    #[must_use]
    pub fn contains(self, timestamp: UtcTimestamp) -> bool {
        timestamp >= self.start && timestamp < self.end
    }

    /// Duration of `[start, end)` in microseconds.
    ///
    /// Callers must [`check`](Self::check) first so `end` is after `start`.
    #[must_use]
    pub fn duration_us(self) -> u64 {
        u64::try_from(self.end.as_micros().saturating_sub(self.start.as_micros())).unwrap_or(0)
    }

    /// Builds a range without validating order. Reserved for TCK fixtures.
    pub(crate) const fn tck_unchecked(start: UtcTimestamp, end: UtcTimestamp) -> Self {
        Self { start, end }
    }
}
