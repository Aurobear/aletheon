//! Pure contracts for conscious arbitration: field readout, batch planning,
//! and typed decisions. These types live in Fabric so Dasein, Cognit, and
//! Executive can share them without reversing crate dependency direction.
//!
//! Philosophy grounding:
//! - The readout translates an R1 broadcast into a bounded, decision-ready field
//!   projection that belongs to neither Dasein nor Executive.
//! - Batch plans are exact permutations — no missing, duplicate, or injected IDs.
//!   Cognit validates then applies; every other consumer sees the validated result.
//! - Decisions are typed at the Fabric layer so safety rules (authorization-first)
//!   can be enforced without runtime introspection.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::dasein::CareActionKind;
use crate::{AgoraSpaceId, BroadcastEpoch, ConsciousContextProjection, SalienceVector};

// ---------------------------------------------------------------------------
// Context read port (moved from Executive → Fabric)
// ---------------------------------------------------------------------------

/// Read the latest completed conscious context for a given workspace.
///
/// This is the *only* consciousness signal available to non-Executive crates.
/// Dasein consumes it for R2 field feedback; Cognit and batch planners use its
/// projection indirectly through the workspace-backed implementation.
#[async_trait]
pub trait LatestConsciousContextPort: Send + Sync {
    /// Return the latest completed projection for `space`, or an error.
    ///
    /// An absent workspace or an empty broadcast is represented as a valid
    /// `Ok(...)` projection whose `latest_broadcast` is `None` — it is NOT
    /// returned as an `Err`.  Callers MUST treat `Err` the same as `None`.
    async fn latest_context(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection>;
}

// ---------------------------------------------------------------------------
// Arbitration mode
// ---------------------------------------------------------------------------

/// How the conscious field affects turn execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsciousArbitrationMode {
    /// Compute and trace decisions but preserve legacy behavior.
    /// This is the safe production default.
    #[default]
    Observe,
    /// Apply reorder and enforce soft-defer decisions.
    /// This mode requires explicit operator confirmation.
    Enforce,
}

impl ConsciousArbitrationMode {
    pub fn parse_env(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "observe" => Some(Self::Observe),
            "enforce" => Some(Self::Enforce),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-call decision types
// ---------------------------------------------------------------------------

/// What the arbitrator decided about a single capability call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDecisionKind {
    /// Execute normally.
    Proceed,
    /// Reorder within the batch; still execute.
    Reorder,
    /// Would have deferred in Enforce mode; execute normally in Observe.
    WouldDefer,
    /// Do not execute this call; return a structured deferred result.
    Defer,
}

/// Why the arbitrator made its decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDecisionReason {
    /// No usable broadcast — empty context fallback.
    FieldAbsent,
    /// The action was selected by Agora competition.
    Selected,
    /// Dasein care structure decided to Negate.
    Negated,
    /// The action was submitted but lost the Agora competition.
    LostCompetition,
}

// ---------------------------------------------------------------------------
// Deterministic field readout
// ---------------------------------------------------------------------------

/// A bounded, decision-ready snapshot of the latest conscious field.
///
/// Derived from a [`ConsciousContextProjection`] using only selected broadcast
/// records.  The precision formula follows `design.md:104-135`:
///
/// ```text
/// u = max selected CareConcern urgency, or 0
/// s = max selected candidate (urgency, self_relevance), or 0
/// a = selected CareDecision weight, or 0
/// p = clamp(max(u, s, a), 0, 1)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsciousFieldReadout {
    /// Broadcast epoch this readout is based on.
    pub epoch: BroadcastEpoch,
    /// The Dasein care decision, if one was selected in this epoch.
    pub care_action: Option<CareActionKind>,
    /// Maximum selected `CareConcernFrame.urgency`, or 0.
    pub concern_urgency: f32,
    /// Salience of the most-salient selected candidate.
    pub salience: SalienceVector,
    /// Bounded field precision (clamped to [0, 1]).
    pub precision: f32,
}

impl ConsciousFieldReadout {
    // CareActionKind weights (design.md §5.2).
    fn care_action_weight(action: CareActionKind) -> f32 {
        match action {
            CareActionKind::Direct => 0.25,
            CareActionKind::Deliberate => 0.60,
            CareActionKind::Wait => 0.75,
            CareActionKind::Negate => 1.00,
        }
    }

