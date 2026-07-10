//! End-to-end tests for the self-awareness seed mechanism.
//!
//! Tests the full flow:
//! 1. BrainCore generates awareness during reasoning
//! 2. Awareness is stored in Episodic Memory
//! 3. AwarenessGrowthAnalyzer produces suggestions
//! 4. Suggestions are fed back to BrainCore

use dasein::core::awareness_growth::AwarenessGrowthAnalyzer;
use fabric::{AwarenessExtension, AwarenessExtensionCounts, SelfAwareness, SelfState};

#[test]
fn test_full_awareness_flow() {
    // Step 1: Generate awareness (simulating BrainCore)
    let awareness = SelfAwareness::with_extensions(
        "answering user question",
        vec![
            AwarenessExtension::Intent {
                reason: "help user understand".into(),
            },
            AwarenessExtension::SelfState {
                state: SelfState::Focused,
            },
        ],
    );

    // Step 2: Verify awareness structure
    assert_eq!(awareness.core.action, "answering user question");
    assert!(awareness.core.aware);
    assert_eq!(awareness.extensions.len(), 2);

    // Step 3: Assess quality (simulating analysis)
    let analyzer = AwarenessGrowthAnalyzer::new();
    let quality = analyzer.assess_quality(&awareness);
    assert!(quality > 0.5); // Should be decent quality

    // Step 4: Verify extension counts
    let counts = awareness.extension_counts();
    assert_eq!(counts.intent, 1);
    assert_eq!(counts.self_state, 1);
    assert_eq!(counts.significance, 0);
    assert_eq!(counts.reflexive, 0);
}

#[test]
fn test_growth_suggestion_flow() {
    // Simulate awareness history with missing significance
    let history: Vec<SelfAwareness> = (0..20)
        .map(|i| {
            SelfAwareness::with_extensions(
                format!("action {}", i),
                vec![AwarenessExtension::Intent {
                    reason: "reason".into(),
                }],
            )
        })
        .collect();

    let stats = AwarenessExtensionCounts {
        intent: 20,
        self_state: 0,
        significance: 0, // Missing!
        reflexive: 0,
    };

    // Analyze for growth suggestions
    let analyzer = AwarenessGrowthAnalyzer::with_config(10, 0.3);
    let suggestions = analyzer.analyze(&history, &stats).unwrap();

    // Should suggest significance and self_state
    assert!(suggestions
        .iter()
        .any(|s| s.extension_type == "significance"));
    assert!(suggestions.iter().any(|s| s.extension_type == "self_state"));
}

#[test]
fn test_minimal_awareness_is_valid() {
    // The minimal awareness (seed) should always be valid
    let awareness = SelfAwareness::minimal("doing something");

    assert_eq!(awareness.core.action, "doing something");
    assert!(awareness.core.aware); // Always true
    assert!(awareness.extensions.is_empty()); // No extensions yet

    // Quality should still be positive (core = 0.3)
    let analyzer = AwarenessGrowthAnalyzer::new();
    let quality = analyzer.assess_quality(&awareness);
    assert!(quality >= 0.3);
}

#[test]
fn test_awareness_serialization() {
    // Awareness should survive serialization round-trip
    let awareness = SelfAwareness::with_extensions(
        "test action",
        vec![
            AwarenessExtension::Intent {
                reason: "test reason".into(),
            },
            AwarenessExtension::SelfState {
                state: SelfState::Curious,
            },
        ],
    );

    let bytes = awareness.to_json_bytes();
    let restored = SelfAwareness::from_json_bytes(&bytes).unwrap();

    assert_eq!(restored.core.action, awareness.core.action);
    assert_eq!(restored.core.aware, awareness.core.aware);
    assert_eq!(restored.extensions.len(), awareness.extensions.len());
}
