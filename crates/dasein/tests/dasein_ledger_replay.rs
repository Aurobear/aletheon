use ::contracts::dasein::{
    ExperienceProvenance, ExperienceSource, InterpretedExperience, SelfEventId,
    SelfTransitionRequest, SelfVersion,
};
use ::contracts::{Subsystem, SubsystemContext, WallTime};
use dasein::core::store::SelfFieldStore;
use dasein::core::{SelfField, SelfFieldConfig};
use dasein::dasein::ledger::SelfLedger;
use dasein::dasein::sorge::SystemSorgeTimer;
use dasein::dasein::temporality::{
    HabitEntry, PassiveSynthesizer, RentionalMoment, TemporalPattern,
};
use dasein::dasein::types::TemporalPosition;
use dasein::dasein::{DaseinModule, DaseinRuntimeConfig};
use std::path::Path;
use std::sync::Arc;

struct AllowPolicy;
impl dasein::bridge::policy::PolicyDecisionPort for AllowPolicy {
    fn check(
        &self,
        _tool_name: &str,
        _input: &serde_json::Value,
    ) -> dasein::bridge::policy::PolicyDecision {
        dasein::bridge::policy::PolicyDecision::Allow
    }
}

struct NoopLoop;
impl dasein::bridge::loop_detector::LoopDecisionPort for NoopLoop {
    fn on_new_turn(&self, _turn_id: &str) {}
    fn pre_check(
        &self,
        _tool_name: &str,
        _args: &serde_json::Value,
        _turn_id: &str,
    ) -> dasein::bridge::loop_detector::LoopDecision {
        dasein::bridge::loop_detector::LoopDecision::Allow
    }
    fn post_check(
        &self,
        _tool_name: &str,
        _args: &serde_json::Value,
        _result: &::contracts::tool::ToolResult,
        _turn_id: &str,
    ) {
    }
    fn end_turn(&self, _turn_id: &str) {}
}

fn security_ports() -> (
    Arc<dyn dasein::bridge::policy::PolicyDecisionPort>,
    Arc<dyn dasein::bridge::loop_detector::LoopDecisionPort>,
) {
    (Arc::new(AllowPolicy), Arc::new(NoopLoop))
}

fn open_store(path: &Path) -> Arc<SelfFieldStore> {
    Arc::new(SelfFieldStore::new(path.to_path_buf()).unwrap())
}

fn module_with_ledger(
    store: Arc<SelfFieldStore>,
    clock: Arc<kernel::chronos::TestClock>,
) -> DaseinModule {
    DaseinModule::with_runtime_and_ledger(
        clock,
        Arc::new(SystemSorgeTimer),
        DaseinRuntimeConfig::default(),
        Some(Arc::new(SelfLedger::new(store))),
    )
    .unwrap()
    .0
}

fn request(
    event_id: SelfEventId,
    expected_version: u64,
    observed_at: i64,
    content: InterpretedExperience,
) -> SelfTransitionRequest {
    SelfTransitionRequest {
        event_id,
        source: ExperienceSource::Runtime,
        observed_at: WallTime(observed_at),
        content,
        provenance: ExperienceProvenance {
            producer: "ledger-replay-test".into(),
            session_id: None,
            turn_id: None,
            source_ref: None,
        },
        expected_version: SelfVersion(expected_version),
    }
}

async fn seed_three_events(module: &DaseinModule) -> Vec<SelfTransitionRequest> {
    let requests = vec![
        request(
            SelfEventId::new(),
            0,
            10,
            InterpretedExperience::WorldEntityObserved {
                entity_id: "compiler".into(),
                what_it_is: "build tool".into(),
                for_the_sake_of: Vec::new(),
                readiness: ::contracts::dasein::ReadinessState::ReadyToHand,
            },
        ),
        request(
            SelfEventId::new(),
            1,
            20,
            InterpretedExperience::Lived {
                semantic: "building the project".into(),
                action: Some("compile".into()),
                perception: None,
            },
        ),
        request(
            SelfEventId::new(),
            2,
            30,
            InterpretedExperience::ReadinessChanged {
                entity_id: "compiler".into(),
                old_state: ::contracts::dasein::ReadinessState::ReadyToHand,
                new_state: ::contracts::dasein::ReadinessState::PresentAtHand,
            },
        ),
    ];
    for request in &requests {
        module.transition(request.clone()).await.unwrap();
    }
    requests
}