    /// Derive a readout from a projection, or return `None` for empty context.
    ///
    /// # Errors
    ///
    /// Returns `Err` only when the projection fails structural validation.
    /// An empty broadcast (no winners) returns `Ok(None)` — this is the
    /// normal empty/degraded case that callers treat as baseline.
    pub fn from_projection(
        projection: &ConsciousContextProjection,
    ) -> anyhow::Result<Option<Self>> {
        projection.validate()?;

        let broadcast = match &projection.latest_broadcast {
            Some(b) => b,
            None => return Ok(None),
        };

        if broadcast.winner_ids.is_empty() {
            return Ok(None);
        }

        // u = max selected CareConcern urgency
        let u: f32 = broadcast
            .selected
            .iter()
            .filter_map(|w| match &w.content {
                crate::WorkspaceContent::CareConcern(c) => Some(c.urgency),
                _ => None,
            })
            .fold(0.0_f32, f32::max);

        // s = max selected candidate (urgency, self_relevance)
        let s: f32 = broadcast
            .selected
            .iter()
            .map(|w| w.salience.urgency.max(w.salience.self_relevance))
            .fold(0.0_f32, f32::max);

        // a = selected CareDecision weight, or 0
        let a: f32 = broadcast
            .selected
            .iter()
            .filter_map(|w| match &w.content {
                crate::WorkspaceContent::Concern(crate::dasein::SelfSignal::CareDecision {
                    action,
                    ..
                }) => Some(Self::care_action_weight(action.clone())),
                _ => None,
            })
            .fold(0.0_f32, f32::max);

        // Select the most conservative care action deterministically. A later
        // Negate must never be hidden by an earlier Direct in the coalition.
        let care_action: Option<CareActionKind> = broadcast
            .selected
            .iter()
            .filter_map(|w| match &w.content {
                crate::WorkspaceContent::Concern(crate::dasein::SelfSignal::CareDecision {
                    action,
                    ..
                }) => Some(action.clone()),
                _ => None,
            })
            .max_by(|left, right| {
                Self::care_action_weight(left.clone())
                    .total_cmp(&Self::care_action_weight(right.clone()))
            });

        // Each dimension is the bounded maximum across the selected
        // coalition. Copying one candidate's whole vector would discard strong
        // signals carried by a different winner.
        let salience = broadcast.selected.iter().fold(
            SalienceVector {
                urgency: 0.0,
                goal_relevance: 0.0,
                self_relevance: 0.0,
                novelty: 0.0,
                confidence: 0.0,
                prediction_error: 0.0,
                affect_intensity: 0.0,
                social_relevance: 0.0,
            },
            |maxima, candidate| SalienceVector {
                urgency: maxima.urgency.max(candidate.salience.urgency),
                goal_relevance: maxima.goal_relevance.max(candidate.salience.goal_relevance),
                self_relevance: maxima.self_relevance.max(candidate.salience.self_relevance),
                novelty: maxima.novelty.max(candidate.salience.novelty),
                confidence: maxima.confidence.max(candidate.salience.confidence),
                prediction_error: maxima
                    .prediction_error
                    .max(candidate.salience.prediction_error),
                affect_intensity: maxima
                    .affect_intensity
                    .max(candidate.salience.affect_intensity),
                social_relevance: maxima
                    .social_relevance
                    .max(candidate.salience.social_relevance),
            },
        );

        let p = u.max(s).max(a).clamp(0.0, 1.0);

        Ok(Some(Self {
            epoch: broadcast.epoch,
            care_action,
            concern_urgency: u,
            salience,
            precision: p,
        }))
    }
}

// ---------------------------------------------------------------------------
// Batch plan types
// ---------------------------------------------------------------------------

/// One call's decision within a batch plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBatchDecision {
    pub call_id: String,
    pub decision: FieldDecisionKind,
    pub reason: FieldDecisionReason,
    /// Priority used for stable sort (higher = earlier in the batch).
    pub priority: f32,
    /// Which conscious broadcast epoch informed this decision.
    pub broadcast_epoch: Option<BroadcastEpoch>,
}

/// A validated batch execution plan produced by the conscious arbitrator.
///
/// Plans are **exact permutations**: every `call_id` from the input batch
/// must appear exactly once in `ordered_call_ids`.  Cognit validates this
/// property before applying the order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBatchPlan {
    /// The arbitration mode that produced this plan.
    pub mode: ConsciousArbitrationMode,
    /// Stable order of call IDs (exact permutation of the input).
    pub ordered_call_ids: Vec<String>,
    /// Per-call decisions (one entry per call).
    pub decisions: Vec<CapabilityBatchDecision>,
}

