//! End-to-end integration test for the self-evolution persistence loop.
//!
//! Tests the complete cycle: create layers, mutate state, save to SQLite,
//! load fresh layers from store, verify all state preserved. Then repeat
//! a second cycle to verify accumulated evolution.

use base::self_field::RiskLevel;
use base::{MutationIntent, Verdict};
use dasein::core::attention::AttentionLayer;
use dasein::core::boundary::{BoundaryAction, BoundaryLayer, BoundaryRule};
use dasein::core::care::CareLayer;
use dasein::core::continuity::ContinuityLayer;
use dasein::core::identity::IdentityLayer;
use dasein::core::mutation::MutationLayer;
use dasein::core::narrative::NarrativeLayer;
use dasein::core::store::SelfFieldStore;
use chrono::Duration;
use serde_json::json;
use tempfile::NamedTempFile;

/// Approximate f64 comparison for test assertions.
fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-10
}

/// Helper: create a SelfFieldStore backed by a temporary SQLite file.
fn temp_store() -> (NamedTempFile, SelfFieldStore) {
    let tmp = NamedTempFile::new().expect("create temp file");
    let store = SelfFieldStore::new(tmp.path().to_path_buf()).expect("open store");
    (tmp, store)
}

// ---------------------------------------------------------------------------
// Cycle 1: create, mutate, save, reload, verify
// ---------------------------------------------------------------------------

#[test]
fn cycle1_narrative_roundtrip() {
    let (_tmp, store) = temp_store();

    // -- Phase 1: create and populate --
    let narrative = NarrativeLayer::new(1000);
    narrative.narrate("task_started", "user requested file analysis");
    narrative.narrate("boundary_check", "allowed: no matching rule");
    narrative.narrate("task_completed", "analysis finished");

    narrative.save_to_store(&store).expect("save narrative");

    // -- Phase 2: simulate restart --
    let mut loaded = NarrativeLayer::new(1000);
    loaded.load_from_store(&store).expect("load narrative");

    assert_eq!(loaded.len(), 3);
    let entries = loaded.recent(10);
    assert_eq!(entries[0].event, "task_started");
    assert_eq!(entries[0].reason, "user requested file analysis");
    assert_eq!(entries[1].event, "boundary_check");
    assert_eq!(entries[2].event, "task_completed");
    assert_eq!(entries[2].reason, "analysis finished");
}

#[test]
fn cycle1_attention_roundtrip() {
    let (_tmp, store) = temp_store();

    let attention = AttentionLayer::new(0.0); // no decay for deterministic test
    attention.attend("code_review", 0.9);
    attention.attend("bug_triage", 0.6);

    attention.save_to_store(&store).expect("save attention");

    let mut loaded = AttentionLayer::new(0.0);
    loaded.load_from_store(&store).expect("load attention");

    let topics = loaded.all_topics();
    assert_eq!(topics.len(), 2);
    let highest = loaded.current_focus().expect("has focus");
    assert_eq!(highest.topic, "code_review");
    assert!((highest.priority - 0.9).abs() < f64::EPSILON);

    // Verify both topics present
    let topic_names: Vec<&str> = topics.iter().map(|t| t.topic.as_str()).collect();
    assert!(topic_names.contains(&"code_review"));
    assert!(topic_names.contains(&"bug_triage"));
}

#[test]
fn cycle1_care_weight_adjustment_roundtrip() {
    let (_tmp, store) = temp_store();

    let care = CareLayer::new();
    // Default: safety=1.0, user_intent=0.8, efficiency=0.5, learning=0.3
    assert_eq!(care.weight_of("efficiency"), Some(0.5));

    // Adjust efficiency weight upward
    let result = care.adjust_weight("efficiency", 0.2);
    assert_eq!(result, Some((0.5, 0.7)));
    assert_eq!(care.weight_of("efficiency"), Some(0.7));

    care.save_to_store(&store).expect("save care");

    // Simulate restart
    let mut loaded = CareLayer::new(); // starts with defaults again
    loaded.load_from_store(&store).expect("load care");

    // Verify the adjusted weight persisted
    assert!(approx_eq(loaded.weight_of("efficiency").unwrap(), 0.7));
    // Other defaults should also be present
    assert!(approx_eq(loaded.weight_of("safety").unwrap(), 1.0));
    assert!(approx_eq(loaded.weight_of("user_intent").unwrap(), 0.8));
    assert!(approx_eq(loaded.weight_of("learning").unwrap(), 0.3));
}

