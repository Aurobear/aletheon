//! Morphogenesis Pipeline — the self-evolution flow.
//!
//! Pipeline: read_genome → generate_candidate → sandbox_test → evaluate → migrate
//!
//! Orchestrates the MetaRuntimeOps trait methods in sequence.

use anyhow::Result;
use fabric::{Evaluation, MetaRuntimeOps, MigrationResult, MutationIntent, RuntimeCandidate};

/// Orchestrates the full morphogenesis pipeline.
pub struct MorphogenesisPipeline<M: MetaRuntimeOps> {
    meta_runtime: M,
}

impl<M: MetaRuntimeOps> MorphogenesisPipeline<M> {
    pub fn new(meta_runtime: M) -> Self {
        Self { meta_runtime }
    }

    /// Run the full pipeline: read_genome → generate_candidate → sandbox_test → evaluate → migrate.
    ///
    /// Takes a pre-generated MutationIntent, produces a candidate, tests it,
    /// evaluates the test results, and migrates if the evaluation recommends adoption.
    pub async fn run(&self, intent: &MutationIntent) -> Result<PipelineResult> {
        tracing::info!(
            "Starting morphogenesis pipeline for target: {}",
            intent.target
        );

        // Step 1: Generate candidate from intent
        let candidate = self.meta_runtime.generate_candidate(intent).await?;
        tracing::info!(
            "Generated candidate {} with {} change(s)",
            candidate.id,
            candidate.changes.len()
        );

        // Step 2: Sandbox test -- rollback on error (candidate was already generated)
        let test_result = match self.meta_runtime.sandbox_test(&candidate).await {
            Ok(t) => t,
            Err(e) => {
                let _ = self.meta_runtime.rollback().await;
                return Err(e);
            }
        };
        tracing::info!(
            "Sandbox test: {} passed, {} failed ({}ms)",
            test_result.tests_passed,
            test_result.tests_failed,
            test_result.elapsed_ms
        );

        // Step 3: Evaluate -- rollback on error
        let evaluation = match self.meta_runtime.evaluate(&candidate, &test_result).await {
            Ok(v) => v,
            Err(e) => {
                let _ = self.meta_runtime.rollback().await;
                return Err(e);
            }
        };
        tracing::info!(
            "Evaluation score: {:.2}, recommendation: {:?}",
            evaluation.score,
            evaluation.recommendation
        );

        // Step 4: Migrate if recommended, otherwise roll back the pre-generation snapshot.
        let (migration, rolled_back) = match &evaluation.recommendation {
            fabric::meta::Recommendation::Adopt => {
                let result = self.meta_runtime.migrate(&candidate).await?;
                tracing::info!(
                    "Migration successful: {} -> {}",
                    result.from_version,
                    result.to_version
                );
                (Some(result), false)
            }
            fabric::meta::Recommendation::PartialAdopt { changes } => {
                tracing::info!("Partial adopt with {} changes — migrating", changes.len());
                let result = self.meta_runtime.migrate(&candidate).await?;
                (Some(result), false)
            }
            other => {
                // Candidate was generated (snapshot saved by generate_candidate); undo it.
                tracing::info!(
                    "Not adopting ({:?}) — rolling back candidate {}",
                    other,
                    candidate.id
                );
                let rolled_back = match self.meta_runtime.rollback().await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("rollback after non-adopt failed: {e}");
                        false
                    }
                };
                (None, rolled_back)
            }
        };

        let success = migration.is_some();
        let message = if success {
            format!(
                "Pipeline complete. Candidate {} adopted with score {:.2}.",
                candidate.id, evaluation.score
            )
        } else {
            format!(
                "Pipeline complete. Candidate {} not adopted. Recommendation: {:?}",
                candidate.id, evaluation.recommendation
            )
        };

        Ok(PipelineResult {
            success,
            candidate: Some(candidate),
            evaluation: Some(evaluation),
            migration,
            message,
            rolled_back,
        })
    }
}

#[derive(Debug)]
pub struct PipelineResult {
    pub success: bool,
    pub candidate: Option<RuntimeCandidate>,
    pub evaluation: Option<Evaluation>,
    pub migration: Option<MigrationResult>,
    pub message: String,
    /// Whether a rollback was performed (candidate was generated but not adopted).
    pub rolled_back: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fabric::genome::*;
    use fabric::meta::Recommendation;
    use fabric::{wall_to_datetime, Clock, Subsystem, SubsystemHealth, TestResult, Version};
    use kernel::chronos::TestClock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    fn genome() -> Genome {
        Genome {
            topology: Topology { subsystems: vec![] },
            identity: IdentitySpec {
                name: "t".into(),
                description: "t".into(),
                self_model: "t".into(),
            },
            boundary: BoundarySpec { rules: vec![] },
            care: CareSpec { priorities: vec![] },
            memory: MemorySpec {
                backends: vec![],
                compaction_strategy: "none".into(),
            },
            mutation: MutationSpec {
                allowed_targets: vec!["care.priorities".into()],
                require_sandbox: false,
                require_self_field_approval: false,
            },
            lifecycle: LifecycleSpec {
                auto_compact: false,
                health_check_interval_secs: 60,
                max_idle_time_secs: 3600,
            },
        }
    }

