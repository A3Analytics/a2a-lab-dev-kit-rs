//! History reduction for bounded decision reads.

use std::collections::BTreeMap;

use crate::id::FrameDecisionId;
use crate::videos::frame_decision::predicate::LabPredicate;
use crate::videos::frame_decision::record::{FrameDecisionRecord, JevProducerIdentity};
use crate::videos::frame_decision::scope::FrameDecisionReadScope;
use crate::videos::semantic_search::DecisionHistoryPolicy;
use crate::{DecisionOutcome, DecisionSubject};

/// Filters ledger rows to a scope and optional predicate/subject/producer.
#[must_use]
pub fn filter_records<'a>(
    records: impl Iterator<Item = &'a FrameDecisionRecord>,
    scope: &FrameDecisionReadScope,
    predicate: Option<&LabPredicate>,
    subject: Option<&DecisionSubject>,
    producer: Option<&JevProducerIdentity>,
    outcome: Option<DecisionOutcome>,
) -> Vec<&'a FrameDecisionRecord> {
    records
        .filter(|record| scope.contains(record))
        .filter(|record| predicate.is_none_or(|expected| record.predicate() == expected))
        .filter(|record| subject.is_none_or(|expected| record.subject() == expected))
        .filter(|record| producer.is_none_or(|expected| record.provenance().producer() == expected))
        .filter(|record| outcome.is_none_or(|expected| record.outcome() == expected))
        .collect()
}

/// Applies an explicit history policy before paging.
#[must_use]
pub fn reduce_history<'a>(
    mut records: Vec<&'a FrameDecisionRecord>,
    policy: DecisionHistoryPolicy,
    producer: Option<&'a JevProducerIdentity>,
) -> Vec<&'a FrameDecisionRecord> {
    match policy {
        DecisionHistoryPolicy::FullHistory => records,
        DecisionHistoryPolicy::LatestPerPredicateSubject => {
            latest_grouped(records, |record| group_key(record, None))
        }
        DecisionHistoryPolicy::LatestAuthorizedProduction => {
            if let Some(expected) = producer {
                records.retain(|record| record.provenance().producer() == expected);
            }
            latest_grouped(records, |record| group_key(record, producer))
        }
    }
}

fn latest_grouped<'a>(
    records: Vec<&'a FrameDecisionRecord>,
    key: impl Fn(&'a FrameDecisionRecord) -> String,
) -> Vec<&'a FrameDecisionRecord> {
    let mut chosen: BTreeMap<String, (&FrameDecisionRecord, FrameDecisionId)> = BTreeMap::new();
    for record in records {
        let bucket = key(record);
        let replace = match chosen.get(&bucket) {
            None => true,
            Some((prior, _)) => {
                record.provenance().evaluated_at() >= prior.provenance().evaluated_at()
            }
        };
        if replace {
            chosen.insert(bucket, (record, record.decision_id().clone()));
        }
    }
    let mut ordered: Vec<_> = chosen.into_values().map(|(record, _)| record).collect();
    ordered.sort_by_key(|record| record.provenance().evaluated_at());
    ordered
}

fn group_key(record: &FrameDecisionRecord, producer: Option<&JevProducerIdentity>) -> String {
    let producer = if let Some(identity) = producer.or(Some(record.provenance().producer())) {
        format!("{}@{}", identity.producer_id(), identity.model_revision())
    } else {
        "any".to_owned()
    };
    format!(
        "{}|v{}|{producer}|{}|{}|{}",
        record.predicate().id(),
        record.predicate().version(),
        subject_token(record.subject()),
        record.frame().frame_id().as_str(),
        record.provenance().evaluation_id().as_str(),
    )
}

fn subject_token(subject: &DecisionSubject) -> String {
    serde_json::to_string(subject).unwrap_or_else(|_| "subject".to_owned())
}