#[test]
fn cycle1_boundary_rule_roundtrip() {
    let (_tmp, store) = temp_store();

    let mut boundary = BoundaryLayer::new();
    boundary.add_rule(BoundaryRule {
        action_pattern: "deploy.*".to_string(),
        source_filter: None,
        action: BoundaryAction::Sandbox,
        risk_level: RiskLevel::High,
        description: "sandbox all deploys".to_string(),
        immutable: false,
    });
    boundary.add_rule(BoundaryRule {
        action_pattern: "rm *".to_string(),
        source_filter: None,
        action: BoundaryAction::Deny,
        risk_level: RiskLevel::Critical,
        description: "no rm allowed".to_string(),
        immutable: true,
    });
    assert_eq!(boundary.rule_count(), 2);

    boundary.save_to_store(&store).expect("save boundary");

    // Simulate restart
    let mut loaded = BoundaryLayer::new();
    loaded.load_from_store(&store).expect("load boundary");

    assert_eq!(loaded.rule_count(), 2);

    // Verify deploy rule is Sandbox (mutable)
    let intent_deploy = base::Intent {
        action: "deploy.prod".to_string(),
        parameters: json!({}),
        source: base::IntentSource::User,
        description: "deploy to production".to_string(),
    };
    let verdict = loaded.check(&intent_deploy);
    assert!(matches!(verdict, Some(Verdict::SandboxFirst { .. })));

    // Verify rm rule is Deny (immutable)
    let intent_rm = base::Intent {
        action: "rm -rf /".to_string(),
        parameters: json!({}),
        source: base::IntentSource::User,
        description: "delete everything".to_string(),
    };
    let verdict = loaded.check(&intent_rm);
    assert!(matches!(verdict, Some(Verdict::Deny { .. })));
}

#[test]
fn cycle1_identity_roundtrip() {
    let (_tmp, store) = temp_store();

    let identity = IdentityLayer::new("aletheon", "persistent self-evolving runtime", "0.1.0");

    // Apply a mutation to build history
    identity.mutate(
        None,
        Some("enhanced runtime with persistence".to_string()),
        Some("0.2.0".to_string()),
        "added persistence layer",
    );

    let current = identity.current();
    assert_eq!(current.name, "aletheon");
    assert_eq!(current.version, "0.2.0");
    assert_eq!(identity.mutation_count(), 1);

    identity.save_to_store(&store).expect("save identity");

    // Simulate restart
    let mut loaded = IdentityLayer::new("temp", "temp", "0.0.0");
    loaded.load_from_store(&store).expect("load identity");

    let loaded_current = loaded.current();
    assert_eq!(loaded_current.name, "aletheon");
    assert_eq!(
        loaded_current.description,
        "enhanced runtime with persistence"
    );
    assert_eq!(loaded_current.version, "0.2.0");
    assert!(loaded_current.last_mutation.is_some());
    assert_eq!(loaded.mutation_count(), 1);

    let history = loaded.history();
    assert_eq!(history[0].identity.name, "aletheon");
    assert_eq!(history[0].identity.version, "0.1.0");
    assert_eq!(history[0].reason, "added persistence layer");
}

#[test]
fn cycle1_mutation_records_roundtrip() {
    let (_tmp, store) = temp_store();

    let mutation = MutationLayer::new();

    // Review a reversible mutation (auto-approved)
    let m1 = MutationIntent {
        target: "care_priorities".to_string(),
        change: json!({"efficiency": 0.7}),
        reason: "adjusting efficiency weight".to_string(),
        reversible: true,
    };
    let verdict = mutation.review(&m1);
    assert!(matches!(verdict, Verdict::Allow));

    // Review an irreversible core identity mutation (auto-denied)
    let m2 = MutationIntent {
        target: "name".to_string(),
        change: json!("new_name"),
        reason: "rename attempt".to_string(),
        reversible: false,
    };
    let verdict = mutation.review(&m2);
    assert!(matches!(verdict, Verdict::Deny { .. }));

    assert_eq!(mutation.records().len(), 2);

    mutation.save_to_store(&store).expect("save mutation");

    // Simulate restart
    let mut loaded = MutationLayer::new();
    loaded.load_from_store(&store).expect("load mutation");

    let records = loaded.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].target, "care_priorities");
    assert_eq!(
        records[0].status,
        dasein::core::mutation::MutationStatus::Approved
    );
    assert!(records[0].reversible);
    assert_eq!(records[1].target, "name");
    assert_eq!(
        records[1].status,
        dasein::core::mutation::MutationStatus::Denied
    );
    assert!(!records[1].reversible);
}

