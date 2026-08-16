//! Episode report — the authoritative, evidence-backed record of one embodied
//! robot task. The natural-language answer returned to the user is only a
//! projection of this structure; it is never a new source of truth.
//!
//! The mutable report builder is deliberately separated from
//! [`SettledEpisodeReport`]. Persistence and promotion consume only the latter,
//! whose digest binds the complete report at the settlement boundary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::embodiment::{DeviceId, EvidenceRef, SkillRequest};
use crate::types::expected_outcome::ExpectedOutcome;
use crate::types::frame::FrameRef;
use crate::types::outcome_verification::{VerificationDecision, VerificationReport};
use crate::types::robot_failure::{RobotFailure, RobotFailureClass};
use crate::types::skill_proposal::PolicyProvenance;
use crate::types::turn::TurnStop;

/// Boundary for distilling a settled, verified robot episode into long-term memory.
#[async_trait::async_trait]
pub trait EpisodePromotionPort: Send + Sync {
    async fn promote(&self, report: &SettledEpisodeReport) -> Result<(), String>;
}

fn is_inline_artifact_uri(uri: &str) -> bool {
    uri.split_once(':')
        .map(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("data") || scheme.eq_ignore_ascii_case("inline")
        })
        .unwrap_or(false)
}

/// The one authoritative settlement classification for a robot episode.
///
/// Both durable episode close and the public turn terminal result are projected
/// from this value. Callers must not independently infer a `TurnStop` from the
/// state-machine terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeSettlement {
    Completed,
    Failed,
    Cancelled,
}

impl EpisodeSettlement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn turn_stop(self) -> TurnStop {
        match self {
            Self::Completed => TurnStop::Completed,
            Self::Failed => TurnStop::Blocked,
            Self::Cancelled => TurnStop::Cancelled,
        }
    }
}

/// Authoritative terminal result returned by the local safe-stop boundary.
///
/// The receipt is recorded only after the executor call returns. It therefore
/// proves an observed terminal result instead of inferring safe-stop from a
/// failed/cancelled episode settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeStopOutcome {
    Succeeded,
    Failed,
}

/// Bounded safe-stop evidence attached to a failed or cancelled episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeStopReceipt {
    /// Number of execution attempts completed before safe-stop was requested.
    /// Zero is valid for a pre-execution failure or cancellation.
    pub attempted_after_attempt: u32,
    /// Last typed failure observed before the safe-stop call. `None` is retained
    /// honestly for legacy/alternate callers that have no trigger fact.
    pub trigger: Option<RobotFailureClass>,
    pub outcome: SafeStopOutcome,
}

/// Inclusive source time range for an externally stored artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactTimeRange {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

/// Artifact retention state exposed by report projections.
///
/// `MetadataIncomplete` is retained rather than fabricating digest/size facts
/// when a legacy provider supplied only a URI. Such an entry is never described
/// as an available, verified artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactAvailability {
    Available,
    RetentionExpired {
        expired_at_unix_ms: i64,
        reason: String,
    },
    MetadataIncomplete {
        missing_fields: Vec<String>,
    },
}

/// Durable metadata retained after an artifact's bytes are deleted by policy.
///
/// The tombstone is intentionally small and content-addressed. It can be stored
/// in the episode database and applied as a read-time projection without
/// mutating the immutable settled report receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRetentionTombstone {
    pub uri: String,
    pub digest: String,
    pub expired_at_unix_ms: i64,
    pub reason: String,
}

impl ArtifactRetentionTombstone {
    pub fn validate(&self) -> Result<(), String> {
        if self.uri.trim().is_empty() || self.uri.len() > 4_096 || is_inline_artifact_uri(&self.uri)
        {
            return Err("artifact tombstone URI is invalid or inline".into());
        }
        let digest = self.digest.strip_prefix("sha256:").unwrap_or(&self.digest);
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("artifact tombstone digest must be lowercase SHA-256 hex".into());
        }
        if self.expired_at_unix_ms < 0 || self.reason.trim().is_empty() {
            return Err("artifact tombstone time/reason is invalid".into());
        }
        Ok(())
    }
}

