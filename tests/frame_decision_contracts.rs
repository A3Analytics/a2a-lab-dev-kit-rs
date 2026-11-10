//! Contract tests for Jev-only frame decision records and ledger boundary.

use a2a_lab_dev_kit::{
    DecisionEvaluationCoverage, DecisionEvidenceItem, DecisionIdempotencyKey, DecisionOutcome,
    DecisionSubject, DecisionUnknownReason, DvrStoreNamespace, DvrTrackId, ForeignObjectSubject,
    FrameDecisionAppendError, FrameDecisionLedger, FrameDecisionRecord, FrameRoiTransform,
    JevEvaluationId, JevEvaluationProvenance, JevProducerIdentity, JevStructuredOutput,
    LabPredicate, LabProbabilityVector, PREDICATE_PIPETTE_TIP_ATTACHED, PipetteChannelSubject,
    PlateAlignmentReference, PlateSlotSubject, RejectedDecisionProvenance, SavedFrameRef,
    Sha256Digest, UtcTimestamp, VideoSourceId, derive_decision_id, reject_decision_provenance,
};
use a2a_lab_dev_kit::{PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PLATE_MISALIGNED};

fn frame(payload: &[u8]) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        VideoSourceId::new("deck-left").unwrap(),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        payload,
        None,
    )
    .unwrap()
}

fn evaluation_id() -> JevEvaluationId {
    JevEvaluationId::new("jev-req-chupacabra-001").unwrap()
}

fn idempotency_key() -> DecisionIdempotencyKey {
    DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1").unwrap()
}