#[test]
fn cycle1_continuity_roundtrip() {
    let (_tmp, store) = temp_store();

    let continuity = ContinuityLayer::new(Duration::hours(24));
    continuity.record("aletheon", "0.1.0", "initialized");
    continuity.record("aletheon", "0.1.0", "first_task_completed");

    assert!(continuity.is_continuous());
    assert_eq!(continuity.len(), 2);

    continuity.save_to_store(&store).expect("save continuity");

    // Simulate restart
    let mut loaded = ContinuityLayer::new(Duration::hours(24));
    loaded.load_from_store(&store).expect("load continuity");

    assert_eq!(loaded.len(), 2);
    assert!(loaded.is_continuous());
    let records = loaded.all_records();
    assert_eq!(records[0].identity_name, "aletheon");
    assert_eq!(records[0].event, "initialized");
    assert_eq!(records[1].event, "first_task_completed");
}

// ---------------------------------------------------------------------------
// Full e2e: all layers together, cycle 1 + cycle 2
// ---------------------------------------------------------------------------

#[test]
fn full_e2e_two_cycles() {
    let (_tmp, store) = temp_store();

    // ================================================================
    // CYCLE 1: Create all layers, simulate a task, save everything
    // ================================================================

    // -- Narrative: record task decisions --
    let narrative = NarrativeLayer::new(1000);
    narrative.narrate("task_started", "analyze codebase structure");
    narrative.narrate("review", "allowed: care_score=0.35");

    // -- Attention: focus on code review --
    let attention = AttentionLayer::new(0.0);
    attention.attend("code_review", 0.9);
    attention.attend("refactoring", 0.5);

    // -- Care: adjust efficiency weight --
    let care = CareLayer::new();
    care.adjust_weight("efficiency", 0.2); // 0.5 -> 0.7
    care.adjust_weight("learning", 0.15); // 0.3 -> 0.45

    // -- Boundary: add a rule (Deny, so we can relax it in cycle 2) --
    let mut boundary = BoundaryLayer::new();
    boundary.add_rule(BoundaryRule {
        action_pattern: "deploy.*".to_string(),
        source_filter: None,
        action: BoundaryAction::Deny,
        risk_level: RiskLevel::High,
        description: "deny deploys".to_string(),
        immutable: false,
    });

    // -- Identity: initial state --
    let identity = IdentityLayer::new("aletheon", "persistent runtime", "0.1.0");

    // -- Mutation: record a reversible mutation review --
    let mutation = MutationLayer::new();
    mutation.review(&MutationIntent {
        target: "care_priorities".to_string(),
        change: json!({"efficiency": 0.7}),
        reason: "cycle 1 adjustment".to_string(),
        reversible: true,
    });

    // -- Continuity: record lineage --
    let continuity = ContinuityLayer::new(Duration::hours(24));
    continuity.record("aletheon", "0.1.0", "initialized");
    continuity.record("aletheon", "0.1.0", "cycle_1_complete");

    // -- Save all layers --
    narrative.save_to_store(&store).unwrap();
    attention.save_to_store(&store).unwrap();
    care.save_to_store(&store).unwrap();
    boundary.save_to_store(&store).unwrap();
    identity.save_to_store(&store).unwrap();
    mutation.save_to_store(&store).unwrap();
    continuity.save_to_store(&store).unwrap();

    // ================================================================
    // SIMULATE RESTART: load all layers fresh from store
    // ================================================================

    let mut narrative2 = NarrativeLayer::new(1000);
    let mut attention2 = AttentionLayer::new(0.0);
    let mut care2 = CareLayer::new();
    let mut boundary2 = BoundaryLayer::new();
    let mut identity2 = IdentityLayer::new("temp", "temp", "0.0.0");
    let mut mutation2 = MutationLayer::new();
    let mut continuity2 = ContinuityLayer::new(Duration::hours(24));

    narrative2.load_from_store(&store).unwrap();
    attention2.load_from_store(&store).unwrap();
    care2.load_from_store(&store).unwrap();
    boundary2.load_from_store(&store).unwrap();
    identity2.load_from_store(&store).unwrap();
    mutation2.load_from_store(&store).unwrap();
    continuity2.load_from_store(&store).unwrap();

    // -- Verify cycle 1 state preserved --
    assert_eq!(narrative2.len(), 2);
    assert_eq!(narrative2.recent(10)[0].event, "task_started");

    assert_eq!(attention2.all_topics().len(), 2);
    assert_eq!(attention2.current_focus().unwrap().topic, "code_review");

    assert!(approx_eq(care2.weight_of("efficiency").unwrap(), 0.7));
    assert!(approx_eq(care2.weight_of("learning").unwrap(), 0.45));
    assert!(approx_eq(care2.weight_of("safety").unwrap(), 1.0));

    assert_eq!(boundary2.rule_count(), 1);
    let deploy_intent = base::Intent {
        action: "deploy.prod".to_string(),
        parameters: json!({}),
        source: base::IntentSource::User,
        description: "deploy to production".to_string(),
    };
    assert!(matches!(
        boundary2.check(&deploy_intent),
        Some(Verdict::Deny { .. })
    ));

    assert_eq!(identity2.current().name, "aletheon");
    assert_eq!(identity2.current().version, "0.1.0");

    assert_eq!(mutation2.records().len(), 1);
    assert_eq!(mutation2.records()[0].target, "care_priorities");
    assert_eq!(
        mutation2.records()[0].status,
        dasein::core::mutation::MutationStatus::Approved
    );

    assert_eq!(continuity2.len(), 2);
    assert_eq!(continuity2.all_records()[1].event, "cycle_1_complete");

    // ================================================================
    // CYCLE 2: evolve further on top of loaded state
    // ================================================================

    // -- Add more narrative entries --
    narrative2.narrate("cycle_2_start", "continuing evolution");
    narrative2.narrate("care_adjusted", "learning weight increased");

    // -- Add new attention topic --
    attention2.attend("evolution_tracking", 0.7);

    // -- Adjust care weights again --
    care2.adjust_weight("learning", 0.1); // 0.45 -> 0.55
    care2.adjust_weight("safety", -0.1); // 1.0 -> 0.9 (safety floor = 0.8)

    // -- Relax the deploy boundary rule --
    assert!(boundary2.relax_rule("deploy.*"));
    // It should now be Sandbox (already was, but let's also add a new rule)
    boundary2.add_rule(BoundaryRule {
        action_pattern: "delete_*".to_string(),
        source_filter: None,
        action: BoundaryAction::RequireConfirmation,
        risk_level: RiskLevel::Medium,
        description: "confirm deletes".to_string(),
        immutable: false,
    });

    // -- Mutate identity --
    identity2.mutate(
        None,
        Some("evolved persistent runtime".to_string()),
        Some("0.2.0".to_string()),
        "cycle 2 evolution",
    );

    // -- Record another mutation review --
    mutation2.review(&MutationIntent {
        target: "boundary_rules".to_string(),
        change: json!({"add": "delete_*"}),
        reason: "cycle 2 boundary addition".to_string(),
        reversible: true,
    });

    // -- Extend continuity --
    continuity2.record("aletheon", "0.2.0", "cycle_2_complete");

    // -- Save everything again --
    narrative2.save_to_store(&store).unwrap();
    attention2.save_to_store(&store).unwrap();
    care2.save_to_store(&store).unwrap();
    boundary2.save_to_store(&store).unwrap();
    identity2.save_to_store(&store).unwrap();
    mutation2.save_to_store(&store).unwrap();
    continuity2.save_to_store(&store).unwrap();

    // ================================================================
    // SIMULATE SECOND RESTART: verify accumulated evolution
    // ================================================================

    let mut narrative3 = NarrativeLayer::new(1000);
    let mut attention3 = AttentionLayer::new(0.0);
    let mut care3 = CareLayer::new();
    let mut boundary3 = BoundaryLayer::new();
    let mut identity3 = IdentityLayer::new("temp", "temp", "0.0.0");
    let mut mutation3 = MutationLayer::new();
    let mut continuity3 = ContinuityLayer::new(Duration::hours(24));

    narrative3.load_from_store(&store).unwrap();
    attention3.load_from_store(&store).unwrap();
    care3.load_from_store(&store).unwrap();
    boundary3.load_from_store(&store).unwrap();
    identity3.load_from_store(&store).unwrap();
    mutation3.load_from_store(&store).unwrap();
    continuity3.load_from_store(&store).unwrap();

    // -- Narrative: should have cycle 1 + cycle 2 entries --
    assert_eq!(narrative3.len(), 4);
    let entries = narrative3.recent(10);
    assert_eq!(entries[0].event, "task_started"); // cycle 1
    assert_eq!(entries[1].event, "review"); // cycle 1
    assert_eq!(entries[2].event, "cycle_2_start"); // cycle 2
    assert_eq!(entries[3].event, "care_adjusted"); // cycle 2

    // -- Attention: should have all 3 topics --
    assert_eq!(attention3.all_topics().len(), 3);
    let topic_names: Vec<String> = attention3
        .all_topics()
        .iter()
        .map(|t| t.topic.clone())
        .collect();
    assert!(topic_names.contains(&"code_review".to_string()));
    assert!(topic_names.contains(&"refactoring".to_string()));
    assert!(topic_names.contains(&"evolution_tracking".to_string()));

    // -- Care: accumulated adjustments --
    assert!(approx_eq(care3.weight_of("efficiency").unwrap(), 0.7)); // unchanged from cycle 1
    assert!(approx_eq(care3.weight_of("learning").unwrap(), 0.55)); // 0.45 + 0.1
    assert!(approx_eq(care3.weight_of("safety").unwrap(), 0.9)); // 1.0 - 0.1
    assert!(approx_eq(care3.weight_of("user_intent").unwrap(), 0.8)); // default, never touched

    // -- Boundary: 2 rules now --
    assert_eq!(boundary3.rule_count(), 2);

    // deploy rule should now be Sandbox (relaxed from Deny)
    assert!(matches!(
        boundary3.check(&deploy_intent),
        Some(Verdict::SandboxFirst { .. })
    ));

    // delete rule should require confirmation
    let delete_intent = base::Intent {
        action: "delete_user_data".to_string(),
        parameters: json!({}),
        source: base::IntentSource::User,
        description: "delete user data".to_string(),
    };
    assert!(matches!(
        boundary3.check(&delete_intent),
        Some(Verdict::RequireConfirmation { .. })
    ));

    // -- Identity: evolved --
    assert_eq!(identity3.current().name, "aletheon");
    assert_eq!(identity3.current().version, "0.2.0");
    assert_eq!(
        identity3.current().description,
        "evolved persistent runtime"
    );
    assert_eq!(identity3.mutation_count(), 1);
    let history = identity3.history();
    assert_eq!(history[0].identity.version, "0.1.0");
    assert_eq!(history[0].reason, "cycle 2 evolution");

    // -- Mutation: 2 records --
    assert_eq!(mutation3.records().len(), 2);
    assert_eq!(mutation3.records()[0].target, "care_priorities");
    assert_eq!(
        mutation3.records()[0].status,
        dasein::core::mutation::MutationStatus::Approved
    );
    assert_eq!(mutation3.records()[1].target, "boundary_rules");
    assert_eq!(
        mutation3.records()[1].status,
        dasein::core::mutation::MutationStatus::Approved
    );

    // -- Continuity: 3 records --
    assert_eq!(continuity3.len(), 3);
    let cont_records = continuity3.all_records();
    assert_eq!(cont_records[0].event, "initialized");
    assert_eq!(cont_records[1].event, "cycle_1_complete");
    assert_eq!(cont_records[2].event, "cycle_2_complete");
    assert_eq!(cont_records[2].identity_version, "0.2.0");
}