impl CapabilityBatchPlan {
    /// Return an identity plan that preserves the input order exactly.
    ///
    /// Every call is `Proceed` with `FieldAbsent` reason, matching the
    /// no-conscious-core baseline.
    pub fn identity(calls: &[crate::CapabilityCall]) -> Self {
        let ordered_call_ids: Vec<String> = calls.iter().map(|c| c.call_id.clone()).collect();
        let decisions = calls
            .iter()
            .map(|c| CapabilityBatchDecision {
                call_id: c.call_id.clone(),
                decision: FieldDecisionKind::Proceed,
                reason: FieldDecisionReason::FieldAbsent,
                priority: 0.0,
                broadcast_epoch: None,
            })
            .collect();
        Self {
            mode: ConsciousArbitrationMode::Observe,
            ordered_call_ids,
            decisions,
        }
    }

    /// Validate that `ordered_call_ids` is an exact permutation of `calls`.
    ///
    /// Returns `Err` if any call ID is missing, duplicated, or unknown.
    pub fn validate_against(&self, calls: &[crate::CapabilityCall]) -> anyhow::Result<()> {
        let mut expected: Vec<String> = calls.iter().map(|c| c.call_id.clone()).collect();
        expected.sort();
        anyhow::ensure!(
            expected.windows(2).all(|pair| pair[0] != pair[1]),
            "input batch contains duplicate call IDs"
        );

        let mut actual = self.ordered_call_ids.clone();
        actual.sort();

        anyhow::ensure!(
            expected == actual,
            "batch plan is not an exact permutation: expected {expected:?}, got {actual:?}",
        );

        // Decisions are also an exact permutation: precisely one bounded
        // decision for every unique input call, with no duplicate or omission.
        let mut expected_decisions: Vec<&str> = calls.iter().map(|c| c.call_id.as_str()).collect();
        expected_decisions.sort_unstable();
        let mut actual_decisions: Vec<&str> = self
            .decisions
            .iter()
            .map(|decision| decision.call_id.as_str())
            .collect();
        actual_decisions.sort_unstable();
        anyhow::ensure!(
            expected_decisions == actual_decisions,
            "batch decisions are not exactly one per call: expected {expected_decisions:?}, got {actual_decisions:?}",
        );

        // Every decision's priority must be finite.
        for d in &self.decisions {
            anyhow::ensure!(
                d.priority.is_finite(),
                "batch decision priority for '{}' is not finite",
                d.call_id,
            );
        }

        Ok(())
    }