fn provenance(captured: &str, evaluated: &str) -> JevEvaluationProvenance {
    JevEvaluationProvenance::new(
        UtcTimestamp::parse(captured).unwrap(),
        UtcTimestamp::parse(evaluated).unwrap(),
        JevProducerIdentity::default_production(),
        evaluation_id(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response-bytes"),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .unwrap()
}

fn tip_subject() -> DecisionSubject {
    DecisionSubject::PipetteChannel(
        PipetteChannelSubject::new(
            "left-pipetter",
            "channel-1",
            Some(FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap()),
        )
        .unwrap(),
    )
}

fn tip_evidence(saved: &SavedFrameRef) -> Vec<DecisionEvidenceItem> {
    vec![
        DecisionEvidenceItem::saved_frame_revision(
            saved.frame_id().clone(),
            FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap(),
        )
        .unwrap(),
    ]
}

fn tip_record(
    saved: &SavedFrameRef,
    outcome: DecisionOutcome,
    key: DecisionIdempotencyKey,
    supersedes: Option<a2a_lab_dev_kit::FrameDecisionId>,
) -> FrameDecisionRecord {
    let output = JevStructuredOutput::new(
        Some(0.93),
        None,
        None,
        None,
        None,
        Some(FrameRoiTransform::new([0.22, 0.31, 0.48, 0.69]).unwrap()),
        None,
    )
    .unwrap();
    FrameDecisionRecord::new(
        key,
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(saved),
        outcome,
        output,
        provenance("2024-06-01T12:00:00Z", "2024-06-01T12:00:02Z"),
        supersedes,
    )
    .unwrap()
}

#[test]
fn decision_id_and_idempotency_are_stable() {
    let saved = frame(b"chupacabra-pilot");
    let record = tip_record(&saved, DecisionOutcome::Present, idempotency_key(), None);
    let derived = derive_decision_id(saved.frame_id(), &idempotency_key()).unwrap();
    assert_eq!(record.decision_id(), &derived);
    assert_eq!(
        record.idempotency_key().as_str(),
        idempotency_key().as_str()
    );
    assert_eq!(record.frame().frame_id(), saved.frame_id());
}

#[test]
fn failed_coverage_cannot_author_absent_outcome() {
    let err = JevEvaluationProvenance::new(
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:01Z").unwrap(),
        JevProducerIdentity::default_production(),
        evaluation_id(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response-bytes"),
        DecisionEvaluationCoverage::Failed,
        Option::<&str>::None,
    )
    .expect_err("failed coverage");
    assert!(err.to_string().contains("coverage"));
}

#[test]
fn vendor_confidence_is_not_treated_as_class_probability_field() {
    let saved = frame(b"confidence");
    let output = JevStructuredOutput::new(
        Some(0.12),
        Some(
            LabProbabilityVector::new(
                vec!["present".to_owned(), "absent".to_owned()],
                vec![0.12, 0.88],
            )
            .unwrap(),
        ),
        None,
        Some(0.4),
        Some(0.88),
        None,
        None,
    )
    .unwrap();
    let record = FrameDecisionRecord::new(
        idempotency_key(),
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(&frame(b"confidence")),
        DecisionOutcome::Unknown,
        output,
        provenance("2024-06-01T12:00:00Z", "2024-06-01T12:00:02Z"),
        None,
    )
    .unwrap();
    assert_ne!(
        record.structured_output().noul_probability(),
        record.structured_output().vendor_confidence()
    );
    let distribution = record.structured_output().option_distribution().unwrap();
    let noul = record.structured_output().noul_probability().unwrap();
    assert!((distribution.values()[0] - noul).abs() < f32::EPSILON);
}

#[test]
fn initial_predicate_schemas_cover_tip_foreign_object_and_plate() {
    let foreign = DecisionSubject::ForeignObject(
        ForeignObjectSubject::new(
            "deck-slot-3",
            FrameRoiTransform::new([0.1, 0.1, 0.9, 0.9]).unwrap(),
            Some(Sha256Digest::hash_bytes(b"expected-scene-v1")),
            Some(DecisionUnknownReason::UncertainExpectation),
        )
        .unwrap(),
    );
    assert!(foreign.matches_predicate(&LabPredicate::foreign_object_present_v1()));

    let plate = DecisionSubject::PlateSlot(
        PlateSlotSubject::new(
            "plate-a",
            "slot-3",
            PlateAlignmentReference::Unknown(DecisionUnknownReason::UnavailableCalibration),
            None,
        )
        .unwrap(),
    );
    assert!(plate.matches_predicate(&LabPredicate::plate_misaligned_v1()));
    assert!(tip_subject().matches_predicate(&LabPredicate::pipette_tip_attached_v1()));
    assert!(!tip_subject().matches_predicate(&LabPredicate::foreign_object_present_v1()));
    assert_eq!(PREDICATE_PIPETTE_TIP_ATTACHED, "lab.pipette_tip_attached");
    assert_eq!(
        PREDICATE_FOREIGN_OBJECT_PRESENT,
        "lab.foreign_object_present"
    );
    assert_eq!(PREDICATE_PLATE_MISALIGNED, "lab.plate_misaligned");
}

#[test]
fn ledger_preserves_supersession_history() {
    let saved = frame(b"v1");
    let mut ledger = FrameDecisionLedger::new();
    let first = ledger
        .append(tip_record(
            &saved,
            DecisionOutcome::Unknown,
            DecisionIdempotencyKey::new(
                "jev-req-chupacabra-001:lab.pipette_tip_attached:left-1:v1",
            )
            .unwrap(),
            None,
        ))
        .unwrap()
        .clone();
    let second = ledger
        .append(tip_record(
            &saved,
            DecisionOutcome::Present,
            DecisionIdempotencyKey::new(
                "jev-req-chupacabra-001:lab.pipette_tip_attached:left-1:v2",
            )
            .unwrap(),
            Some(first.decision_id().clone()),
        ))
        .unwrap()
        .clone();
    let history = ledger.history_for_predicate(&saved, &LabPredicate::pipette_tip_attached_v1());
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].decision_id(), first.decision_id());
    assert_eq!(history[1].supersedes(), Some(first.decision_id()));
    assert_ne!(first.outcome(), second.outcome());
}

#[test]
fn idempotent_replay_returns_original_record() {
    let saved = frame(b"idem");
    let mut ledger = FrameDecisionLedger::new();
    let first_id = ledger
        .append(tip_record(
            &saved,
            DecisionOutcome::Absent,
            idempotency_key(),
            None,
        ))
        .unwrap()
        .decision_id()
        .clone();
    let second_id = ledger
        .append(tip_record(
            &saved,
            DecisionOutcome::Absent,
            idempotency_key(),
            None,
        ))
        .unwrap()
        .decision_id()
        .clone();
    assert_eq!(first_id, second_id);
    assert_eq!(ledger.history().len(), 1);
}

#[test]
fn rejects_invalid_roi_scores_and_predicate_versions() {
    let roi_err = FrameRoiTransform::new([1.2, 0.0, 0.5, 0.6]).expect_err("roi");
    assert!(roi_err.to_string().contains("roi"));

    let predicate_err = LabPredicate::new("lab.bad", 0).expect_err("version");
    assert!(predicate_err.to_string().contains("predicate_version"));

    let prob_err = LabProbabilityVector::new(vec!["a".to_owned()], vec![1.5]).expect_err("prob");
    assert!(prob_err.to_string().contains("probabilities"));
}

#[test]
fn rejects_spoofed_and_llm_description_derived_provenance() {
    let label_err = reject_decision_provenance(RejectedDecisionProvenance::ProducerLabelOnly)
        .expect_err("label");
    assert!(label_err.to_string().contains("producer label"));

    let llm_err =
        reject_decision_provenance(RejectedDecisionProvenance::LlmAssessment).expect_err("llm");
    assert!(llm_err.to_string().contains("LLM"));

    let saved = frame(b"spoof");
    let mut evidence = tip_evidence(&saved);
    evidence[0] = DecisionEvidenceItem::reference_image(
        saved.frame_id().clone(),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"description-derived"),
    )
    .unwrap();
    let err = FrameDecisionRecord::new(
        idempotency_key(),
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        evidence,
        DecisionOutcome::Present,
        JevStructuredOutput::empty(),
        provenance("2024-06-01T12:00:00Z", "2024-06-01T12:00:02Z"),
        None,
    )
    .expect_err("description-derived");
    assert!(err.to_string().contains("description"));
}

