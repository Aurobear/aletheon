//! Integration tests for governed EvolutionCoordinator verification.
//!
//! Verifies the full flow from turn metrics through reflection accumulation
//! to Metacog verification without bypassing governed apply.
//!
//! Tests:
//! 1. Failure-triggered evolution
//! 2. Periodic trigger (every N turns)
//! 3. Sliding window eviction

use anyhow::Result;
use async_trait::async_trait;
use executive::core::evolution_coordinator::{EvolutionConfig, EvolutionCoordinator};
use fabric::genome::*;
use fabric::meta::{
    Evaluation, MetaRuntimeOps, MigrationResult, Recommendation, RuntimeCandidate, TestResult,
};
use fabric::MutationIntent;
use fabric::{Subsystem, SubsystemHealth, Version};
use kernel::chronos::TestClock;
use metacog::DefaultMetacogService;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal genome for mock use.
fn minimal_genome() -> Genome {
    Genome {
        topology: Topology { subsystems: vec![] },
        identity: IdentitySpec {
            name: "test".to_string(),
            description: "test agent".to_string(),
            self_model: "test".to_string(),
        },
        boundary: BoundarySpec { rules: vec![] },
        care: CareSpec { priorities: vec![] },
        memory: MemorySpec {
            backends: vec![],
            compaction_strategy: "none".to_string(),
        },
        mutation: MutationSpec {
            allowed_targets: vec!["care.priorities".to_string(), "mutation.config".to_string()],
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

// ---------------------------------------------------------------------------
// Mock MetaRuntimeOps — always adopts, tracks calls
// ---------------------------------------------------------------------------

struct MockMetaRuntime {
    generate_calls: Arc<AtomicUsize>,
    migrate_calls: Arc<AtomicUsize>,
}

impl MockMetaRuntime {
    fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let generate_calls = Arc::new(AtomicUsize::new(0));
        let migrate_calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                generate_calls: generate_calls.clone(),
                migrate_calls: migrate_calls.clone(),
            },
            generate_calls,
            migrate_calls,
        )
    }
}

#[async_trait]
impl Subsystem for MockMetaRuntime {
    fn name(&self) -> &str {
        "mock-meta"
    }
    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
    async fn init(&mut self, _ctx: &fabric::SubsystemContext) -> Result<()> {
        Ok(())
    }
    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
    async fn health(&self) -> SubsystemHealth {
        SubsystemHealth::Healthy
    }
}

#[async_trait]
impl MetaRuntimeOps for MockMetaRuntime {
    async fn read_genome(&self) -> Result<Genome> {
        Ok(minimal_genome())
    }

    async fn generate_candidate(&self, _intent: &MutationIntent) -> Result<RuntimeCandidate> {
        self.generate_calls.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeCandidate {
            id: uuid::Uuid::new_v4(),
            genome: minimal_genome(),
            changes: vec!["mock change".to_string()],
            generated_at: chrono::Utc::now(),
        })
    }

    async fn sandbox_test(&self, _candidate: &RuntimeCandidate) -> Result<TestResult> {
        Ok(TestResult {
            passed: true,
            tests_run: 1,
            tests_passed: 1,
            tests_failed: 0,
            failures: vec![],
            elapsed_ms: 10,
        })
    }

    async fn evaluate(
        &self,
        _candidate: &RuntimeCandidate,
        _test: &TestResult,
    ) -> Result<Evaluation> {
        Ok(Evaluation {
            score: 0.9,
            strengths: vec!["mock".to_string()],
            weaknesses: vec![],
            recommendation: Recommendation::Adopt,
        })
    }

    async fn migrate(&self, _candidate: &RuntimeCandidate) -> Result<MigrationResult> {
        self.migrate_calls.fetch_add(1, Ordering::SeqCst);
        Ok(MigrationResult {
            success: true,
            from_version: "0.1.0".to_string(),
            to_version: "0.1.1".to_string(),
            memories_migrated: 0,
            identity_preserved: true,
            message: "mock migration".to_string(),
        })
    }

    async fn rollback(&self) -> Result<()> {
        Ok(())
    }

    fn current_version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