    /// Apply this plan to reorder `calls` and return the ordered list.
    ///
    /// Panics if `validate_against` would fail.
    pub fn apply_to(
        &self,
        calls: &[crate::CapabilityCall],
    ) -> anyhow::Result<Vec<crate::CapabilityCall>> {
        self.validate_against(calls)?;

        let by_id: std::collections::HashMap<&str, &crate::CapabilityCall> =
            calls.iter().map(|c| (c.call_id.as_str(), c)).collect();

        let ordered = self
            .ordered_call_ids
            .iter()
            .map(|id| {
                by_id
                    .get(id.as_str())
                    .cloned()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("call_id '{id}' not in input batch"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(ordered)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dasein::{SelfVersion, Stimmung};
    use crate::{
        ContextProjectionReceipt, SelectionExplanation, SelectionResult, StructuredSelfView,
        VisibilityScope, WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent,
        WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
    };

    // ---- helpers ----

    fn empty_projection() -> ConsciousContextProjection {
        let view = StructuredSelfView {
            version: SelfVersion(1),
            mood: Stimmung::Gelassenheit,
            concerns: vec![],
            care_concerns: vec![],
            projection: None,
            protentions: vec![],
        };
        let receipt = ContextProjectionReceipt {
            space: AgoraSpaceId("test-space".into()),
            broadcast_epoch: None,
            workspace_version: None,
            dasein_version: SelfVersion(1),
            content_ids: vec![],
        };
        ConsciousContextProjection {
            latest_broadcast: None,
            self_view: view,
            receipt,
        }
    }

    fn fixture_call(id: &str) -> crate::CapabilityCall {
        crate::CapabilityCall {
            operation_id: crate::OperationId::new(),
            process_id: crate::ProcessId::new(),
            name: "test_tool".into(),
            input: serde_json::json!({}),
            call_id: id.to_string(),
            deadline: None,
        }
    }

    fn selected_candidate(
        id: u128,
        content: WorkspaceContent,
        salience: SalienceVector,
    ) -> WorkspaceCandidate {
        let source = crate::ProcessId(uuid::Uuid::from_u128(90));
        WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: crate::ContentId(uuid::Uuid::from_u128(id)),
            space: AgoraSpaceId("test-space".into()),
            source,
            turn: None,
            content,
            confidence: 1.0,
            salience,
            provenance: WorkspaceProvenance {
                producer: source,
                operation: None,
                source_refs: vec![format!("fixture:{id}")],
                observed_at: crate::WallTime(1),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![],
            created_at: crate::MonoTime(1),
            expires_at: None,
        }
    }

    fn projection_with_selected(selected: Vec<WorkspaceCandidate>) -> ConsciousContextProjection {
        let selected_ids = selected
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let broadcast = WorkspaceBroadcast::from_selection(
            BroadcastEpoch(7),
            SelectionResult {
                selected,
                explanation: SelectionExplanation {
                    policy_version: 1,
                    evaluated: vec![],
                    selected_ids: selected_ids.clone(),
                    rejected_below_ignition: vec![],
                },
            },
            SelfVersion(1),
            1,
        )
        .unwrap();
        ConsciousContextProjection {
            latest_broadcast: Some(broadcast),
            self_view: StructuredSelfView {
                version: SelfVersion(1),
                mood: Stimmung::Gelassenheit,
                concerns: vec![],
                care_concerns: vec![],
                projection: None,
                protentions: vec![],
            },
            receipt: ContextProjectionReceipt {
                space: AgoraSpaceId("test-space".into()),
                broadcast_epoch: Some(BroadcastEpoch(7)),
                workspace_version: Some(1),
                dasein_version: SelfVersion(1),
                content_ids: selected_ids,
            },
        }
    }

    fn zero_salience() -> SalienceVector {
        SalienceVector {
            urgency: 0.0,
            goal_relevance: 0.0,
            self_relevance: 0.0,
            novelty: 0.0,
            confidence: 0.0,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        }
    }

    // ---- readout tests ----

    #[test]
    fn readout_empty_projection_returns_none() {
        let proj = empty_projection();
        let readout = ConsciousFieldReadout::from_projection(&proj).unwrap();
        assert!(readout.is_none(), "empty projection should return None");
    }

    #[test]
    fn readout_invalid_projection_is_error() {
        // A projection with mismatched broadcast epoch should fail validation.
        let mut proj = empty_projection();
        proj.receipt.broadcast_epoch = Some(BroadcastEpoch(99));
        // This creates a projection that fails validate() because the receipt
        // has broadcast_epoch but latest_broadcast is None.
        let result = ConsciousFieldReadout::from_projection(&proj);
        assert!(
            result.is_err(),
            "invalid projection should be Err, got {result:?}"
        );
    }

    #[test]
    fn readout_aggregates_each_selected_salience_dimension() {
        let low_urgency = selected_candidate(
            1,
            WorkspaceContent::CareConcern(crate::CareConcernFrame {
                purpose: "goal".into(),
                urgency: 0.2,
            }),
            SalienceVector {
                urgency: 0.2,
                goal_relevance: 0.9,
                self_relevance: 0.1,
                novelty: 0.8,
                confidence: 0.3,
                prediction_error: 0.7,
                affect_intensity: 0.4,
                social_relevance: 0.6,
            },
        );
        let high_urgency = selected_candidate(
            2,
            WorkspaceContent::CareConcern(crate::CareConcernFrame {
                purpose: "urgent".into(),
                urgency: 0.9,
            }),
            SalienceVector {
                urgency: 0.9,
                goal_relevance: 0.2,
                self_relevance: 0.95,
                novelty: 0.1,
                confidence: 0.85,
                prediction_error: 0.2,
                affect_intensity: 0.75,
                social_relevance: 0.1,
            },
        );
        let readout = ConsciousFieldReadout::from_projection(&projection_with_selected(vec![
            low_urgency,
            high_urgency,
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            readout.salience.values(),
            [0.9, 0.9, 0.95, 0.8, 0.85, 0.7, 0.75, 0.6]
        );
    }

    #[test]
    fn strongest_selected_care_action_wins_regardless_of_order() {
        let direct = selected_candidate(
            3,
            WorkspaceContent::Concern(crate::dasein::SelfSignal::CareDecision {
                action: CareActionKind::Direct,
                rationale: "first".into(),
            }),
            zero_salience(),
        );
        let negate = selected_candidate(
            4,
            WorkspaceContent::Concern(crate::dasein::SelfSignal::CareDecision {
                action: CareActionKind::Negate,
                rationale: "later".into(),
            }),
            zero_salience(),
        );
        for selected in [
            vec![direct.clone(), negate.clone()],
            vec![negate.clone(), direct.clone()],
        ] {
            let readout =
                ConsciousFieldReadout::from_projection(&projection_with_selected(selected))
                    .unwrap()
                    .unwrap();
            assert_eq!(readout.care_action, Some(CareActionKind::Negate));
            assert_eq!(readout.precision, 1.0);
        }
    }

    // ---- batch plan tests ----

    #[test]
    fn batch_plan_identity_preserves_input_order() {
        let calls = vec![fixture_call("c"), fixture_call("a"), fixture_call("b")];
        let plan = CapabilityBatchPlan::identity(&calls);
        assert_eq!(plan.ordered_call_ids, vec!["c", "a", "b"]);
        assert!(plan
            .decisions
            .iter()
            .all(|d| d.decision == FieldDecisionKind::Proceed
                && d.reason == FieldDecisionReason::FieldAbsent));
        assert!(plan.mode == ConsciousArbitrationMode::Observe);
    }

    #[test]
    fn batch_plan_validates_exact_permutation() {
        let calls = vec![fixture_call("a"), fixture_call("b")];

        // Valid permutation.
        let plan = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["b".into(), "a".into()],
            decisions: vec![
                CapabilityBatchDecision {
                    call_id: "b".into(),
                    decision: FieldDecisionKind::Reorder,
                    reason: FieldDecisionReason::Selected,
                    priority: 0.9,
                    broadcast_epoch: None,
                },
                CapabilityBatchDecision {
                    call_id: "a".into(),
                    decision: FieldDecisionKind::Reorder,
                    reason: FieldDecisionReason::Selected,
                    priority: 0.2,
                    broadcast_epoch: None,
                },
            ],
        };
        assert!(plan.validate_against(&calls).is_ok());

        // Non-permutation: duplicate.
        let bad = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["a".into(), "a".into()],
            decisions: vec![],
        };
        assert!(bad.validate_against(&calls).is_err());
    }

    #[test]
    fn batch_plan_rejects_non_permutations() {
        let calls = vec![fixture_call("a"), fixture_call("b")];
        let plan = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["a".into(), "a".into()],
            decisions: vec![],
        };
        assert!(plan.validate_against(&calls).is_err());
    }

    #[test]
    fn batch_plan_rejects_unknown_call_id_in_decisions() {
        let calls = vec![fixture_call("a")];
        let plan = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["a".into()],
            decisions: vec![CapabilityBatchDecision {
                call_id: "ghost".into(),
                decision: FieldDecisionKind::Defer,
                reason: FieldDecisionReason::Negated,
                priority: 0.5,
                broadcast_epoch: None,
            }],
        };
        assert!(plan.validate_against(&calls).is_err());
    }

    #[test]
    fn batch_plan_requires_exactly_one_decision_per_call() {
        let calls = vec![fixture_call("a"), fixture_call("b")];
        let decision = |call_id: &str| CapabilityBatchDecision {
            call_id: call_id.into(),
            decision: FieldDecisionKind::Proceed,
            reason: FieldDecisionReason::Selected,
            priority: 0.5,
            broadcast_epoch: Some(BroadcastEpoch(1)),
        };
        let missing = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["a".into(), "b".into()],
            decisions: vec![decision("a")],
        };
        assert!(missing.validate_against(&calls).is_err());

        let duplicate = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["a".into(), "b".into()],
            decisions: vec![decision("a"), decision("a")],
        };
        assert!(duplicate.validate_against(&calls).is_err());
    }

