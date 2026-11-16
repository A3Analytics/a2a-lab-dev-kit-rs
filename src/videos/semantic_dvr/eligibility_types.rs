//! Durable stage-eligibility slot records.

/// One canonical eligibility slot for a source, track, stage, and interval bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageEligibilityRecord {
    pub(crate) source_id: String,
    pub(crate) track_id: String,
    pub(crate) stage: String,
    pub(crate) bucket_start_us: i64,
    pub(crate) interval_us: Option<i64>,
    pub(crate) winner_frame_id: String,
    pub(crate) winner_start_us: i64,
}

impl StageEligibilityRecord {
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    #[must_use]
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    #[must_use]
    pub fn stage(&self) -> &str {
        &self.stage
    }

    #[must_use]
    pub const fn bucket_start_us(&self) -> i64 {
        self.bucket_start_us
    }

    #[must_use]
    pub const fn interval_us(&self) -> Option<i64> {
        self.interval_us
    }

    #[must_use]
    pub fn winner_frame_id(&self) -> &str {
        &self.winner_frame_id
    }

    #[must_use]
    pub const fn winner_start_us(&self) -> i64 {
        self.winner_start_us
    }
}

/// Epoch-aligned bucket start for a capture timestamp.
#[must_use]
pub fn interval_bucket_start(start_us: i64, interval_us: i64) -> i64 {
    start_us.div_euclid(interval_us) * interval_us
}

/// True when `(start_us, frame_id)` should replace the current winner.
#[must_use]
pub fn eligibility_winner_is_better(
    new_start_us: i64,
    new_frame_id: &str,
    current_start_us: i64,
    current_frame_id: &str,
) -> bool {
    (new_start_us, new_frame_id) < (current_start_us, current_frame_id)
}

/// Durable job id for one eligibility slot. `|` is outside public id charset.
#[must_use]
pub fn stage_eligibility_job_id(
    source_id: &str,
    track_id: &str,
    stage: &str,
    bucket_start_us: i64,
) -> String {
    format!("{source_id}|{track_id}|{stage}|{bucket_start_us}")
}

#[cfg(test)]
mod tests {
    use super::{eligibility_winner_is_better, interval_bucket_start};

    #[test]
    fn buckets_are_epoch_aligned() {
        assert_eq!(interval_bucket_start(0, 5_000_000), 0);
        assert_eq!(interval_bucket_start(4_999_999, 5_000_000), 0);
        assert_eq!(interval_bucket_start(5_000_000, 5_000_000), 5_000_000);
        assert_eq!(interval_bucket_start(-1, 5_000_000), -5_000_000);
    }

    #[test]
    fn winner_prefers_earlier_timestamp_then_frame_id() {
        assert!(eligibility_winner_is_better(1, "b", 2, "a"));
        assert!(!eligibility_winner_is_better(2, "a", 1, "b"));
        assert!(eligibility_winner_is_better(5, "aaa", 5, "bbb"));
        assert!(!eligibility_winner_is_better(5, "bbb", 5, "aaa"));
    }
}
