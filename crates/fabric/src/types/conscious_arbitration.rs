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
                crate::WorkspaceContent::Concern(signal) => match signal {
                    crate::dasein::SelfSignal::CareDecision { action, .. } => {
                        Some(Self::care_action_weight(action.clone()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .fold(0.0_f32, f32::max);

        // Find the care action from the broadcast
        let care_action: Option<CareActionKind> = broadcast
            .selected
            .iter()
            .filter_map(|w| match &w.content {
                crate::WorkspaceContent::Concern(signal) => match signal {
                    crate::dasein::SelfSignal::CareDecision { action, .. } => Some(action.clone()),
                    _ => None,
                },
                _ => None,
            })
            .next();

        // Max salience across all selected candidates
        let salience = broadcast
            .selected
            .iter()
            .max_by(|a, b| {
                let sa = a.salience.urgency.max(a.salience.self_relevance);
                let sb = b.salience.urgency.max(b.salience.self_relevance);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|w| w.salience)
            .unwrap_or_else(|| SalienceVector {
                urgency: 0.0,
                goal_relevance: 0.0,
                self_relevance: 0.0,
                novelty: 0.0,
                confidence: 0.0,
                prediction_error: 0.0,
                affect_intensity: 0.0,
                social_relevance: 0.0,
            });

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

        let mut actual = self.ordered_call_ids.clone();
        actual.sort();

        anyhow::ensure!(
            expected == actual,
            "batch plan is not an exact permutation: expected {:?}, got {:?}",
            expected,
            actual,
        );

        // Every decision must reference a known call_id.
        let call_ids: std::collections::HashSet<&str> =
            calls.iter().map(|c| c.call_id.as_str()).collect();
        for d in &self.decisions {
            anyhow::ensure!(
                call_ids.contains(d.call_id.as_str()),
                "batch decision references unknown call_id '{}'",
                d.call_id,
            );
        }

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
                    .ok_or_else(|| anyhow::anyhow!("call_id '{}' not in input batch", id))
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
    use crate::ContextProjectionReceipt;
    use crate::StructuredSelfView;

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
            "invalid projection should be Err, got {:?}",
            result
        );
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