    #[test]
    fn batch_plan_rejects_duplicate_input_call_ids() {
        let calls = vec![fixture_call("same"), fixture_call("same")];
        let plan = CapabilityBatchPlan::identity(&calls);
        assert!(plan.validate_against(&calls).is_err());
    }

    #[test]
    fn batch_plan_apply_to_reorders() {
        let calls = vec![fixture_call("a"), fixture_call("b"), fixture_call("c")];
        let plan = CapabilityBatchPlan {
            mode: ConsciousArbitrationMode::Enforce,
            ordered_call_ids: vec!["c".into(), "a".into(), "b".into()],
            decisions: vec![
                CapabilityBatchDecision {
                    call_id: "c".into(),
                    decision: FieldDecisionKind::Reorder,
                    reason: FieldDecisionReason::Selected,
                    priority: 1.0,
                    broadcast_epoch: None,
                },
                CapabilityBatchDecision {
                    call_id: "a".into(),
                    decision: FieldDecisionKind::Reorder,
                    reason: FieldDecisionReason::Selected,
                    priority: 0.5,
                    broadcast_epoch: None,
                },
                CapabilityBatchDecision {
                    call_id: "b".into(),
                    decision: FieldDecisionKind::Reorder,
                    reason: FieldDecisionReason::Selected,
                    priority: 0.2,
                    broadcast_epoch: None,
                },
            ],
        };
        let reordered = plan.apply_to(&calls).unwrap();
        assert_eq!(
            reordered
                .iter()
                .map(|c| c.call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn conscious_arbitration_mode_default_is_observe() {
        assert_eq!(
            ConsciousArbitrationMode::default(),
            ConsciousArbitrationMode::Observe
        );
    }

    #[test]
    fn conscious_arbitration_mode_parse_env() {
        assert_eq!(
            ConsciousArbitrationMode::parse_env("observe"),
            Some(ConsciousArbitrationMode::Observe)
        );
        assert_eq!(
            ConsciousArbitrationMode::parse_env("enforce"),
            Some(ConsciousArbitrationMode::Enforce)
        );
        assert_eq!(
            ConsciousArbitrationMode::parse_env("  OBSERVE  "),
            Some(ConsciousArbitrationMode::Observe)
        );
        assert_eq!(ConsciousArbitrationMode::parse_env("garbage"), None);
    }

    #[test]
    fn care_action_weights_are_correct() {
        // Verify the design.md §5.2 weight table.
        assert_eq!(
            ConsciousFieldReadout::care_action_weight(CareActionKind::Direct),
            0.25
        );
        assert!(ConsciousFieldReadout::care_action_weight(CareActionKind::Deliberate) > 0.25);
        assert!(ConsciousFieldReadout::care_action_weight(CareActionKind::Wait) > 0.60);
        assert_eq!(
            ConsciousFieldReadout::care_action_weight(CareActionKind::Negate),
            1.00
        );
    }

    #[test]
    fn field_decision_kinds_are_distinct() {
        // All four variants must deserialize correctly.
        let proceed: FieldDecisionKind = serde_json::from_str("\"proceed\"").unwrap();
        assert_eq!(proceed, FieldDecisionKind::Proceed);

        let reorder: FieldDecisionKind = serde_json::from_str("\"reorder\"").unwrap();
        assert_eq!(reorder, FieldDecisionKind::Reorder);

        let would_defer: FieldDecisionKind = serde_json::from_str("\"would_defer\"").unwrap();
        assert_eq!(would_defer, FieldDecisionKind::WouldDefer);

        let defer: FieldDecisionKind = serde_json::from_str("\"defer\"").unwrap();
        assert_eq!(defer, FieldDecisionKind::Defer);
    }

    #[test]
    fn field_decision_reasons_are_distinct() {
        assert_eq!(
            serde_json::from_str::<FieldDecisionReason>("\"field_absent\"").unwrap(),
            FieldDecisionReason::FieldAbsent
        );
        assert_eq!(
            serde_json::from_str::<FieldDecisionReason>("\"selected\"").unwrap(),
            FieldDecisionReason::Selected
        );
        assert_eq!(
            serde_json::from_str::<FieldDecisionReason>("\"negated\"").unwrap(),
            FieldDecisionReason::Negated
        );
        assert_eq!(
            serde_json::from_str::<FieldDecisionReason>("\"lost_competition\"").unwrap(),
            FieldDecisionReason::LostCompetition
        );
    }
}
