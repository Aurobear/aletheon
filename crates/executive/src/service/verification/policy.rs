//! Pure aggregation policy for coding verification evidence.

use super::{VerificationCheckKind, VerificationContext};
use fabric::{VerificationCheck, VerificationReport, VerificationSeverity};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationPolicyError {
    InvalidContext(String),
    EmptyReport,
    UnknownCheck(String),
    DuplicateCheck(String),
    UnselectedCheck(String),
    MissingCheck(String),
    SeverityMismatch(String),
    InvalidTimeRange,
}

impl fmt::Display for VerificationPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContext(message) => write!(f, "invalid verification context: {message}"),
            Self::EmptyReport => write!(f, "verification report must not be empty"),
            Self::UnknownCheck(name) => write!(f, "unknown verification check: {name}"),
            Self::DuplicateCheck(name) => write!(f, "duplicate verification check: {name}"),
            Self::UnselectedCheck(name) => write!(f, "unselected verification check: {name}"),
            Self::MissingCheck(name) => write!(f, "missing verification check: {name}"),
            Self::SeverityMismatch(name) => {
                write!(f, "verification severity does not match policy: {name}")
            }
            Self::InvalidTimeRange => write!(f, "verification end time precedes start time"),
        }
    }
}

impl std::error::Error for VerificationPolicyError {}

#[derive(Debug, Clone, Copy, Default)]
pub struct VerificationPolicy;