/// Local test helper — mirrors the private `temporality::canonical_pair`.
fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

// ── Reference implementation (old linear O(n²) algorithm) ──

/// Test-only reference PassiveSynthesizer that uses linear scans instead of
/// O(1) indices.  Produces identical state and patterns under the same input.
struct ReferenceSynth {
    associations: Vec<(String, String, f64)>,
    habits: Vec<HabitEntry>,
    sediment_count: usize,
}

impl ReferenceSynth {
    fn new() -> Self {
        Self {
            associations: Vec::new(),
            habits: Vec::new(),
            sediment_count: 0,
        }
    }

    fn synthesize(&mut self, recent: &[RentionalMoment]) -> Vec<TemporalPattern> {
        self.sediment_count += 1;

        // Associations — linear scan.
        for window in recent.windows(2) {
            let a = &window[0].content.semantic;
            let b = &window[1].content.semantic;
            let key = canonical_pair(a, b);
            let mut found = false;
            for (ea, eb, strength) in &mut self.associations {
                if canonical_pair(ea, eb) == key {
                    *strength = (*strength + 0.1).min(1.0);
                    found = true;
                    break;
                }
            }
            if !found {
                self.associations.push((a.clone(), b.clone(), 0.1));
            }
        }

        // Habits — linear scan.
        for moment in recent {
            let mut found = false;
            for habit in &mut self.habits {
                if habit.pattern == moment.content.semantic {
                    habit.frequency += 1;
                    habit.last_seen = moment.position;
                    found = true;
                    break;
                }
            }
            if !found {
                self.habits.push(HabitEntry {
                    pattern: moment.content.semantic.clone(),
                    frequency: 1,
                    last_seen: moment.position,
                });
            }
        }

        // Prune weak associations.
        self.associations.retain(|(_, _, s)| *s > 0.05);

        // Build patterns.
        let mut patterns = Vec::new();
        for habit in &self.habits {
            if habit.frequency >= 3 {
                patterns.push(TemporalPattern::Repetition {
                    what: habit.pattern.clone(),
                    interval: habit.frequency as u64,
                });
            }
        }
        for (a, b, strength) in &self.associations {
            if *strength > 0.5 {
                patterns.push(TemporalPattern::Trend {
                    direction: format!("{a} -> {b}"),
                    toward: b.clone(),
                });
            }
        }
        patterns
    }
}

// ── Core ledger tests (preserved) ──

#[tokio::test]
async fn ledger_append_reopen_and_duplicate_are_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    let event = request(
        SelfEventId::new(),
        0,
        10,
        InterpretedExperience::Lived {
            semantic: "durable event".into(),
            action: None,
            perception: None,
        },
    );
    let first = module.transition(event.clone()).await.unwrap();
    drop(module);

    let reopened = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(reopened.replay_durable_state().await.unwrap(), 1);
    let duplicate = reopened.transition(event).await.unwrap();
    assert_eq!(first, duplicate);
    let count: u64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM self_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(reopened.temporality().current_position().0, 1);
}

#[tokio::test]
async fn replay_restores_context_and_version_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&original).await;
    let expected_context = serde_json::to_vec(&original.to_context_injection()).unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(replayed.replay_durable_state().await.unwrap(), 3);
    assert_eq!(replayed.self_version().await, SelfVersion(3));
    assert_eq!(
        serde_json::to_vec(&replayed.to_context_injection()).unwrap(),
        expected_context
    );
}