// ---------------------------------------------------------------------------
// Test: Failure-triggered evolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failure_triggers_evolution() {
    let tmp = tempfile::tempdir().unwrap();
    let config = EvolutionConfig {
        enabled: true,
        evolution_permitted: true,
        trigger_every_n_turns: 0, // disable periodic
        trigger_on_failure: true,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    };
    let coordinator = EvolutionCoordinator::new(config, Arc::new(TestClock::default())).unwrap();
    let (mock, gen_calls, mig_calls) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));

    // Simulate 3 failure turns — each should trigger evolution
    for i in 0..3 {
        let summary = coordinator
            .post_turn(
                &format!("task {i}"),
                "error output",
                false, // failure
                5,     // tool_calls
                2,     // tool_errors
                1000,  // elapsed_ms
                1,     // iterations
                &service,
                vec![], // awareness_signals
            )
            .await
            .unwrap();

        assert!(summary.reflected, "turn {i} should reflect");
        assert!(
            summary.evolution_triggered,
            "turn {i} should trigger evolution on failure"
        );
    }

    assert_eq!(coordinator.turn_count().await, 3);
    // Each failure turn generates and verifies intents.
    assert!(
        gen_calls.load(Ordering::SeqCst) >= 3,
        "generate_candidate called at least 3 times"
    );
    assert_eq!(
        mig_calls.load(Ordering::SeqCst),
        0,
        "verification cannot bypass governed apply"
    );
    // Reflections should accumulate
    assert_eq!(coordinator.recent_reflections().await.len(), 3);
}

// ---------------------------------------------------------------------------
// Test: Periodic trigger
// ---------------------------------------------------------------------------

#[tokio::test]
async fn periodic_trigger_at_n_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let config = EvolutionConfig {
        enabled: true,
        evolution_permitted: true,
        trigger_every_n_turns: 3, // trigger every 3rd turn
        trigger_on_failure: false,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    };
    let coordinator = EvolutionCoordinator::new(config, Arc::new(TestClock::default())).unwrap();
    let (mock, gen_calls, mig_calls) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));

    let mut trigger_turns = Vec::new();

    // Simulate 5 successful turns
    for i in 1..=5 {
        let summary = coordinator
            .post_turn(
                &format!("task {i}"),
                "ok output",
                true, // success
                3,    // tool_calls
                0,    // tool_errors
                500,  // elapsed_ms
                1,    // iterations
                &service,
                vec![], // awareness_signals
            )
            .await
            .unwrap();

        assert!(summary.reflected);
        if summary.evolution_triggered {
            trigger_turns.push(i);
        }
    }

    assert_eq!(coordinator.turn_count().await, 5);
    // Evolution should trigger on turns 3 only (not on 1, 2, 4, 5)
    assert_eq!(trigger_turns, vec![3], "should trigger only on turn 3");
    assert_eq!(
        gen_calls.load(Ordering::SeqCst),
        1,
        "generate_candidate called once"
    );
    assert_eq!(
        mig_calls.load(Ordering::SeqCst),
        0,
        "verification cannot migrate without governed evidence"
    );
}

