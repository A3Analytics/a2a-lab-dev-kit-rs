//! Versioned lab predicates and subject schemas for Jev frame decisions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::id::SavedFrameId;
use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::FrameRoiTransform;

/// Initial predicate id for pipette tip attachment.
pub const PREDICATE_PIPETTE_TIP_ATTACHED: &str = "lab.pipette_tip_attached";

/// Initial predicate id for foreign object presence.
pub const PREDICATE_FOREIGN_OBJECT_PRESENT: &str = "lab.foreign_object_present";

/// Initial predicate id for plate misalignment.
pub const PREDICATE_PLATE_MISALIGNED: &str = "lab.plate_misaligned";

/// Supported lab predicate identifier and version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct LabPredicate {
    id: String,
    version: u32,
}

impl LabPredicate {
    /// Pipette tip attached predicate v1.
    #[must_use]
    pub fn pipette_tip_attached_v1() -> Self {
        Self {
            id: PREDICATE_PIPETTE_TIP_ATTACHED.to_owned(),
            version: 1,
        }
    }

    /// Foreign object present predicate v1.
    #[must_use]
    pub fn foreign_object_present_v1() -> Self {
        Self {
            id: PREDICATE_FOREIGN_OBJECT_PRESENT.to_owned(),
            version: 1,
        }
    }

    /// Plate misaligned predicate v1.
    #[must_use]
    pub fn plate_misaligned_v1() -> Self {
        Self {
            id: PREDICATE_PLATE_MISALIGNED.to_owned(),
            version: 1,
        }
    }

    /// Validates a predicate declaration.
    pub fn new(id: impl AsRef<str>, version: u32) -> Result<Self, A2aLabError> {
        let id = id.as_ref().trim();
        if id.is_empty() || !id.starts_with("lab.") {
            return Err(A2aLabError::invalid(
                "predicate",
                "must be a non-empty lab.* identifier",
            ));
        }
        if version == 0 {
            return Err(A2aLabError::invalid(
                "predicate_version",
                "must be greater than zero",
            ));
        }
        Ok(Self {
            id: id.to_owned(),
            version,
        })
    }

    /// Predicate identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Predicate schema version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
}

/// Why a predicate could not be resolved to present or absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionUnknownReason {
    /// Target occluded from view.
    Occlusion,
    /// Subject outside the observable region.
    OutOfView,
    /// Required alignment or scene reference missing.
    MissingReference,
    /// Pose/alignment calibration unavailable.
    UnavailableCalibration,
    /// Configured expectation too uncertain to decide.
    UncertainExpectation,
    /// Evidence quality prevents a defensible call.
    IndeterminateEvidence,
}

/// Pipetter channel within a mount.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PipetteChannelSubject {
    pipetter_id: String,
    channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    tip_evidence_roi: Option<FrameRoiTransform>,
}

impl PipetteChannelSubject {
    /// Validates a pipette tip attachment subject.
    pub fn new(
        pipetter_id: impl AsRef<str>,
        channel: impl AsRef<str>,
        tip_evidence_roi: Option<FrameRoiTransform>,
    ) -> Result<Self, A2aLabError> {
        validate_subject_token("pipetter_id", pipetter_id.as_ref())?;
        validate_subject_token("channel", channel.as_ref())?;
        Ok(Self {
            pipetter_id: pipetter_id.as_ref().to_owned(),
            channel: channel.as_ref().to_owned(),
            tip_evidence_roi,
        })
    }

    /// Pipetter or mount id.
    #[must_use]
    pub fn pipetter_id(&self) -> &str {
        &self.pipetter_id
    }

    /// Channel id within the pipetter.
    #[must_use]
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Optional ROI where tip evidence is evaluated.
    #[must_use]
    pub const fn tip_evidence_roi(&self) -> Option<FrameRoiTransform> {
        self.tip_evidence_roi
    }
}

/// Bench/deck region for foreign-object inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignObjectSubject {
    deck_region: String,
    evidence_roi: FrameRoiTransform,
    expected_scene_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    unknown_reason: Option<DecisionUnknownReason>,
}

impl ForeignObjectSubject {
    /// Validates a foreign-object predicate subject.
    pub fn new(
        deck_region: impl AsRef<str>,
        evidence_roi: FrameRoiTransform,
        expected_scene_digest: Option<Sha256Digest>,
        unknown_reason: Option<DecisionUnknownReason>,
    ) -> Result<Self, A2aLabError> {
        validate_subject_token("deck_region", deck_region.as_ref())?;
        Ok(Self {
            deck_region: deck_region.as_ref().to_owned(),
            evidence_roi,
            expected_scene_digest,
            unknown_reason,
        })
    }

    /// Named deck or bench region.
    #[must_use]
    pub fn deck_region(&self) -> &str {
        &self.deck_region
    }

    /// Region inspected for foreign objects.
    #[must_use]
    pub const fn evidence_roi(&self) -> FrameRoiTransform {
        self.evidence_roi
    }

    /// Versioned expected-scene policy digest when configured.
    #[must_use]
    pub const fn expected_scene_digest(&self) -> Option<&Sha256Digest> {
        self.expected_scene_digest.as_ref()
    }