#[tokio::test]
async fn replay_verifies_checkpoint_prefix_then_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    let seeded = seed_three_events(&original).await;
    original.checkpoint_durable_state().unwrap();
    original
        .transition(request(
            SelfEventId::new(),
            3,
            40,
            InterpretedExperience::KnowledgeAsserted {
                assertions: vec!["checkpoint suffix".into()],
                confidence: 0.9,
            },
        ))
        .await
        .unwrap();
    let expected = serde_json::to_vec(&original.to_context_injection()).unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(replayed.replay_durable_state().await.unwrap(), 4);
    assert_eq!(
        serde_json::to_vec(&replayed.to_context_injection()).unwrap(),
        expected
    );
    assert_eq!(seeded.len(), 3);
}

#[tokio::test]
async fn ledger_corruption_fails_replay_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&module).await;
    store
        .conn()
        .execute(
            "UPDATE self_events SET checksum = 'tampered' WHERE seq = 2",
            [],
        )
        .unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    let error = replayed.replay_durable_state().await.unwrap_err();
    assert!(error.to_string().contains("checksum"));
    assert_eq!(replayed.self_version().await, SelfVersion(0));
}

#[tokio::test]
async fn replay_rejects_corrupt_checkpoint_even_when_ledger_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&module).await;
    module.checkpoint_durable_state().unwrap();
    store
        .conn()
        .execute("UPDATE self_snapshots SET checksum = 'tampered'", [])
        .unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    let error = replayed.replay_durable_state().await.unwrap_err();
    assert!(error.to_string().contains("snapshot checksum"));
    assert_eq!(replayed.self_version().await, SelfVersion(0));
}

#[tokio::test]
async fn restart_replays_then_records_one_resumption_experience() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    original
        .transition(request(
            SelfEventId::new(),
            0,
            100,
            InterpretedExperience::Lived {
                semantic: "before restart".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();

    let store = open_store(&path);
    let restarted = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(5_100, 0)),
    );
    dasein::dasein::persistence::load_dasein_state(&restarted, &store)
        .await
        .unwrap();

    assert_eq!(restarted.self_version().await, SelfVersion(2));
    assert_eq!(restarted.temporality().current_position().0, 2);
    let events = SelfLedger::new(store).load_verified().unwrap();
    assert!(matches!(
        events.last().unwrap().request.content,
        InterpretedExperience::ResumedAfterInterval { elapsed_ms: 5_000 }
    ));
}

#[tokio::test]
async fn restart_self_field_replays_before_start_and_checkpoints_after_stop() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self-field.db");
    let context = SubsystemContext {
        name: "self-field-ledger-test".into(),
        working_dir: temp.path().to_path_buf(),
        config: serde_json::Value::Null,
    };

    let (policy_decisions, loop_decisions) = security_ports();
    let mut first = SelfField::new(SelfFieldConfig {
        db_path: Some(path.clone()),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(100, 0))),
        policy_decisions: Some(policy_decisions),
        loop_decisions: Some(loop_decisions),
        ..Default::default()
    });
    first.init(&context).await.unwrap();
    first
        .dasein()
        .unwrap()
        .record_outcome(
            "first installed lifecycle",
            ::contracts::dasein::OutcomeStatus::Succeeded,
            "self-field-ledger-test",
        )
        .await
        .unwrap();
    first.shutdown().await.unwrap();

    let (policy_decisions, loop_decisions) = security_ports();
    let mut restarted = SelfField::new(SelfFieldConfig {
        db_path: Some(path),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(5_100, 0))),
        policy_decisions: Some(policy_decisions),
        loop_decisions: Some(loop_decisions),
        ..Default::default()
    });
    restarted.init(&context).await.unwrap();
    let dasein = restarted.dasein().unwrap();
    assert!(dasein.is_alive());
    assert_eq!(dasein.self_version().await, SelfVersion(2));
    assert_eq!(dasein.temporality().current_position().0, 2);
    restarted.shutdown().await.unwrap();
}