// ---------------------------------------------------------------------------
// Test: Sliding window eviction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sliding_window_eviction() {
    let tmp = tempfile::tempdir().unwrap();
    let config = EvolutionConfig {
        enabled: true,
        evolution_permitted: true,
        trigger_every_n_turns: 0, // disable periodic
        trigger_on_failure: false,
        window_size: 5,
        lineage_dir: tmp.path().to_path_buf(),
    };
    let coordinator = EvolutionCoordinator::new(config, Arc::new(TestClock::default())).unwrap();
    let (mock, _gen, _mig) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));

    // Simulate 10 successful turns — no evolution triggers
    for i in 1..=10 {
        let summary = coordinator
            .post_turn(
                &format!("task {i}"),
                "ok",
                true, // success
                1,    // tool_calls
                0,    // tool_errors
                100,  // elapsed_ms
                1,    // iterations
                &service,
                vec![], // awareness_signals
            )
            .await
            .unwrap();

        assert!(summary.reflected);
        assert!(!summary.evolution_triggered, "no evolution should trigger");
    }

    assert_eq!(coordinator.turn_count().await, 10);

    // Only the last 5 reflections should remain
    let reflections = coordinator.recent_reflections().await;
    assert_eq!(
        reflections.len(),
        5,
        "window should keep only last 5 reflections"
    );

    // The remaining reflections should be for tasks 6-10
    for (i, entry) in reflections.iter().enumerate() {
        let expected_task = format!("task {}", i + 6);
        assert_eq!(
            entry.task_summary, expected_task,
            "reflection {} should be '{}', got '{}'",
            i, expected_task, entry.task_summary
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Disabled coordinator never touches the pipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disabled_coordinator_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let config = EvolutionConfig {
        enabled: false, // default-off gate
        evolution_permitted: false,
        trigger_every_n_turns: 1, // would trigger every turn if enabled
        trigger_on_failure: true,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    };
    let coordinator = EvolutionCoordinator::new(config, Arc::new(TestClock::default())).unwrap();
    let (mock, gen_calls, mig_calls) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));

    let summary = coordinator
        .post_turn(
            "task",
            "error output",
            false,
            5,
            2,
            1000,
            1,
            &service,
            vec![],
        )
        .await
        .unwrap();

    assert!(
        !summary.evolution_triggered,
        "disabled loop must not trigger"
    );
    assert_eq!(
        gen_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no candidate generated"
    );
    assert_eq!(
        mig_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no migration"
    );
    assert!(!summary.reflected, "disabled loop skips reflection too");
}

#[tokio::test]
async fn mood_fallback_intents_use_governed_verification_without_apply() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator = EvolutionCoordinator::new(
        EvolutionConfig {
            enabled: true,
            evolution_permitted: true,
            trigger_every_n_turns: 0,
            trigger_on_failure: false,
            window_size: 20,
            lineage_dir: tmp.path().to_path_buf(),
        },
        Arc::new(TestClock::default()),
    )
    .unwrap();
    let (mock, generate_calls, migrate_calls) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));
    let intents = vec![MutationIntent {
        target: "care.priorities".into(),
        change: serde_json::json!({"action": "adjust", "magnitude": 0.1}),
        reason: "Dasein Angst fallback".into(),
        reversible: true,
    }];

    let (triggered, receipts) = coordinator
        .run_mood_fallback(&intents, &service)
        .await
        .unwrap();

    assert!(triggered);
    assert_eq!(receipts.len(), 1);
    assert_eq!(generate_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        migrate_calls.load(Ordering::SeqCst),
        0,
        "fallback must not bypass governed apply"
    );
}

#[tokio::test]
async fn mood_fallback_respects_default_off_operator_gates() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator = EvolutionCoordinator::new(
        EvolutionConfig {
            enabled: false,
            evolution_permitted: false,
            trigger_every_n_turns: 0,
            trigger_on_failure: false,
            window_size: 20,
            lineage_dir: tmp.path().to_path_buf(),
        },
        Arc::new(TestClock::default()),
    )
    .unwrap();
    let (mock, generate_calls, migrate_calls) = MockMetaRuntime::new();
    let service = DefaultMetacogService::in_memory(Arc::new(mock), Arc::new(TestClock::default()));
    let intents = vec![MutationIntent {
        target: "care.priorities".into(),
        change: serde_json::json!({"action": "adjust", "magnitude": 0.1}),
        reason: "Dasein Angst fallback".into(),
        reversible: true,
    }];

    let (triggered, receipts) = coordinator
        .run_mood_fallback(&intents, &service)
        .await
        .unwrap();

    assert!(!triggered);
    assert!(receipts.is_empty());
    assert_eq!(generate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(migrate_calls.load(Ordering::SeqCst), 0);
}