    /// Recorded reason when expectations are not sufficient to decide.
    #[must_use]
    pub const fn unknown_reason(&self) -> Option<DecisionUnknownReason> {
        self.unknown_reason
    }
}

/// Alignment reference for plate misalignment assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlateAlignmentReference {
    /// Expected pose digest from operator configuration.
    ExpectedPoseDigest(Sha256Digest),
    /// Recorded reason when no reference is available.
    Unknown(DecisionUnknownReason),
}

/// Plate deck-slot subject for misalignment predicates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlateSlotSubject {
    plate_id: String,
    deck_slot: String,
    alignment_reference: PlateAlignmentReference,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    inspection_roi: Option<FrameRoiTransform>,
}

impl PlateSlotSubject {
    /// Validates a plate misalignment subject.
    pub fn new(
        plate_id: impl AsRef<str>,
        deck_slot: impl AsRef<str>,
        alignment_reference: PlateAlignmentReference,
        inspection_roi: Option<FrameRoiTransform>,
    ) -> Result<Self, A2aLabError> {
        validate_subject_token("plate_id", plate_id.as_ref())?;
        validate_subject_token("deck_slot", deck_slot.as_ref())?;
        Ok(Self {
            plate_id: plate_id.as_ref().to_owned(),
            deck_slot: deck_slot.as_ref().to_owned(),
            alignment_reference,
            inspection_roi,
        })
    }

    /// Plate identifier.
    #[must_use]
    pub fn plate_id(&self) -> &str {
        &self.plate_id
    }

    /// Deck slot label.
    #[must_use]
    pub fn deck_slot(&self) -> &str {
        &self.deck_slot
    }

    /// Alignment reference or explicit unknown reason.
    #[must_use]
    pub const fn alignment_reference(&self) -> &PlateAlignmentReference {
        &self.alignment_reference
    }

    /// Optional ROI for plate pose inspection.
    #[must_use]
    pub const fn inspection_roi(&self) -> Option<FrameRoiTransform> {
        self.inspection_roi
    }
}

/// Predicate subject required for evaluation and filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionSubject {
    /// Pipetter channel for tip attachment.
    PipetteChannel(PipetteChannelSubject),
    /// Deck region for foreign object evidence.
    ForeignObject(ForeignObjectSubject),
    /// Plate and deck slot for misalignment.
    PlateSlot(PlateSlotSubject),
}

impl DecisionSubject {
    /// Subject is compatible with the predicate declaration.
    #[must_use]
    pub fn matches_predicate(&self, predicate: &LabPredicate) -> bool {
        matches!(
            (predicate.id(), predicate.version(), self),
            (PREDICATE_PIPETTE_TIP_ATTACHED, 1, Self::PipetteChannel(_))
                | (PREDICATE_FOREIGN_OBJECT_PRESENT, 1, Self::ForeignObject(_))
                | (PREDICATE_PLATE_MISALIGNED, 1, Self::PlateSlot(_))
        )
    }
}

/// Modality of image evidence supplied to Jev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEvidenceKind {
    /// Exact saved frame revision bytes.
    SavedFrameRevision,
    /// Operator reference still image digest.
    ReferenceImage,
}

/// One bounded evidence item for a frame decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionEvidenceItem {
    frame_id: SavedFrameId,
    roi: FrameRoiTransform,
    kind: DecisionEvidenceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    reference_digest: Option<Sha256Digest>,
}

impl DecisionEvidenceItem {
    /// Saved-frame evidence for the decision record's revision.
    pub fn saved_frame_revision(
        frame_id: SavedFrameId,
        roi: FrameRoiTransform,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            frame_id,
            roi,
            kind: DecisionEvidenceKind::SavedFrameRevision,
            reference_digest: None,
        })
    }

    /// Reference image digest supplied as criteria context.
    pub fn reference_image(
        frame_id: SavedFrameId,
        roi: FrameRoiTransform,
        reference_digest: Sha256Digest,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            frame_id,
            roi,
            kind: DecisionEvidenceKind::ReferenceImage,
            reference_digest: Some(reference_digest),
        })
    }

    /// Evidence frame revision id.
    #[must_use]
    pub const fn frame_id(&self) -> &SavedFrameId {
        &self.frame_id
    }

    /// Evidence ROI on the oriented saved frame.
    #[must_use]
    pub const fn roi(&self) -> FrameRoiTransform {
        self.roi
    }

    /// Evidence modality.
    #[must_use]
    pub const fn kind(&self) -> DecisionEvidenceKind {
        self.kind
    }

    /// Reference image digest when [`Self::kind`] is reference.
    #[must_use]
    pub const fn reference_digest(&self) -> Option<&Sha256Digest> {
        self.reference_digest.as_ref()
    }
}

fn validate_subject_token(field: &'static str, value: &str) -> Result<(), A2aLabError> {
    if value.trim().is_empty() || value.len() > 64 {
        return Err(A2aLabError::invalid(
            field,
            "must be 1..=64 visible characters",
        ));
    }
    Ok(())
}