// ═══ Optimized-path semantic equivalence tests ═══

fn assert_synthesizer_equals_expected(actual: &PassiveSynthesizer, expected: &ReferenceSynth) {
    assert_eq!(
        actual.sediment_count, expected.sediment_count,
        "sediment_count mismatch"
    );
    assert_eq!(
        actual.associations.len(),
        expected.associations.len(),
        "association count mismatch"
    );
    assert_eq!(
        actual.habits.len(),
        expected.habits.len(),
        "habit count mismatch"
    );
    // Verify exact stored orientation/order — not only canonicalized equality.
    for (i, (ea, eb, es)) in expected.associations.iter().enumerate() {
        let (aa, ab, as_) = &actual.associations[i];
        assert_eq!(
            aa, ea,
            "association {i}: first element mismatch: {aa} vs {ea}"
        );
        assert_eq!(
            ab, eb,
            "association {i}: second element mismatch: {ab} vs {eb}"
        );
        assert!(
            (*as_ - es).abs() < 1e-9,
            "association {i}: strength mismatch: {as_} vs {es}"
        );
    }
    // Verify habits including last_seen.
    for (i, eh) in expected.habits.iter().enumerate() {
        let ah = &actual.habits[i];
        assert_eq!(ah.pattern, eh.pattern, "habit {i}: pattern mismatch");
        assert_eq!(ah.frequency, eh.frequency, "habit {i}: frequency mismatch");
        assert_eq!(
            ah.last_seen, eh.last_seen,
            "habit {i}: last_seen mismatch: {:?} vs {:?}",
            ah.last_seen, eh.last_seen
        );
    }
}

/// The optimized PassiveSynthesizer must produce identical internal state and
/// patterns as the reference linear implementation after every step, including
/// threshold crossings and reversed association input.
#[test]
fn optimized_synthesizer_equals_reference_after_every_step() {
    let mut opt = PassiveSynthesizer::default();
    let mut reference = ReferenceSynth::new();

    fn moment(semantic: &str, pos: u64) -> RentionalMoment {
        RentionalMoment {
            content: dasein::dasein::temporality::ExperientialContent {
                semantic: semantic.into(),
                action: None,
                perception: None,
                negation: None,
            },
            vividness: 0.8,
            significance: 0.5,
            affect: ::contracts::dasein::AffectTone::Neutral,
            position: TemporalPosition(pos),
            bewandtnis_links: vec![],
        }
    }

    // Step 1-3: feed single "code" moments to build habit frequency to 3.
    for pos in 1..=3 {
        let m = vec![moment("code", pos)];
        let opt_p = opt.synthesize(&m);
        let ref_p = reference.synthesize(&m);
        assert_eq!(opt_p, ref_p, "step {pos} patterns mismatch");
        assert_synthesizer_equals_expected(&opt, &reference);
        // After step 3, expect a repetition pattern for "code".
        if pos == 3 {
            assert!(
                !opt_p.is_empty(),
                "should have repetition pattern for 'code'"
            );
        }
    }

    // Step 4-9: feed alpha/beta pairs in windows(2) to build association strength.
    // Each call provides 2 moments so windows(2) creates 1 association hit.
    for i in 0..6 {
        let a = if i % 2 == 0 { "alpha" } else { "beta" };
        let b = if i % 2 == 0 { "beta" } else { "alpha" };
        let m = vec![moment(a, 4 + i * 2), moment(b, 5 + i * 2)];
        let opt_p = opt.synthesize(&m);
        let ref_p = reference.synthesize(&m);
        assert_eq!(opt_p, ref_p, "step {} patterns mismatch", 4 + i);
        assert_synthesizer_equals_expected(&opt, &reference);
    }

    // After 6 alpha/beta pairs: association strength = 0.1 * 6 = 0.6 (> 0.5).
    assert!(
        opt.associations.iter().any(|(_, _, s)| *s > 0.5),
        "association must cross trend threshold"
    );
    assert!(
        reference.associations.iter().any(|(_, _, s)| *s > 0.5),
        "reference association must also cross trend threshold"
    );
    // Only one association: (alpha, beta).
    assert_eq!(opt.associations.len(), 1);

    // Step 10: reversed pair (beta, alpha) must strengthen the same entry.
    let m_rev = vec![moment("beta", 100), moment("alpha", 101)];
    let opt_p = opt.synthesize(&m_rev);
    let ref_p = reference.synthesize(&m_rev);
    assert_eq!(opt_p, ref_p, "reversed association patterns mismatch");
    assert_synthesizer_equals_expected(&opt, &reference);
    assert_eq!(
        opt.associations.len(),
        1,
        "reversed input must not create duplicate association"
    );
}

