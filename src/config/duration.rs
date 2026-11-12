//! Elapsed-time strings for configuration fields.
//!
//! [`ConfigDuration`] keeps retention fields at whole seconds. [`SaveIntervalDuration`]
//! accepts positive sub-second intervals for DVR persistence cadence.
//!
//! Parsing goes through Jiff. Days are fixed at 24 hours and weeks at seven
//! such days. Whole-second retention still fits the existing runtime conversion
//! into microseconds.

use jiff::{Span, SpanRelativeTo};

use crate::error::A2aLabError;

/// Largest whole-second retention whose microsecond total fits in `i64`.
const MAX_WHOLE_SECONDS: u64 = 9_223_372_036_854;

/// Positive elapsed time stored as whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigDuration {
    seconds: u64,
}

impl ConfigDuration {
    /// Parses one retention string without echoing `text` in an error.
    pub fn parse(field: &'static str, text: &str) -> Result<Self, A2aLabError> {
        if text.chars().all(char::is_whitespace) {
            return Err(invalid(field));
        }
        let span: Span = text.parse().map_err(|_| invalid(field))?;
        if span.get_years() != 0 || span.get_months() != 0 {
            return Err(invalid(field));
        }
        let duration = span
            .to_duration(SpanRelativeTo::days_are_24_hours())
            .map_err(|_| invalid(field))?;
        if duration.subsec_nanos() != 0 {
            return Err(invalid(field));
        }
        let seconds = duration.as_secs();
        if seconds <= 0 {
            return Err(invalid(field));
        }
        let seconds = u64::try_from(seconds).map_err(|_| invalid(field))?;
        if seconds > MAX_WHOLE_SECONDS {
            return Err(invalid(field));
        }
        Ok(Self { seconds })
    }

    /// Whole seconds after fixed-day conversion.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.seconds
    }
}

fn invalid(field: &'static str) -> A2aLabError {
    A2aLabError::invalid(field, "must be a positive whole-second duration")
}

/// Largest save interval that still fits signed nanosecond timestamps.
const MAX_SAVE_INTERVAL_NANOS: u64 = i64::MAX as u64;

/// Positive DVR save cadence stored as whole nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveIntervalDuration {
    nanos: u64,
}

impl SaveIntervalDuration {
    /// Parses one interval string without echoing `text` in an error.
    pub fn parse(field: &'static str, text: &str) -> Result<Self, A2aLabError> {
        if text.chars().all(char::is_whitespace) {
            return Err(invalid_save_interval(field));
        }
        let span: Span = text.parse().map_err(|_| invalid_save_interval(field))?;
        if span.get_years() != 0 || span.get_months() != 0 {
            return Err(invalid_save_interval(field));
        }
        let duration = span
            .to_duration(SpanRelativeTo::days_are_24_hours())
            .map_err(|_| invalid_save_interval(field))?;
        let nanos = u64::try_from(duration.as_nanos()).map_err(|_| invalid_save_interval(field))?;
        if nanos == 0 {
            return Err(invalid_save_interval(field));
        }
        if nanos > MAX_SAVE_INTERVAL_NANOS {
            return Err(invalid_save_interval(field));
        }
        Ok(Self { nanos })
    }

    /// Stores a validated positive interval.
    pub fn from_nanos(nanos: u64) -> Result<Self, A2aLabError> {
        if nanos == 0 || nanos > MAX_SAVE_INTERVAL_NANOS {
            return Err(invalid_save_interval("interval"));
        }
        Ok(Self { nanos })
    }

    /// Whole nanoseconds after fixed-day conversion.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.nanos
    }

    /// Compact text for settings views and round-trips.
    #[must_use]
    pub fn as_text(self) -> String {
        if self.nanos.is_multiple_of(1_000_000_000) {
            format!("{}s", self.nanos / 1_000_000_000)
        } else if self.nanos.is_multiple_of(1_000_000) {
            format!("{}ms", self.nanos / 1_000_000)
        } else if self.nanos.is_multiple_of(1_000) {
            format!("{}us", self.nanos / 1_000)
        } else {
            format!("{}ns", self.nanos)
        }
    }
}

fn invalid_save_interval(field: &'static str) -> A2aLabError {
    A2aLabError::invalid(field, "must be a positive duration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_interval_accepts_subsecond_examples() {
        for (text, nanos) in [
            ("500ms", 500_000_000),
            ("1s", 1_000_000_000),
            ("5s", 5_000_000_000),
            ("30s", 30_000_000_000),
            ("1.5s", 1_500_000_000),
        ] {
            let parsed = SaveIntervalDuration::parse("videos.dvr.save_interval", text)
                .unwrap_or_else(|_| panic!("{text}"));
            assert_eq!(parsed.as_nanos(), nanos, "{text}");
        }
    }

    #[test]
    fn save_interval_rejects_invalid_text() {
        for text in ["", "0", "-1s", "0ms", "nope", "1y"] {
            assert!(
                SaveIntervalDuration::parse("videos.dvr.save_interval", text).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn save_interval_text_is_compact() {
        assert_eq!(
            SaveIntervalDuration::parse("videos.dvr.save_interval", "500ms")
                .unwrap()
                .as_text(),
            "500ms"
        );
        assert_eq!(
            SaveIntervalDuration::parse("videos.dvr.save_interval", "1s")
                .unwrap()
                .as_text(),
            "1s"
        );
    }
}