#[test]
fn non_jev_producer_cannot_build_provenance() {
    let spoofed = JevProducerIdentity::new("jev-1.14.0").unwrap();
    assert_eq!(spoofed.producer_id(), "typesafe-jev");
    let spoofed: JevProducerIdentity = serde_json::from_str(
        r#"{"producer_id":"bedrock","model_revision":"global.openai.gpt-5.6-luna"}"#,
    )
    .unwrap();
    let err = JevEvaluationProvenance::new(
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:01Z").unwrap(),
        spoofed,
        evaluation_id(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response-bytes"),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .expect_err("luna");
    assert!(err.to_string().contains("typesafe-jev"));
}

#[test]
fn idempotency_conflict_preserves_original_payload() {
    let saved = frame(b"conflict");
    let mut ledger = FrameDecisionLedger::new();
    ledger
        .append(tip_record(
            &saved,
            DecisionOutcome::Present,
            idempotency_key(),
            None,
        ))
        .unwrap();
    let conflict_key =
        DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1")
            .unwrap();
    let different = FrameDecisionRecord::new(
        conflict_key,
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(&frame(b"conflict")),
        DecisionOutcome::Absent,
        JevStructuredOutput::empty(),
        provenance("2024-06-01T12:00:00Z", "2024-06-01T12:00:02Z"),
        None,
    )
    .unwrap();
    let err = ledger.append(different).expect_err("conflict");
    assert!(matches!(
        err,
        FrameDecisionAppendError::IdempotencyConflict { .. }
    ));
}

fn deserialize_forged_record(
    saved: &SavedFrameRef,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> FrameDecisionRecord {
    let record = tip_record(saved, DecisionOutcome::Present, idempotency_key(), None);
    let mut value = serde_json::to_value(record).unwrap();
    mutate(&mut value);
    serde_json::from_value(value).unwrap()
}

#[test]
fn ledger_rejects_deserialized_non_jev_producer() {
    let saved = frame(b"forge-producer");
    let forged = deserialize_forged_record(&saved, |value| {
        value["provenance"]["producer"]["producer_id"] =
            serde_json::Value::String("bedrock".to_owned());
        value["provenance"]["producer"]["model_revision"] =
            serde_json::Value::String("global.openai.gpt-5.6-luna".to_owned());
    });
    let validate_err = forged.validate().expect_err("non-jev");
    assert!(validate_err.to_string().contains("typesafe-jev"));

    let mut ledger = FrameDecisionLedger::new();
    let append_err = ledger.append(forged).expect_err("append");
    assert!(matches!(
        append_err,
        FrameDecisionAppendError::Validation(_)
    ));
    assert_eq!(append_err.code(), "validation");
    assert!(ledger.history().is_empty());
}

#[test]
fn ledger_rejects_deserialized_failed_coverage() {
    let saved = frame(b"forge-coverage");
    let forged = deserialize_forged_record(&saved, |value| {
        value["provenance"]["coverage"] = serde_json::Value::String("failed".to_owned());
    });
    let validate_err = forged.validate().expect_err("failed coverage");
    assert!(validate_err.to_string().contains("coverage"));

    let mut ledger = FrameDecisionLedger::new();
    let append_err = ledger.append(forged).expect_err("append");
    assert!(matches!(
        append_err,
        FrameDecisionAppendError::Validation(_)
    ));
    assert!(ledger.history().is_empty());
}

#[test]
fn ledger_rejects_deserialized_forged_decision_id() {
    let saved = frame(b"forge-id");
    let forged = deserialize_forged_record(&saved, |value| {
        value["decision_id"] = serde_json::Value::String(
            "fd:forged0000000000000000000000000000000000000000000000000000000001".to_owned(),
        );
    });
    let validate_err = forged.validate().expect_err("decision id");
    assert!(validate_err.to_string().contains("decision_id"));

    let mut ledger = FrameDecisionLedger::new();
    let append_err = ledger.append(forged).expect_err("append");
    assert!(matches!(
        append_err,
        FrameDecisionAppendError::Validation(_)
    ));
    assert!(ledger.history().is_empty());
}