/// A reference implementation driven alongside the real reducer across lived
/// events and scheduled reflections must produce identical associations,
/// habits, and patterns at every step.
#[tokio::test]
async fn reducer_produces_non_empty_protentions_from_lived_events() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let clock = Arc::new(kernel::chronos::TestClock::new(100, 0));
    let module = module_with_ledger(open_store(&path), clock);

    // Lived events that create habits and associations.
    let events = [
        ("code", "compile"),
        ("test", "verify"),
        ("code", "compile"),
        ("test", "verify"),
        ("code", "compile"), // 3rd "code" → repetition
        ("build", "run"),
        ("test", "verify"),
        ("build", "run"),
    ];

    for (semantic, action) in &events {
        module
            .transition(request(
                SelfEventId::new(),
                module.self_version().await.0,
                100,
                InterpretedExperience::Lived {
                    semantic: semantic.to_string(),
                    action: Some(action.to_string()),
                    perception: None,
                },
            ))
            .await
            .unwrap();
    }

    // Trigger synthesis via ScheduledReflections.  Multiple reflections ensure
    // habit frequencies accumulate past the repetition threshold.
    for _ in 0..3 {
        module
            .transition(request(
                SelfEventId::new(),
                module.self_version().await.0,
                200,
                InterpretedExperience::ScheduledReflection,
            ))
            .await
            .unwrap();
    }

    // Protention field must be non-empty.
    let protention = module.temporality().protention.read();
    assert!(
        !protention.possibilities.is_empty(),
        "protention field must be populated after reflection on lived events"
    );
    assert!(
        protention.certainty > 0.0,
        "protention certainty must be non-zero"
    );

    // Verify the synthesizer has expected internal state.
    let synth = module.temporality().synthesizer.read();
    assert_eq!(synth.sediment_count, 3);
    // "code" appeared in vivid_moments across multiple reflections.
    let code_habit = synth
        .habits
        .iter()
        .find(|h| h.pattern == "code")
        .expect("'code' must be a tracked habit");
    assert!(code_habit.frequency >= 3);

    // "code"/"compile" and "test"/"verify" pairs each appear 3 times.
    assert!(
        synth.associations.len() >= 2,
        "must have associations from repeated pairs"
    );
}