impl VerificationPolicy {
    pub fn evaluate(
        &self,
        context: &VerificationContext,
        checks: Vec<VerificationCheck>,
        started_at_ms: i64,
        ended_at_ms: i64,
    ) -> Result<VerificationReport, VerificationPolicyError> {
        context
            .validate()
            .map_err(|error| VerificationPolicyError::InvalidContext(error.to_string()))?;
        if checks.is_empty() {
            return Err(VerificationPolicyError::EmptyReport);
        }
        if ended_at_ms < started_at_ms {
            return Err(VerificationPolicyError::InvalidTimeRange);
        }

        let mut indexed = BTreeMap::new();
        for check in checks {
            let kind = VerificationCheckKind::parse(&check.name)
                .ok_or_else(|| VerificationPolicyError::UnknownCheck(check.name.clone()))?;
            if !context.selection.contains(kind) {
                return Err(VerificationPolicyError::UnselectedCheck(check.name));
            }
            let expected = if kind.required() {
                VerificationSeverity::Required
            } else {
                VerificationSeverity::Advisory
            };
            if check.severity != expected {
                return Err(VerificationPolicyError::SeverityMismatch(check.name));
            }
            if indexed.insert(kind, check).is_some() {
                return Err(VerificationPolicyError::DuplicateCheck(
                    kind.as_str().into(),
                ));
            }
        }

        let mut ordered = Vec::with_capacity(context.selection.checks().len());
        for kind in context.selection.checks() {
            let check = indexed
                .remove(kind)
                .ok_or_else(|| VerificationPolicyError::MissingCheck(kind.as_str().into()))?;
            ordered.push(check);
        }

        let passed = ordered.iter().all(|check| {
            check.severity != VerificationSeverity::Required
                || (check.passed && !check.timed_out && !check.cancelled)
        });
        let risk_summary = ordered
            .iter()
            .filter(|check| {
                check.severity == VerificationSeverity::Advisory
                    && (!check.passed || check.timed_out || check.cancelled)
            })
            .map(|check| format!("{}: {}", check.name, check.summary))
            .collect();

        Ok(VerificationReport {
            job_id: context.job_id,
            goal_id: context.goal_id,
            attempt_id: context.attempt_id,
            passed,
            checks: ordered,
            risk_summary,
            started_at_ms,
            ended_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::verification::{CapabilityAuditSummary, VerificationSelection};
    use fabric::{AttemptId, CodingJobId, GoalId};
    use tempfile::TempDir;

    fn context(temp: &TempDir) -> VerificationContext {
        VerificationContext {
            job_id: CodingJobId::new(),
            goal_id: GoalId(4),
            attempt_id: AttemptId::new(),
            worktree: temp.path().canonicalize().unwrap(),
            base_commit: "0123456789abcdef".into(),
            changed_files: vec![],
            capability_audit: CapabilityAuditSummary {
                audit_present: true,
                observed_capabilities: vec![],
                allowed_capabilities: vec![],
            },
            selection: VerificationSelection::default(),
        }
    }

    fn check(kind: VerificationCheckKind, passed: bool) -> VerificationCheck {
        VerificationCheck {
            name: kind.as_str().into(),
            severity: if kind.required() {
                VerificationSeverity::Required
            } else {
                VerificationSeverity::Advisory
            },
            passed,
            timed_out: false,
            cancelled: false,
            summary: if passed { "passed" } else { "failed" }.into(),
            evidence: vec![],
        }
    }

    fn all_checks() -> Vec<VerificationCheck> {
        VerificationCheckKind::REQUIRED
            .into_iter()
            .chain(VerificationCheckKind::ADVISORY)
            .map(|kind| check(kind, true))
            .rev()
            .collect()
    }

    #[test]
    fn ordering_is_deterministic_and_report_serializes() {
        let temp = TempDir::new().unwrap();
        let report = VerificationPolicy
            .evaluate(&context(&temp), all_checks(), 10, 20)
            .unwrap();
        let names: Vec<_> = report
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert_eq!(
            names,
            VerificationSelection::default()
                .checks()
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
        );
        let encoded = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<VerificationReport>(&encoded).unwrap(),
            report
        );
        let context = context(&temp);
        let encoded = serde_json::to_string(&context).unwrap();
        assert_eq!(
            serde_json::from_str::<VerificationContext>(&encoded).unwrap(),
            context
        );
    }

    #[test]
    fn duplicate_empty_and_missing_checks_are_rejected() {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        assert_eq!(
            VerificationPolicy.evaluate(&context, vec![], 0, 1),
            Err(VerificationPolicyError::EmptyReport)
        );
        let mut checks = all_checks();
        checks.push(check(VerificationCheckKind::Format, true));
        assert!(matches!(
            VerificationPolicy.evaluate(&context, checks, 0, 1),
            Err(VerificationPolicyError::DuplicateCheck(_))
        ));
        let mut checks = all_checks();
        checks.retain(|check| check.name != VerificationCheckKind::Compile.as_str());
        assert!(matches!(
            VerificationPolicy.evaluate(&context, checks, 0, 1),
            Err(VerificationPolicyError::MissingCheck(_))
        ));
    }

    #[test]
    fn required_failure_timeout_or_cancel_blocks_pass() {
        let temp = TempDir::new().unwrap();
        for mode in 0..3 {
            let mut checks = all_checks();
            let compile = checks
                .iter_mut()
                .find(|check| check.name == VerificationCheckKind::Compile.as_str())
                .unwrap();
            match mode {
                0 => compile.passed = false,
                1 => compile.timed_out = true,
                _ => compile.cancelled = true,
            }
            assert!(
                !VerificationPolicy
                    .evaluate(&context(&temp), checks, 0, 1)
                    .unwrap()
                    .passed
            );
        }
    }

    #[test]
    fn advisory_failure_adds_risk_without_blocking() {
        let temp = TempDir::new().unwrap();
        let mut checks = all_checks();
        let clippy = checks
            .iter_mut()
            .find(|check| check.name == VerificationCheckKind::Clippy.as_str())
            .unwrap();
        clippy.passed = false;
        clippy.summary = "warning debt".into();
        let report = VerificationPolicy
            .evaluate(&context(&temp), checks, 0, 1)
            .unwrap();
        assert!(report.passed);
        assert_eq!(report.risk_summary, vec!["clippy: warning debt"]);
    }

    #[test]
    fn selection_rejects_duplicates_and_missing_required_checks() {
        assert!(VerificationSelection::new(vec![
            VerificationCheckKind::DiffScope,
            VerificationCheckKind::DiffScope,
        ])
        .is_err());
        assert!(VerificationSelection::new(vec![VerificationCheckKind::Format]).is_err());
    }
}
