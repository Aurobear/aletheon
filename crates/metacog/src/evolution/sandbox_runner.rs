//! Candidate-aware, bounded genome sandbox.
//!
//! A genome is data, so production verification validates and replays the exact
//! candidate. It deliberately never launches repository build tools.

use crate::genome::contracts::Genome;
use crate::governance::contracts::{RuntimeCandidate, TestResult};
use ::contracts::Clock;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::PathBuf, sync::Arc};

const CORPUS_VERSION: &str = "genome-safety-v1";
const MAX_CASES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSandboxReceipt {
    pub candidate_id: uuid::Uuid,
    pub candidate_digest: String,
    pub corpus_version: String,
    pub passed_cases: Vec<String>,
    pub failed_cases: Vec<String>,
    pub elapsed_ms: u64,
    pub terminal_status: String,
}

pub struct SandboxRunner {
    receipt_dir: Option<PathBuf>,
    clock: Arc<dyn Clock>,
}

impl SandboxRunner {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            receipt_dir: None,
            clock,
        }
    }

    /// Compatibility constructor: the directory now stores sandbox receipts;
    /// it is never used as a process working directory.
    pub fn with_work_dir(work_dir: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            receipt_dir: Some(work_dir.join("candidate-sandbox-receipts")),
            clock,
        }
    }

    pub async fn run_tests_against(
        &self,
        candidate: &RuntimeCandidate,
        baseline: Option<&Genome>,
    ) -> Result<TestResult> {
        let start = self.clock.mono_now();
        let encoded =
            serde_json::to_vec(&candidate.genome).context("serialize candidate genome")?;
        let digest = format!("{:x}", Sha256::digest(&encoded));
        let mut passed = Vec::new();
        let mut failed = Vec::new();
        let mut check = |name: &str, ok: bool| {
            if passed.len() + failed.len() >= MAX_CASES {
                return;
            }
            (if ok { &mut passed } else { &mut failed }).push(name.to_string());
        };

        check(
            "identity.name.non_empty",
            !candidate.genome.identity.name.trim().is_empty(),
        );
        check(
            "identity.self_model.non_empty",
            !candidate.genome.identity.self_model.trim().is_empty(),
        );
        let ids: HashSet<_> = candidate
            .genome
            .boundary
            .rules
            .iter()
            .map(|r| &r.id)
            .collect();
        check(
            "boundary.ids.unique",
            ids.len() == candidate.genome.boundary.rules.len(),
        );
        check(
            "boundary.fields.valid",
            candidate.genome.boundary.rules.iter().all(|r| {
                !r.id.trim().is_empty()
                    && !r.condition.trim().is_empty()
                    && !r.action.trim().is_empty()
            }),
        );
        check(
            "care.weights.finite_range",
            candidate
                .genome
                .care
                .priorities
                .iter()
                .all(|p| p.weight.is_finite() && (0.0..=1.0).contains(&p.weight)),
        );
        check(
            "mutation.sandbox.required",
            candidate.genome.mutation.require_sandbox,
        );
        check(
            "mutation.approval.required",
            candidate.genome.mutation.require_self_field_approval,
        );
        check(
            "lifecycle.intervals.nonzero",
            candidate.genome.lifecycle.health_check_interval_secs > 0
                && candidate.genome.lifecycle.max_idle_time_secs > 0,
        );
        check(
            "mutation.targets.genome_only",
            candidate.genome.mutation.allowed_targets.iter().all(|t| {
                matches!(
                    t.as_str(),
                    "care.priorities" | "boundary.rules" | "identity.name" | "identity.description"
                )
            }),
        );
        if let Some(base) = baseline {
            check(
                "identity.self_model.immutable",
                candidate.genome.identity.self_model == base.identity.self_model,
            );
            let mandatory: Vec<_> = base
                .boundary
                .rules
                .iter()
                .filter(|r| r.action.eq_ignore_ascii_case("deny") || r.id.contains("immutable"))
                .collect();
            check(
                "mandatory.boundaries.no_regression",
                mandatory.iter().all(|rule| {
                    candidate.genome.boundary.rules.iter().any(|r| {
                        r.id == rule.id
                            && r.condition == rule.condition
                            && r.action == rule.action
                            && r.priority >= rule.priority
                    })
                }),
            );
        }

        let elapsed_ms = self.clock.mono_now().0.saturating_sub(start.0);
        let receipt = CandidateSandboxReceipt {
            candidate_id: candidate.id,
            candidate_digest: digest,
            corpus_version: CORPUS_VERSION.into(),
            passed_cases: passed.clone(),
            failed_cases: failed.clone(),
            elapsed_ms,
            terminal_status: if failed.is_empty() {
                "passed"
            } else {
                "rejected"
            }
            .into(),
        };
        if let Some(dir) = &self.receipt_dir {
            std::fs::create_dir_all(dir)?;
            let final_path = dir.join(format!("{}.json", candidate.id));
            let tmp = dir.join(format!(".{}.tmp", candidate.id));
            let bytes = serde_json::to_vec_pretty(&receipt)?;
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(tmp, final_path)?;
        }
        Ok(TestResult {
            passed: failed.is_empty(),
            tests_run: passed.len() + failed.len(),
            tests_passed: passed.len(),
            tests_failed: failed.len(),
            failures: failed,
            elapsed_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{evolution::candidate::CandidateGenerator, genome::loader::GenomeLoader};
    use dasein::MutationIntent;
    use kernel::chronos::TestClock;

    #[tokio::test]
    async fn validates_candidate_without_launching_cargo() {
        let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
        let base = GenomeLoader::new()
            .load(std::path::Path::new("/missing"))
            .unwrap();
        let candidate = CandidateGenerator::new(clock.clone())
            .generate(
                &base,
                &MutationIntent {
                    target: "care.priorities".into(),
                    change: serde_json::json!({"topic":"helpfulness","weight_delta":0.01}),
                    reason: "test".into(),
                    reversible: true,
                },
            )
            .await
            .unwrap();
        let result = SandboxRunner::new(clock)
            .run_tests_against(&candidate, Some(&base))
            .await
            .unwrap();
        assert!(result.passed, "{:?}", result.failures);
        assert!(result.tests_run >= 10);
    }
}