    struct RejectingMeta {
        rollbacks: Arc<AtomicUsize>,
        clock: Arc<dyn Clock>,
    }

    #[async_trait]
    impl Subsystem for RejectingMeta {
        fn name(&self) -> &str {
            "reject-meta"
        }
        fn version(&self) -> Version {
            Version::new(0, 1, 0)
        }
        async fn init(&mut self, _c: &fabric::SubsystemContext) -> anyhow::Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn health(&self) -> SubsystemHealth {
            SubsystemHealth::Healthy
        }
    }

    #[async_trait]
    impl MetaRuntimeOps for RejectingMeta {
        async fn read_genome(&self) -> anyhow::Result<Genome> {
            Ok(genome())
        }
        async fn generate_candidate(
            &self,
            _i: &MutationIntent,
        ) -> anyhow::Result<RuntimeCandidate> {
            Ok(RuntimeCandidate {
                id: uuid::Uuid::new_v4(),
                genome: genome(),
                changes: vec!["c".into()],
                generated_at: wall_to_datetime(self.clock.wall_now()),
            })
        }
        async fn sandbox_test(&self, _c: &RuntimeCandidate) -> anyhow::Result<TestResult> {
            Ok(TestResult {
                passed: false,
                tests_run: 1,
                tests_passed: 0,
                tests_failed: 1,
                failures: vec!["boom".into()],
                elapsed_ms: 1,
            })
        }
        async fn evaluate(
            &self,
            _c: &RuntimeCandidate,
            _t: &TestResult,
        ) -> anyhow::Result<Evaluation> {
            Ok(Evaluation {
                score: 0.0,
                strengths: vec![],
                weaknesses: vec!["failed".into()],
                recommendation: Recommendation::Reject,
            })
        }
        async fn migrate(&self, _c: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
            panic!("migrate must not be called on a rejected candidate")
        }
        async fn rollback(&self) -> anyhow::Result<()> {
            self.rollbacks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn current_version(&self) -> Version {
            Version::new(0, 1, 0)
        }
    }

    #[tokio::test]
    async fn rejected_candidate_is_rolled_back() {
        let rollbacks = Arc::new(AtomicUsize::new(0));
        let meta = RejectingMeta {
            rollbacks: rollbacks.clone(),
            clock: test_clock(),
        };
        let pipeline = MorphogenesisPipeline::new(meta);
        let intent = MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({ "action": "adjust" }),
            reason: "test".into(),
            reversible: true,
        };
        let result = pipeline.run(&intent).await.unwrap();
        assert!(
            !result.success,
            "rejected candidate must not count as success"
        );
        assert!(result.rolled_back, "rejected candidate must be rolled back");
        assert_eq!(
            rollbacks.load(Ordering::SeqCst),
            1,
            "rollback() must fire exactly once"
        );
    }

    #[tokio::test]
    async fn sandbox_error_rolls_back() {
        use std::sync::atomic::AtomicBool;

        struct SandboxFailingMeta {
            rolled_back: Arc<AtomicBool>,
            clock: Arc<dyn Clock>,
        }

        #[async_trait]
        impl Subsystem for SandboxFailingMeta {
            fn name(&self) -> &str {
                "sandbox-fail"
            }
            fn version(&self) -> Version {
                Version::new(0, 1, 0)
            }
            async fn init(&mut self, _c: &fabric::SubsystemContext) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shutdown(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn health(&self) -> SubsystemHealth {
                SubsystemHealth::Healthy
            }
        }

        #[async_trait]
        impl MetaRuntimeOps for SandboxFailingMeta {
            async fn read_genome(&self) -> anyhow::Result<Genome> {
                Ok(genome())
            }
            async fn generate_candidate(
                &self,
                _i: &MutationIntent,
            ) -> anyhow::Result<RuntimeCandidate> {
                Ok(RuntimeCandidate {
                    id: uuid::Uuid::new_v4(),
                    genome: genome(),
                    changes: vec!["c".into()],
                    generated_at: wall_to_datetime(self.clock.wall_now()),
                })
            }
            async fn sandbox_test(&self, _c: &RuntimeCandidate) -> anyhow::Result<TestResult> {
                anyhow::bail!("sandbox crashed")
            }
            async fn evaluate(
                &self,
                _c: &RuntimeCandidate,
                _t: &TestResult,
            ) -> anyhow::Result<Evaluation> {
                anyhow::bail!("mock(CrashMetaRuntime): evaluate not implemented")
            }
            async fn migrate(&self, _c: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
                anyhow::bail!("mock(CrashMetaRuntime): migrate not implemented")
            }
            async fn rollback(&self) -> anyhow::Result<()> {
                self.rolled_back.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn current_version(&self) -> Version {
                Version::new(0, 1, 0)
            }
        }

        let rolled_back = Arc::new(AtomicBool::new(false));
        let meta = SandboxFailingMeta {
            rolled_back: rolled_back.clone(),
            clock: test_clock(),
        };
        let pipeline = MorphogenesisPipeline::new(meta);
        let intent = MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({}),
            reason: "test".into(),
            reversible: true,
        };
        let result = pipeline.run(&intent).await;
        assert!(result.is_err(), "sandbox crash must error");
        assert!(
            rolled_back.load(Ordering::SeqCst),
            "sandbox crash must trigger rollback"
        );
    }
}
