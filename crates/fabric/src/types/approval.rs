//! Durable approval contracts for restart-safe protected Goal operations.

use crate::{AttemptId, CodingJobId, GoalId, PrincipalId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovalId(pub Uuid);

impl ApprovalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ApprovalId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for ApprovalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCategory {
    ApplyCode,
    SendMail,
    DeleteFile,
    ModifyCalendar,
    GitPush,
    CapabilityExpansion,
    DaseinModification,
    BudgetExpansion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Consumed,
}

impl ApprovalStatus {
    pub const fn is_decided(self) -> bool {
        !matches!(self, Self::Pending)
    }
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Expired | Self::Consumed)
    }
    pub const fn can_resolve(self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// Canonical, hashable description of the protected operation.
///
/// `attributes` holds category-specific immutable facts such as base commit,
/// diff hash, verification hash, and destination. BTreeMap and normalized path
/// ordering make the subject hash stable across serialization implementations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSubject {
    pub category: ApprovalCategory,
    pub goal_id: GoalId,
    pub attempt_id: Option<AttemptId>,
    pub job_id: Option<CodingJobId>,
    pub attributes: BTreeMap<String, String>,
    pub allowed_scope: Vec<PathBuf>,
    pub apply_target: Option<PathBuf>,
}

impl ApprovalSubject {
    pub fn canonicalized(mut self) -> Result<Self, ApprovalContractError> {
        for path in self.allowed_scope.iter().chain(self.apply_target.iter()) {
            validate_relative_path(path)?;
        }
        self.allowed_scope.sort();
        self.allowed_scope.dedup();
        Ok(self)
    }

