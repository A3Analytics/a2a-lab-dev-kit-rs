//! Elapsed-time strings for configuration retention fields.
//!
//! Parsing goes through Jiff. Days are fixed at 24 hours and weeks at seven
//! such days. The result is a positive whole number of seconds that still fits
//! the existing runtime conversion into microseconds.

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