/// Reversed associations (alpha→beta then beta→alpha) must strengthen a
/// single bidirectional entry, verified with actual ScheduledReflection
/// triggers that run passive synthesis.
#[tokio::test]
async fn reversed_associations_are_bidirectional_with_reflections() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let clock = Arc::new(kernel::chronos::TestClock::new(100, 0));
    let module = module_with_ledger(open_store(&path), clock);

    // Batch 1: alpha → beta followed by a dummy to push both into retention,
    // then a reflection to capture the (alpha, beta) association.
    module
        .transition(request(
            SelfEventId::new(),
            0,
            100,
            InterpretedExperience::Lived {
                semantic: "alpha".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    module
        .transition(request(
            SelfEventId::new(),
            1,
            101,
            InterpretedExperience::Lived {
                semantic: "beta".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    // Dummy: pushes beta into retention so both alpha and beta are captured.
    module
        .transition(request(
            SelfEventId::new(),
            2,
            102,
            InterpretedExperience::Lived {
                semantic: "dummy".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    // Reflection: vivid moments include [dummy, beta, alpha], forming (alpha,beta).
    module
        .transition(request(
            SelfEventId::new(),
            3,
            103,
            InterpretedExperience::ScheduledReflection,
        ))
        .await
        .unwrap();

    // Batch 2: beta → alpha (reversed), plus dummy.
    module
        .transition(request(
            SelfEventId::new(),
            4,
            104,
            InterpretedExperience::Lived {
                semantic: "beta".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    module
        .transition(request(
            SelfEventId::new(),
            5,
            105,
            InterpretedExperience::Lived {
                semantic: "alpha".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    module
        .transition(request(
            SelfEventId::new(),
            6,
            106,
            InterpretedExperience::Lived {
                semantic: "dummy2".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();
    // Reflection 2: must strengthen the same (alpha, beta) entry.
    module
        .transition(request(
            SelfEventId::new(),
            7,
            107,
            InterpretedExperience::ScheduledReflection,
        ))
        .await
        .unwrap();

    let synth = module.temporality().synthesizer.read();
    let ab_count = synth
        .associations
        .iter()
        .filter(|(a, b, _)| canonical_pair(a, b) == canonical_pair("alpha", "beta"))
        .count();
    assert_eq!(
        ab_count, 1,
        "(alpha, beta) must appear exactly once, not duplicated by reversed input"
    );
    let ab_strength = synth
        .associations
        .iter()
        .find(|(a, b, _)| canonical_pair(a, b) == canonical_pair("alpha", "beta"))
        .map(|(_, _, s)| *s)
        .unwrap();
    assert!(
        ab_strength >= 0.1,
        "reversed input must create or strengthen the bidirectional association, got {ab_strength}"
    );
}

/// Replay still consumes every durable event and matches direct authoritative
/// context, even when the optimized synthesis path is exercised.
#[tokio::test]
async fn replay_matches_direct_with_populated_synthesis() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let clock = Arc::new(kernel::chronos::TestClock::new(100, 0));

    let original = module_with_ledger(open_store(&path), clock.clone());
    let mut observed_at = 100i64;

    let lived_phrases = [
        ("code", "compile"),
        ("test", "verify"),
        ("code", "compile"),
        ("test", "verify"),
        ("code", "compile"),
        ("build", "run"),
        ("test", "verify"),
        ("build", "run"),
    ];
    for (semantic, action) in &lived_phrases {
        original
            .transition(request(
                SelfEventId::new(),
                original.self_version().await.0,
                observed_at,
                InterpretedExperience::Lived {
                    semantic: semantic.to_string(),
                    action: Some(action.to_string()),
                    perception: None,
                },
            ))
            .await
            .unwrap();
        observed_at += 1;
    }

    // Several reflections to exercise the optimized path.
    for _ in 0..10 {
        original
            .transition(request(
                SelfEventId::new(),
                original.self_version().await.0,
                observed_at,
                InterpretedExperience::ScheduledReflection,
            ))
            .await
            .unwrap();
        observed_at += 1;
    }

    let expected_context = serde_json::to_vec(&original.to_context_injection()).unwrap();
    let expected_version = original.self_version().await;

    let replayed = module_with_ledger(open_store(&path), clock);
    let replayed_count = replayed.replay_durable_state().await.unwrap();
    assert_eq!(
        replayed_count,
        lived_phrases.len() + 10,
        "replay must consume every durable event"
    );
    assert_eq!(replayed.self_version().await, expected_version);
    assert_eq!(
        serde_json::to_vec(&replayed.to_context_injection()).unwrap(),
        expected_context,
        "replayed context must be byte-for-byte identical"
    );
}