/// Bounded manifest for a large external artifact.
///
/// The manifest contains metadata and a content-addressed reference only; it
/// can never contain rosbag, log, plot, or frame bytes. Optional metadata exists
/// solely to preserve legacy evidence references honestly. New/available
/// artifacts must populate every field and pass [`Self::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeArtifactManifest {
    pub kind: String,
    pub uri: String,
    pub digest: Option<String>,
    pub media_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub producer: Option<String>,
    pub time_range: Option<ArtifactTimeRange>,
    pub availability: ArtifactAvailability,
}

impl EpisodeArtifactManifest {
    /// Convert a host-validated frame reference into complete artifact metadata.
    /// The frame bytes remain external; camera/time provenance is retained.
    pub fn from_frame_ref(frame: &FrameRef) -> Result<Self, String> {
        frame.validate()?;
        let manifest = Self {
            kind: "frame".into(),
            uri: frame.uri.clone(),
            digest: Some(frame.sha256.clone()),
            media_type: Some(frame.mime_type.clone()),
            size_bytes: Some(frame.byte_len),
            producer: Some(format!("camera:{}", frame.camera_id)),
            time_range: Some(ArtifactTimeRange {
                start_unix_ms: frame.source_time_ms,
                end_unix_ms: frame.source_time_ms,
            }),
            availability: ArtifactAvailability::Available,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Preserve a legacy provider evidence reference without inventing missing
    /// provenance. Content-addressed URI digests are retained when valid; every
    /// absent manifest field is named explicitly.
    pub fn from_legacy_evidence(reference: &EvidenceRef) -> Self {
        let digest = reference
            .uri
            .strip_prefix("artifact://sha256/")
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(str::to_owned);
        let mut missing_fields = vec![
            "media_type".to_string(),
            "size_bytes".to_string(),
            "producer".to_string(),
            "time_range".to_string(),
        ];
        if digest.is_none() {
            missing_fields.insert(0, "digest".to_string());
        }
        Self {
            kind: reference.kind.clone(),
            uri: reference.uri.clone(),
            digest,
            media_type: None,
            size_bytes: None,
            producer: None,
            time_range: None,
            availability: ArtifactAvailability::MetadataIncomplete { missing_fields },
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind.trim().is_empty() || self.kind.len() > 128 {
            return Err("artifact kind is empty or too long".into());
        }
        if self.uri.trim().is_empty() || self.uri.len() > 4_096 {
            return Err("artifact URI is empty or too long".into());
        }
        if is_inline_artifact_uri(&self.uri) {
            return Err("artifact URI must reference external content, not inline bytes".into());
        }
        let complete = self.digest.is_some()
            && self.media_type.is_some()
            && self.size_bytes.is_some()
            && self.producer.is_some()
            && self.time_range.is_some();
        if !complete
            && !matches!(
                self.availability,
                ArtifactAvailability::MetadataIncomplete { .. }
            )
        {
            return Err("available/tombstoned artifact manifest is incomplete".into());
        }
        if let Some(digest) = &self.digest {
            let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("artifact digest must be a lowercase SHA-256 hex digest".into());
            }
            if let Some(uri_digest) = self.uri.strip_prefix("artifact://sha256/") {
                if uri_digest != digest {
                    return Err("artifact URI digest does not match manifest digest".into());
                }
            }
        }
        if let Some(media_type) = &self.media_type {
            if media_type.trim().is_empty() || media_type.len() > 256 {
                return Err("artifact media type is empty or too long".into());
            }
        }
        if self.size_bytes == Some(0) {
            return Err("artifact size must be nonzero".into());
        }
        if let Some(producer) = &self.producer {
            if producer.trim().is_empty() || producer.len() > 1_024 {
                return Err("artifact producer is empty or too long".into());
            }
        }
        if let Some(range) = self.time_range {
            if range.start_unix_ms < 0 || range.end_unix_ms < range.start_unix_ms {
                return Err("artifact time range is invalid".into());
            }
        }
        match &self.availability {
            ArtifactAvailability::Available => {}
            ArtifactAvailability::RetentionExpired {
                expired_at_unix_ms,
                reason,
            } => {
                if *expired_at_unix_ms < 0 || reason.trim().is_empty() {
                    return Err("artifact retention tombstone is invalid".into());
                }
            }
            ArtifactAvailability::MetadataIncomplete { missing_fields } => {
                if complete || missing_fields.is_empty() {
                    return Err("artifact metadata-incomplete state is inconsistent".into());
                }
                let actual_missing = [
                    ("digest", self.digest.is_none()),
                    ("media_type", self.media_type.is_none()),
                    ("size_bytes", self.size_bytes.is_none()),
                    ("producer", self.producer.is_none()),
                    ("time_range", self.time_range.is_none()),
                ]
                .into_iter()
                .filter_map(|(field, missing)| missing.then_some(field))
                .collect::<std::collections::BTreeSet<_>>();
                let declared_missing = missing_fields
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>();
                if declared_missing != actual_missing {
                    return Err("artifact missing-field declaration is inaccurate".into());
                }
            }
        }
        Ok(())
    }

    fn apply_retention_tombstone(
        &mut self,
        tombstone: &ArtifactRetentionTombstone,
    ) -> Result<bool, String> {
        tombstone.validate()?;
        let manifest_digest = self
            .digest
            .as_deref()
            .map(|digest| digest.strip_prefix("sha256:").unwrap_or(digest));
        let tombstone_digest = tombstone
            .digest
            .strip_prefix("sha256:")
            .unwrap_or(&tombstone.digest);
        if self.uri != tombstone.uri && manifest_digest != Some(tombstone_digest) {
            return Ok(false);
        }
        if manifest_digest != Some(tombstone_digest) {
            return Err("artifact tombstone digest conflicts with report manifest".into());
        }
        self.availability = ArtifactAvailability::RetentionExpired {
            expired_at_unix_ms: tombstone.expired_at_unix_ms,
            reason: tombstone.reason.clone(),
        };
        Ok(true)
    }
}

/// One recorded attempt within an episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt: u32,
    /// Independent attempt identifier — always present, even when the
    /// underlying operation was never created.
    pub attempt_id: String,
    /// Host/provider-issued typed operation id, if the operation was created.
    /// `None` for pre-execution failures — never a fabricated id.
    pub operation_id: Option<String>,
    /// Exact governed request submitted to the embodiment execution boundary.
    /// Legacy reports may omit it; current-format reports require it for every
    /// attempt so the selected skill and final parameters are auditable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<SkillRequest>,
    /// Expected outcome carried by the proposal for this attempt.
    pub expected: ExpectedOutcome,
    /// Provider terminal outcome (e.g. "succeeded"), if the attempt executed.
    pub result_outcome: Option<String>,
    pub verification_decision: Option<VerificationDecision>,
    #[serde(default)]
    pub verification_observed_paths: Vec<String>,
    pub verification_reasons: Vec<String>,
    /// Why a retry/replan was taken (from VerificationReport reasons), if any.
    pub retry_reason: Option<String>,
    /// World sequence used to authorize/execute this attempt.
    pub before_sequence: Option<u64>,
    /// First post-execution world sequence supplied to verification.
    pub after_sequence: Option<u64>,
    /// Exact sequence named by the deterministic verification report.
    pub verified_sequence: Option<u64>,
    /// Bounded external evidence references associated with this attempt. Bytes
    /// are never embedded; the top-level artifact manifest is derived from all
    /// ordered attempts so retry evidence is not lost.
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
}

impl AttemptRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn from_verification(
        attempt: u32,
        attempt_id: String,
        operation_id: Option<String>,
        request: Option<SkillRequest>,
        expected: ExpectedOutcome,
        result_outcome: Option<String>,
        verification: Option<&VerificationReport>,
        retry_reason: Option<String>,
        before_sequence: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Self {
        Self {
            attempt,
            attempt_id,
            operation_id,
            request,
            expected,
            result_outcome,
            verification_decision: verification.map(|v| v.decision.clone()),
            verification_observed_paths: verification
                .map(|v| v.observed_paths.clone())
                .unwrap_or_default(),
            verification_reasons: verification.map(|v| v.reasons.clone()).unwrap_or_default(),
            retry_reason,
            before_sequence,
            after_sequence,
            verified_sequence: verification.map(|report| report.evaluated_sequence),
            evidence_refs: verification
                .map(|report| report.evidence.clone())
                .unwrap_or_default(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.attempt == 0 || self.attempt_id.trim().is_empty() {
            return Err("attempt number/id is invalid".into());
        }
        if let (Some(before), Some(after)) = (self.before_sequence, self.after_sequence) {
            if after < before {
                return Err(format!(
                    "attempt {} after sequence precedes before",
                    self.attempt
                ));
            }
        }
        if let (Some(after), Some(verified)) = (self.after_sequence, self.verified_sequence) {
            if verified < after {
                return Err(format!(
                    "attempt {} verified sequence precedes after",
                    self.attempt
                ));
            }
        }
        for evidence in &self.evidence_refs {
            if evidence.kind.trim().is_empty()
                || evidence.kind.len() > 128
                || evidence.uri.trim().is_empty()
                || evidence.uri.len() > 4_096
                || is_inline_artifact_uri(&evidence.uri)
            {
                return Err(format!(
                    "attempt {} contains an invalid or inline evidence reference",
                    self.attempt
                ));
            }
        }
        Ok(())
    }
}

/// Structured, serializable record of one embodied task episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeReport {
    /// Version zero denotes a legacy receipt created before exact SkillRequest
    /// journaling. Omitting zero preserves the immutable digest of those
    /// already-settled reports across deserialization.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub format_version: u32,
    pub episode_id: String,
    pub goal: String,
    pub device: DeviceId,
    pub sim_scene_version: String,
    pub aletheon_commit: String,
    pub bridge_protocol_digest: String,
    pub skill_descriptor_digest: String,
    /// Host-bound Policy identity. Provider/protocol come from the negotiated
    /// connection; model/version/digest come from typed response fields.
    #[serde(default)]
    pub policy_provenance: Option<PolicyProvenance>,
    /// Ordered typed failure history. The first failure remains the primary
    /// cause; safe-stop/settlement failures append and never overwrite it.
    #[serde(default)]
    pub failures: Vec<RobotFailure>,
    /// Terminal result observed from the local safe-stop boundary. Absence means
    /// safe-stop was not observed; callers must not infer it from settlement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_stop: Option<SafeStopReceipt>,
    /// Host-selected frame provenance only. Image bytes are never embedded.
    #[serde(default)]
    pub selected_frames: Vec<FrameRef>,
    pub before_sequence: Option<u64>,
    pub after_sequence: Option<u64>,
    pub verified_sequence: Option<u64>,
    pub settlement: EpisodeSettlement,
    pub attempts: Vec<AttemptRecord>,
    /// Large artifacts are manifests/references only, never inline bytes.
    pub artifacts: Vec<EpisodeArtifactManifest>,
}

/// Inputs required to assemble an authoritative episode report.
pub struct EpisodeReportInput {
    pub episode_id: String,
    pub goal: String,
    pub device: DeviceId,
    pub sim_scene_version: String,
    pub aletheon_commit: String,
    pub bridge_protocol_digest: String,
    pub skill_descriptor_digest: String,
    pub policy_provenance: Option<PolicyProvenance>,
    pub failures: Vec<RobotFailure>,
    pub safe_stop: Option<SafeStopReceipt>,
    pub selected_frames: Vec<FrameRef>,
    pub settlement: EpisodeSettlement,
    pub attempts: Vec<AttemptRecord>,
    pub artifacts: Vec<EpisodeArtifactManifest>,
}

impl EpisodeReport {
    pub fn final_decision(&self) -> Option<VerificationDecision> {
        self.attempts
            .last()
            .and_then(|attempt| attempt.verification_decision.clone())
    }

    /// Promotion gate: only a `Matched` and completed episode may promote into
    /// Mnemosyne. Failed/unknown episodes keep their evidence but are never
    /// distilled as successful experience.
    pub fn can_promote(&self) -> bool {
        self.settlement == EpisodeSettlement::Completed
            && self.final_decision() == Some(VerificationDecision::Matched)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format_version > 1 {
            return Err(format!(
                "unsupported episode report format version: {}",
                self.format_version
            ));
        }
        for (name, value) in [
            ("episode_id", self.episode_id.as_str()),
            ("goal", self.goal.as_str()),
            ("device", self.device.0.as_str()),
            ("sim_scene_version", self.sim_scene_version.as_str()),
            ("aletheon_commit", self.aletheon_commit.as_str()),
            (
                "bridge_protocol_digest",
                self.bridge_protocol_digest.as_str(),
            ),
            (
                "skill_descriptor_digest",
                self.skill_descriptor_digest.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(format!("episode report {name} is empty"));
            }
        }
        let mut expected_attempt = 1;
        let mut attempt_ids = HashSet::new();
        let mut operation_ids = HashSet::new();
        for attempt in &self.attempts {
            attempt.validate()?;
            if self.format_version >= 1 {
                let request = attempt.request.as_ref().ok_or_else(|| {
                    format!(
                        "current-format episode attempt {} has no governed skill request",
                        attempt.attempt
                    )
                })?;
                if request.skill.0.trim().is_empty() {
                    return Err(format!(
                        "episode attempt {} request skill is empty",
                        attempt.attempt
                    ));
                }
                if request.device != self.device {
                    return Err(format!(
                        "episode attempt {} request device does not match report device",
                        attempt.attempt
                    ));
                }
                if !request.parameters.is_object() {
                    return Err(format!(
                        "episode attempt {} request parameters are not an object",
                        attempt.attempt
                    ));
                }
            }
            if attempt.attempt != expected_attempt {
                return Err("episode attempts are not contiguous from attempt 1".into());
            }
            expected_attempt = expected_attempt.saturating_add(1);
            if !attempt_ids.insert(attempt.attempt_id.as_str()) {
                return Err("episode contains duplicate attempt ids".into());
            }
            if let Some(operation_id) = &attempt.operation_id {
                if !operation_ids.insert(operation_id.as_str()) {
                    return Err("episode contains duplicate operation ids".into());
                }
            }
        }
        let expected_before = self
            .attempts
            .first()
            .and_then(|attempt| attempt.before_sequence);
        let expected_after = self
            .attempts
            .last()
            .and_then(|attempt| attempt.after_sequence);
        let expected_verified = self
            .attempts
            .last()
            .and_then(|attempt| attempt.verified_sequence);
        if self.before_sequence != expected_before
            || self.after_sequence != expected_after
            || self.verified_sequence != expected_verified
        {
            return Err("episode summary sequences do not match ordered attempts".into());
        }
        if self.settlement == EpisodeSettlement::Completed && !self.can_promote() {
            return Err("completed episode does not have a final matched verification".into());
        }
        if self.settlement == EpisodeSettlement::Completed && self.policy_provenance.is_none() {
            return Err("completed episode has no host-bound policy/model provenance".into());
        }
        if self.settlement == EpisodeSettlement::Completed && self.safe_stop.is_some() {
            return Err("completed episode cannot contain a safe-stop receipt".into());
        }
        if let Some(safe_stop) = &self.safe_stop {
            let final_attempt = self
                .attempts
                .last()
                .map(|attempt| attempt.attempt)
                .unwrap_or(0);
            if safe_stop.attempted_after_attempt > final_attempt {
                return Err("safe-stop receipt references an unknown attempt".into());
            }
            if let Some(trigger) = safe_stop.trigger {
                if !self.failures.iter().any(|failure| failure.class == trigger) {
                    return Err("safe-stop trigger is absent from the typed failure history".into());
                }
            }
            let has_safe_stop_failure = self
                .failures
                .iter()
                .any(|failure| failure.class == RobotFailureClass::SafeStopFailure);
            if (safe_stop.outcome == SafeStopOutcome::Failed) != has_safe_stop_failure {
                return Err("safe-stop outcome conflicts with the typed failure history".into());
            }
        }
        if let Some(policy) = &self.policy_provenance {
            for (name, value) in [
                ("policy.provider", policy.provider.as_str()),
                ("policy.model", policy.model.as_str()),
                ("policy.version", policy.version.as_str()),
                ("policy.protocol_version", policy.protocol_version.as_str()),
                ("policy.digest", policy.digest.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("episode report {name} is empty"));
                }
            }
        }
        for frame in &self.selected_frames {
            frame.validate()?;
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

/// Immutable report receipt accepted by persistence and promotion boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettledEpisodeReport {
    report: EpisodeReport,
    report_sha256: String,
    settled_at_unix_ms: i64,
}

impl SettledEpisodeReport {
    pub fn new(report: EpisodeReport, settled_at_unix_ms: i64) -> Result<Self, String> {
        if settled_at_unix_ms < 0 {
            return Err("settlement time must be non-negative".into());
        }
        report.validate()?;
        let report_sha256 = report_digest(&report)?;
        Ok(Self {
            report,
            report_sha256,
            settled_at_unix_ms,
        })
    }

    pub fn report(&self) -> &EpisodeReport {
        &self.report
    }

    pub fn report_sha256(&self) -> &str {
        &self.report_sha256
    }

    pub const fn settled_at_unix_ms(&self) -> i64 {
        self.settled_at_unix_ms
    }

    pub fn verify_integrity(&self) -> Result<(), String> {
        self.report.validate()?;
        let actual = report_digest(&self.report)?;
        if actual != self.report_sha256 {
            return Err(format!(
                "settled episode report digest mismatch: expected={} actual={actual}",
                self.report_sha256
            ));
        }
        Ok(())
    }

    /// Promotion is decided only from a verified immutable receipt.
    pub fn can_promote(&self) -> Result<bool, String> {
        self.verify_integrity()?;
        Ok(self.report.can_promote())
    }

    /// Build a report view with durable retention tombstones applied.
    ///
    /// The receipt and its digest remain unchanged; only this read projection
    /// reports that referenced evidence is past retention.
    pub fn project_with_retention(
        &self,
        tombstones: &[ArtifactRetentionTombstone],
    ) -> Result<EpisodeReport, String> {
        self.verify_integrity()?;
        let mut projected = self.report.clone();
        for tombstone in tombstones {
            let mut matched = false;
            for artifact in &mut projected.artifacts {
                matched |= artifact.apply_retention_tombstone(tombstone)?;
            }
            if !matched {
                return Err(format!(
                    "artifact tombstone is not referenced by episode {}: {}",
                    projected.episode_id, tombstone.uri
                ));
            }
        }
        projected.validate()?;
        Ok(projected)
    }
}

fn report_digest(report: &EpisodeReport) -> Result<String, String> {
    let bytes = serde_json::to_vec(report).map_err(|error| format!("serialize report: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Assemble an episode report from host metadata and recorded attempts.
pub fn build_report(input: EpisodeReportInput) -> EpisodeReport {
    let before_sequence = input
        .attempts
        .first()
        .and_then(|attempt| attempt.before_sequence);
    let after_sequence = input
        .attempts
        .last()
        .and_then(|attempt| attempt.after_sequence);
    let verified_sequence = input
        .attempts
        .last()
        .and_then(|attempt| attempt.verified_sequence);
    EpisodeReport {
        format_version: 1,
        episode_id: input.episode_id,
        goal: input.goal,
        device: input.device,
        sim_scene_version: input.sim_scene_version,
        aletheon_commit: input.aletheon_commit,
        bridge_protocol_digest: input.bridge_protocol_digest,
        skill_descriptor_digest: input.skill_descriptor_digest,
        policy_provenance: input.policy_provenance,
        failures: input.failures,
        safe_stop: input.safe_stop,
        selected_frames: input.selected_frames,
        before_sequence,
        after_sequence,
        verified_sequence,
        settlement: input.settlement,
        attempts: input.attempts,
        artifacts: input.artifacts,
    }
}

const fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::expected_outcome::OutcomePredicate;

    fn expected() -> ExpectedOutcome {
        ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5_000,
        }
    }

    fn matched_report() -> VerificationReport {
        VerificationReport {
            decision: VerificationDecision::Matched,
            evaluated_sequence: 2,
            observed_paths: vec!["mode".into()],
            reasons: vec![],
            evidence: vec![],
        }
    }

    fn sample_report(settlement: EpisodeSettlement) -> EpisodeReport {
        let digest = "b".repeat(64);
        build_report(EpisodeReportInput {
            episode_id: "ep-1".into(),
            goal: "stand".into(),
            device: DeviceId("kuavo-mujoco-01".into()),
            sim_scene_version: "mujoco-v1".into(),
            aletheon_commit: "abc123".into(),
            bridge_protocol_digest: "sha256:proto".into(),
            skill_descriptor_digest: "sha256:skills".into(),
            policy_provenance: Some(PolicyProvenance {
                provider: "openvla-local".into(),
                model: "openvla-7b".into(),
                version: "1".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:model".into(),
            }),
            failures: vec![],
            safe_stop: (settlement != EpisodeSettlement::Completed).then_some(SafeStopReceipt {
                attempted_after_attempt: 1,
                trigger: None,
                outcome: SafeStopOutcome::Succeeded,
            }),
            selected_frames: vec![sample_frame()],
            settlement,
            attempts: vec![AttemptRecord::from_verification(
                1,
                "attempt:ep-1:1".into(),
                Some("00000000-0000-0000-0000-000000000001".into()),
                Some(SkillRequest {
                    skill: crate::types::embodiment::SkillId("kuavo.stance".into()),
                    device: DeviceId("kuavo-mujoco-01".into()),
                    parameters: serde_json::json!({}),
                }),
                expected(),
                Some("succeeded".into()),
                Some(&matched_report()),
                None,
                Some(1),
                Some(2),
            )],
            artifacts: vec![EpisodeArtifactManifest {
                kind: "rosbag".into(),
                uri: format!("artifact://sha256/{digest}"),
                digest: Some(digest),
                media_type: Some("application/x-rosbag".into()),
                size_bytes: Some(4_096),
                producer: Some("kuavo-bridge".into()),
                time_range: Some(ArtifactTimeRange {
                    start_unix_ms: 1_000,
                    end_unix_ms: 2_000,
                }),
                availability: ArtifactAvailability::Available,
            }],
        })
    }

    #[test]
    fn settlement_is_the_single_turn_stop_authority() {
        assert_eq!(
            EpisodeSettlement::Completed.turn_stop(),
            TurnStop::Completed
        );
        assert_eq!(EpisodeSettlement::Failed.turn_stop(), TurnStop::Blocked);
        assert_eq!(
            EpisodeSettlement::Cancelled.turn_stop(),
            TurnStop::Cancelled
        );
    }

    #[test]
    fn matched_and_settled_can_promote() {
        assert!(sample_report(EpisodeSettlement::Completed).can_promote());
    }

    #[test]
    fn failed_or_unknown_never_promote() {
        assert!(!sample_report(EpisodeSettlement::Failed).can_promote());
        let mut report = sample_report(EpisodeSettlement::Completed);
        report.attempts[0].verification_decision = Some(VerificationDecision::Unsafe);
        assert!(!report.can_promote());
        assert!(report.validate().is_err());
    }

    #[test]
    fn safe_stop_receipt_is_observed_evidence_not_a_settlement_inference() {
        let completed = sample_report(EpisodeSettlement::Completed);
        assert!(completed.safe_stop.is_none());

        let mut failed = sample_report(EpisodeSettlement::Failed);
        assert_eq!(
            failed.safe_stop.as_ref().unwrap().outcome,
            SafeStopOutcome::Succeeded
        );
        failed.validate().unwrap();

        failed.safe_stop.as_mut().unwrap().outcome = SafeStopOutcome::Failed;
        assert!(failed.validate().is_err());
        failed.failures.push(RobotFailure::new(
            RobotFailureClass::SafeStopFailure,
            "provider rejected safe stop",
        ));
        failed.validate().unwrap();

        let mut forged = completed;
        forged.safe_stop = Some(SafeStopReceipt {
            attempted_after_attempt: 1,
            trigger: None,
            outcome: SafeStopOutcome::Succeeded,
        });
        assert!(forged.validate().is_err());
    }

    #[test]
    fn settled_report_detects_tampering_and_has_no_inline_artifact() {
        let settled = SettledEpisodeReport::new(sample_report(EpisodeSettlement::Completed), 10)
            .expect("valid settled report");
        settled.verify_integrity().unwrap();
        let json = serde_json::to_string(&settled).unwrap();
        assert!(!json.contains("base64"));

        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["report"]["goal"] = serde_json::json!("tampered");
        let tampered: SettledEpisodeReport = serde_json::from_value(value).unwrap();
        assert!(tampered.verify_integrity().is_err());
    }

    #[test]
    fn report_serde_round_trips_with_sequence_and_manifest() {
        let report = sample_report(EpisodeSettlement::Completed);
        let json = serde_json::to_string(&report).unwrap();
        let decoded: EpisodeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, report);
        assert_eq!(decoded.before_sequence, Some(1));
        assert_eq!(decoded.after_sequence, Some(2));
        assert_eq!(decoded.verified_sequence, Some(2));
        assert_eq!(decoded.artifacts[0].kind, "rosbag");
        assert_eq!(decoded.selected_frames[0].sha256, "a".repeat(64));
        assert_eq!(decoded.format_version, 1);
        let request = decoded.attempts[0].request.as_ref().unwrap();
        assert_eq!(request.skill.0, "kuavo.stance");
        assert_eq!(request.device.0, "kuavo-mujoco-01");
        assert_eq!(request.parameters, serde_json::json!({}));
        decoded.validate().unwrap();
    }

    #[test]
    fn current_report_requires_exact_attempt_request_but_legacy_digest_survives() {
        let mut current = sample_report(EpisodeSettlement::Completed);
        current.attempts[0].request = None;
        assert!(current
            .validate()
            .unwrap_err()
            .contains("no governed skill request"));

        current.format_version = 0;
        current.validate().unwrap();
        let settled = SettledEpisodeReport::new(current, 10).unwrap();
        let json = serde_json::to_string(&settled).unwrap();
        assert!(!json.contains("format_version"));
        assert!(!json.contains("\"request\""));
        let decoded: SettledEpisodeReport = serde_json::from_str(&json).unwrap();
        decoded.verify_integrity().unwrap();
    }

    #[test]
    fn immutable_receipt_projects_retention_without_changing_its_digest() {
        let settled = SettledEpisodeReport::new(sample_report(EpisodeSettlement::Completed), 10)
            .expect("valid settled report");
        let digest = settled.report_sha256().to_owned();
        let artifact = settled.report().artifacts[0].clone();
        let tombstone = ArtifactRetentionTombstone {
            uri: artifact.uri.clone(),
            digest: artifact.digest.clone().unwrap(),
            expired_at_unix_ms: 20,
            reason: "episode retention elapsed".into(),
        };

        let projection = settled.project_with_retention(&[tombstone]).unwrap();
        assert!(matches!(
            projection.artifacts[0].availability,
            ArtifactAvailability::RetentionExpired {
                expired_at_unix_ms: 20,
                ..
            }
        ));
        assert_eq!(projection.artifacts[0].digest, artifact.digest);
        assert_eq!(settled.report_sha256(), digest);
        assert_eq!(
            settled.report().artifacts[0].availability,
            ArtifactAvailability::Available
        );
        settled.verify_integrity().unwrap();
    }

    #[test]
    fn report_rejects_summary_sequence_drift() {
        let mut report = sample_report(EpisodeSettlement::Completed);
        report.after_sequence = Some(99);
        assert!(report.validate().is_err());
    }

    #[test]
    fn report_rejects_attempt_gaps_and_inline_artifact_uri() {
        let mut gap = sample_report(EpisodeSettlement::Completed);
        gap.attempts[0].attempt = 2;
        assert!(gap.validate().is_err());

        let mut inline = sample_report(EpisodeSettlement::Completed);
        inline.artifacts[0].uri = "DATA:application/octet-stream;base64,AAAA".into();
        assert!(inline.validate().is_err());

        let mut inline_evidence = sample_report(EpisodeSettlement::Completed);
        inline_evidence.attempts[0].evidence_refs.push(EvidenceRef {
            kind: "log".into(),
            uri: "InLiNe:raw-log-bytes".into(),
        });
        assert!(inline_evidence.validate().is_err());
    }

    fn sample_frame() -> FrameRef {
        let digest = "a".repeat(64);
        FrameRef {
            uri: format!("artifact://sha256/{digest}"),
            sha256: digest,
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            byte_len: 32_000,
            source_time_ms: 1_000,
            camera_id: "front".into(),
            frame_id: 1,
        }
    }
}