    pub fn subject_hash(&self) -> Result<String, ApprovalContractError> {
        let canonical = self.clone().canonicalized()?;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| ApprovalContractError::Serialization(error.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalArtifactRef {
    pub kind: String,
    pub relative_path: PathBuf,
    pub sha256: String,
}

impl ApprovalArtifactRef {
    pub fn validate(&self) -> Result<(), ApprovalContractError> {
        if self.kind.trim().is_empty() || self.sha256.len() != 64 || !is_lower_hex(&self.sha256) {
            return Err(ApprovalContractError::InvalidArtifact);
        }
        validate_relative_path(&self.relative_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResolution {
    pub status: ApprovalStatus,
    pub principal_id: Option<PrincipalId>,
    pub channel: Option<String>,
    pub resolved_at_ms: i64,
    pub reason: Option<String>,
}

impl ApprovalResolution {
    pub fn approved(principal_id: PrincipalId, channel: impl Into<String>, at_ms: i64) -> Self {
        Self {
            status: ApprovalStatus::Approved,
            principal_id: Some(principal_id),
            channel: Some(channel.into()),
            resolved_at_ms: at_ms,
            reason: None,
        }
    }
    pub fn rejected(
        principal_id: PrincipalId,
        channel: impl Into<String>,
        at_ms: i64,
        reason: Option<String>,
    ) -> Self {
        Self {
            status: ApprovalStatus::Rejected,
            principal_id: Some(principal_id),
            channel: Some(channel.into()),
            resolved_at_ms: at_ms,
            reason,
        }
    }
    pub fn expired(at_ms: i64) -> Self {
        Self {
            status: ApprovalStatus::Expired,
            principal_id: None,
            channel: None,
            resolved_at_ms: at_ms,
            reason: Some("approval expired and was denied".into()),
        }
    }

    fn validate(&self) -> Result<(), ApprovalContractError> {
        match self.status {
            ApprovalStatus::Approved | ApprovalStatus::Rejected => {
                if self.principal_id.is_none() || self.channel.as_deref().is_none_or(str::is_empty)
                {
                    return Err(ApprovalContractError::InvalidResolution);
                }
            }
            ApprovalStatus::Expired => {
                if self.principal_id.is_some() || self.channel.is_some() {
                    return Err(ApprovalContractError::InvalidResolution);
                }
            }
            _ => return Err(ApprovalContractError::InvalidResolution),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSnapshot {
    pub id: ApprovalId,
    pub goal_id: GoalId,
    pub attempt_id: Option<AttemptId>,
    pub job_id: Option<CodingJobId>,
    pub owner_id: PrincipalId,
    pub category: ApprovalCategory,
    pub risk: ApprovalRisk,
    pub subject: ApprovalSubject,
    pub subject_hash: String,
    pub summary: String,
    pub artifacts: Vec<ApprovalArtifactRef>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub status: ApprovalStatus,
    pub version: u64,
    pub resolution: Option<ApprovalResolution>,
}

impl ApprovalSnapshot {
    pub fn validate(&self) -> Result<(), ApprovalContractError> {
        if self.goal_id != self.subject.goal_id
            || self.attempt_id != self.subject.attempt_id
            || self.job_id != self.subject.job_id
            || self.category != self.subject.category
            || self.subject_hash != self.subject.subject_hash()?
            || self.summary.trim().is_empty()
            || self.expires_at_ms <= self.created_at_ms
            || self
                .artifacts
                .iter()
                .any(|artifact| artifact.validate().is_err())
            || (self.status == ApprovalStatus::Pending) != self.resolution.is_none()
        {
            return Err(ApprovalContractError::InvalidSnapshot);
        }
        if let Some(resolution) = &self.resolution {
            resolution.validate()?;
            let status_matches = resolution.status == self.status
                || (self.status == ApprovalStatus::Consumed
                    && resolution.status == ApprovalStatus::Approved);
            if !status_matches || resolution.resolved_at_ms < self.created_at_ms {
                return Err(ApprovalContractError::InvalidSnapshot);
            }
        }
        Ok(())
    }

    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        self.status == ApprovalStatus::Pending && now_ms >= self.expires_at_ms
    }

    pub fn resolve(
        &self,
        expected_version: u64,
        resolution: ApprovalResolution,
    ) -> Result<Self, ApprovalContractError> {
        if self.version != expected_version {
            return Err(ApprovalContractError::VersionConflict {
                expected: expected_version,
                actual: self.version,
            });
        }
        if !self.status.can_resolve() || self.resolution.is_some() {
            return Err(ApprovalContractError::AlreadyDecided);
        }
        resolution.validate()?;
        if resolution.resolved_at_ms >= self.expires_at_ms
            && resolution.status != ApprovalStatus::Expired
        {
            return Err(ApprovalContractError::Expired);
        }
        let mut next = self.clone();
        next.status = resolution.status;
        next.resolution = Some(resolution);
        next.version = next.version.saturating_add(1);
        next.validate()?;
        Ok(next)
    }

    pub fn consume(&self, expected_version: u64) -> Result<Self, ApprovalContractError> {
        if self.version != expected_version {
            return Err(ApprovalContractError::VersionConflict {
                expected: expected_version,
                actual: self.version,
            });
        }
        if self.status != ApprovalStatus::Approved {
            return Err(ApprovalContractError::NotApproved);
        }
        let mut next = self.clone();
        next.status = ApprovalStatus::Consumed;
        next.version = next.version.saturating_add(1);
        // Consumption preserves the original owner decision metadata.
        next.validate_consumed()?;
        Ok(next)
    }

    fn validate_consumed(&self) -> Result<(), ApprovalContractError> {
        if self.status != ApprovalStatus::Consumed
            || self.resolution.as_ref().map(|resolution| resolution.status)
                != Some(ApprovalStatus::Approved)
        {
            return Err(ApprovalContractError::InvalidSnapshot);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalContractError {
    InvalidPath(PathBuf),
    InvalidArtifact,
    InvalidResolution,
    InvalidSnapshot,
    AlreadyDecided,
    Expired,
    NotApproved,
    VersionConflict { expected: u64, actual: u64 },
    Serialization(String),
}
impl fmt::Display for ApprovalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for ApprovalContractError {}

fn validate_relative_path(path: &Path) -> Result<(), ApprovalContractError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ApprovalContractError::InvalidPath(path.to_owned()));
    }
    Ok(())
}
fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> ApprovalSnapshot {
        let subject = ApprovalSubject {
            category: ApprovalCategory::ApplyCode,
            goal_id: GoalId(7),
            attempt_id: Some(AttemptId(Uuid::from_u128(8))),
            job_id: Some(CodingJobId(Uuid::from_u128(9))),
            attributes: BTreeMap::from([
                ("base_commit".into(), "abc123".into()),
                ("diff_sha256".into(), "d".repeat(64)),
                ("verification_sha256".into(), "e".repeat(64)),
            ]),
            allowed_scope: vec![PathBuf::from("src/z"), PathBuf::from("src/a")],
            apply_target: Some(PathBuf::from(".")),
        }
        .canonicalized()
        .unwrap();
        ApprovalSnapshot {
            id: ApprovalId(Uuid::from_u128(1)),
            goal_id: subject.goal_id,
            attempt_id: subject.attempt_id,
            job_id: subject.job_id,
            owner_id: PrincipalId("owner".into()),
            category: subject.category,
            risk: ApprovalRisk::High,
            subject_hash: subject.subject_hash().unwrap(),
            subject,
            summary: "Apply verified coding diff".into(),
            artifacts: vec![ApprovalArtifactRef {
                kind: "diff".into(),
                relative_path: PathBuf::from("coding-diffs/job.diff"),
                sha256: "d".repeat(64),
            }],
            created_at_ms: 100,
            expires_at_ms: 200,
            status: ApprovalStatus::Pending,
            version: 0,
            resolution: None,
        }
    }

    #[test]
    fn contracts_round_trip_through_serde() {
        let snapshot = pending();
        snapshot.validate().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<ApprovalSnapshot>(&json).unwrap(),
            snapshot
        );
    }

    #[test]
    fn terminal_and_expiry_semantics_are_explicit() {
        assert!(!ApprovalStatus::Pending.is_terminal());
        assert!(!ApprovalStatus::Approved.is_terminal());
        assert!(ApprovalStatus::Rejected.is_terminal());
        assert!(ApprovalStatus::Expired.is_terminal());
        assert!(ApprovalStatus::Consumed.is_terminal());
        assert!(!pending().is_expired_at(199));
        assert!(pending().is_expired_at(200));
    }

    #[test]
    fn decisions_are_one_time_and_expiry_denies() {
        let snapshot = pending();
        let approved = snapshot
            .resolve(
                0,
                ApprovalResolution::approved(PrincipalId("owner".into()), "telegram", 150),
            )
            .unwrap();
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(
            approved.resolve(
                1,
                ApprovalResolution::rejected(PrincipalId("owner".into()), "telegram", 160, None)
            ),
            Err(ApprovalContractError::AlreadyDecided)
        );
        let consumed = approved.consume(1).unwrap();
        assert_eq!(consumed.status, ApprovalStatus::Consumed);
        assert_eq!(consumed.consume(2), Err(ApprovalContractError::NotApproved));
        let expired = snapshot
            .resolve(0, ApprovalResolution::expired(200))
            .unwrap();
        assert_eq!(expired.status, ApprovalStatus::Expired);
        assert!(expired.status.is_terminal());
    }

    #[test]
    fn stale_and_late_decisions_fail_closed() {
        let snapshot = pending();
        assert_eq!(
            snapshot.resolve(
                9,
                ApprovalResolution::approved(PrincipalId("owner".into()), "rpc", 150)
            ),
            Err(ApprovalContractError::VersionConflict {
                expected: 9,
                actual: 0
            })
        );
        assert_eq!(
            snapshot.resolve(
                0,
                ApprovalResolution::approved(PrincipalId("owner".into()), "rpc", 200)
            ),
            Err(ApprovalContractError::Expired)
        );
    }

    #[test]
    fn subject_hash_is_stable_after_scope_normalization() {
        let original = pending().subject;
        let mut reordered = original.clone();
        reordered.allowed_scope.reverse();
        reordered.allowed_scope.push(PathBuf::from("src/a"));
        assert_eq!(
            original.subject_hash().unwrap(),
            reordered.subject_hash().unwrap()
        );
        let mut changed = original.clone();
        changed
            .attributes
            .insert("diff_sha256".into(), "f".repeat(64));
        assert_ne!(
            original.subject_hash().unwrap(),
            changed.subject_hash().unwrap()
        );
    }
}
