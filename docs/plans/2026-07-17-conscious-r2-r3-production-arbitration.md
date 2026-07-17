# Conscious-Core R2/R3 Production Arbitration Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the R2 field-feedback loop and deliver observe-first R3 production arbitration, stable same-turn reorder, no-side-effect soft defer, and bounded field metrics without weakening the safety pipeline.

**Architecture:** Fabric owns neutral field, batch-plan, metrics, and trace contracts. Dasein consumes the read port inside `SelfField`; Executive supplies the workspace-backed implementation and safety-preserving decisions; Cognit applies only validated batch permutations. Missing/degraded consciousness preserves the legacy path, and production defaults to `Observe`.

**Tech Stack:** Rust 1.88 workspace, `async-trait`, Tokio, Serde, `parking_lot`, existing Agora/Dasein workspace contracts, deterministic unit and integration tests.

**Approved design:** `docs/plans/2026-07-17-conscious-r2-r3-production-arbitration-design.md:1-284`

**Requirement anchors:**
- R2 injection/modulation/fallback/audit: `docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:13-25`
- R3 arbitration/reorder/observe-first: `docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:15-20`
- Metrics and acceptance: `docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:22-39`
- Cross-batch invariants: `docs/plans/deepseek/2026-07-17-conscious-core-engineering-plan.md:250-258`

---

## File map

| File | Responsibility |
|---|---|
| `crates/fabric/src/types/conscious_arbitration.rs` | Context-read, field-readout, mode, decision, and batch-plan contracts. |
| `crates/fabric/src/types/conscious_field_metrics.rs` | Pure bounded history and deterministic indicator calculations. |
| `crates/fabric/src/types/conscious_core_trace.rs` | Typed pre-execution modulation evidence. |
| `crates/fabric/src/types/conscious_core.rs` | Preserve text concerns and add typed concern urgency to the shared self view. |
| `crates/fabric/src/include/turn.rs` | Identity-default batch-planning method on `TurnServices`. |
| `crates/dasein/src/core/mod.rs` | Reader injection and monotonic R2 care/attention modulation. |
| `crates/executive/src/service/conscious_context_slot.rs` | Once-bound adapter for SelfField/registry construction order. |
| `crates/executive/src/service/conscious_field.rs` | Shared readout, priority, and defer policy. |
| `crates/executive/src/service/conscious_action.rs` | Field-derived action proposal and typed proceed/defer. |
| `crates/executive/src/service/dasein_workspace_adapter.rs` | Project Dasein concern purpose and urgency without a constant. |
| `crates/executive/src/service/governed_capability.rs` | Authorization-first enforcement and structured defer. |
| `crates/cognit/src/harness/linear/tool_exec.rs` | Validated same-turn order application. |
| `crates/cognit/src/harness/session.rs` | Fabric batch-plan adapter. |
| `crates/executive/src/service/conscious_workspace.rs` | Workspace-backed read and batch planning. |
| `crates/executive/src/impl/daemon/bootstrap/request.rs` | Production reader/mode composition. |
| `crates/dasein/tests/conscious_field_feedback.rs` | AC-R2.1/2. |
| `crates/cognit/tests/conscious_batch_order.rs` | Batch identity/observe/enforce/fallback. |
| `crates/executive/tests/conscious_arbitration.rs` | AC-R3.1/2/3 and trace behavior. |
| `crates/executive/tests/functional_indicators.rs` | AC-F.1/2/3. |

---

### Task 1: Add Fabric-owned conscious arbitration contracts

**Files:**
- Create: `crates/fabric/src/types/conscious_arbitration.rs`
- Modify: `crates/fabric/src/types/mod.rs:1-46`
- Modify: `crates/fabric/src/lib.rs:56-107,168-172`
- Test: `crates/fabric/src/types/conscious_arbitration.rs`

- [ ] **Step 1: Write the failing contract tests**

```rust
#[test]
fn readout_uses_selected_field_state_and_empty_is_none() {
    let projection = fixture_projection(CareActionKind::Negate, 0.80, 0.90);
    let readout = ConsciousFieldReadout::from_projection(&projection).unwrap().unwrap();
    assert_eq!(readout.care_action, Some(CareActionKind::Negate));
    assert_eq!(readout.concern_urgency, 0.80);
    assert_eq!(readout.precision, 1.0);
    assert!(ConsciousFieldReadout::from_projection(&fixture_empty_projection()).unwrap().is_none());
}

#[test]
fn batch_plan_rejects_non_permutations() {
    let calls = vec![call("a"), call("b")];
    let plan = CapabilityBatchPlan::enforce(vec!["a".into(), "a".into()], vec![]);
    assert!(plan.validate_against(&calls).is_err());
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test -p fabric conscious_arbitration --lib`

Expected: FAIL because the module/types do not exist.

- [ ] **Step 3: Implement the complete contract surface**

```rust
#[async_trait]
pub trait LatestConsciousContextPort: Send + Sync {
    async fn latest_context(&self, space: &AgoraSpaceId)
        -> anyhow::Result<ConsciousContextProjection>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsciousArbitrationMode { #[default] Observe, Enforce }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDecisionKind { Proceed, Reorder, WouldDefer, Defer }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDecisionReason { FieldAbsent, Selected, Negated, LostCompetition }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsciousFieldReadout {
    pub epoch: BroadcastEpoch,
    pub care_action: Option<CareActionKind>,
    pub concern_urgency: f32,
    pub salience: SalienceVector,
    pub precision: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBatchDecision {
    pub call_id: String,
    pub decision: FieldDecisionKind,
    pub reason: FieldDecisionReason,
    pub priority: f32,
    pub broadcast_epoch: Option<BroadcastEpoch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBatchPlan {
    pub mode: ConsciousArbitrationMode,
    pub ordered_call_ids: Vec<String>,
    pub decisions: Vec<CapabilityBatchDecision>,
}
```

Implement `ConsciousFieldReadout::from_projection` as `anyhow::Result<Option<Self>>` with the exact weights and max/clamp formula from `design.md:104-135`: validation errors remain distinguishable from an empty broadcast. Implement `CapabilityBatchPlan::identity`, `validate_against`, and `apply_to`; equality must cover every unique input `call_id` exactly once.

- [ ] **Step 4: Export and verify**

Run: `cargo test -p fabric conscious_arbitration --lib`

Expected: PASS; empty is `None`, Negate precision is `1.0`, and malformed plans fail.

- [ ] **Step 5: Commit**

```bash
git add crates/fabric/src/types/conscious_arbitration.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs
git commit -F - <<'EOF'
feat(fabric): define conscious arbitration contracts

R2 and R3 need a dependency-neutral field read and batch planning surface.
Define validated contracts in Fabric so Dasein, Cognit, and Executive can
participate without reversing crate dependencies.

- add bounded field readout and observe/enforce modes
- add typed decisions and exact-permutation batch plans
- preserve empty-context identity behavior
EOF
```

---

### Task 2: Inject the R2 reader and prove exact fallback

**Files:**
- Create: `crates/dasein/tests/conscious_field_feedback.rs`
- Modify: `crates/dasein/src/core/mod.rs:47-66,87-114,116-183,387-476`

- [ ] **Step 1: Write R2 acceptance tests**

```rust
#[tokio::test]
async fn higher_field_urgency_raises_attention_for_same_intent() {
    let low = review_priority(StubMode::Broadcast(CareActionKind::Direct, 0.10)).await;
    let high = review_priority(StubMode::Broadcast(CareActionKind::Negate, 0.90)).await;
    assert!(high > low, "high={high} low={low}");
}

#[tokio::test]
async fn empty_and_error_equal_legacy_baseline() {
    let baseline = review_priority_without_reader().await;
    assert_eq!(review_priority(StubMode::Empty).await, baseline);
    assert_eq!(review_priority(StubMode::Error).await, baseline);
}
```

Use a fixed clock and read `SelfField::attention().current_focus().unwrap().priority`.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p dasein --test conscious_field_feedback --no-fail-fast`

Expected: FAIL because SelfField has no reader.

- [ ] **Step 3: Implement monotonic modulation**

Add `conscious_context: Option<Arc<dyn fabric::LatestConsciousContextPort>>` to `SelfFieldConfig` and `SelfField`, defaulting to `None`. Add:

```rust
async fn effective_care_score(&self, baseline: f64, ctx: &fabric::Context) -> f64 {
    let Some(reader) = &self.conscious_context else { return baseline; };
    let Ok(projection) = reader.latest_context(&fabric::AgoraSpaceId(ctx.session_id.clone())).await
    else { tracing::warn!(session_id = %ctx.session_id, "using baseline care"); return baseline; };
    let Ok(Some(readout)) = fabric::ConsciousFieldReadout::from_projection(&projection)
    else { return baseline; };
    (baseline + (1.0 - baseline) * 0.25 * f64::from(readout.precision)).clamp(0.0, 1.0)
}
```

Compute the baseline once in `review`, then use the effective score for permission, narrative, and attention. Do not write Agora or run a cycle.

- [ ] **Step 4: Verify focused and existing tests**

Run: `cargo test -p dasein --test conscious_field_feedback --no-fail-fast && cargo test -p dasein --lib --no-fail-fast`

Expected: PASS with exact fallback equality.

- [ ] **Step 5: Commit**

```bash
git add crates/dasein/src/core/mod.rs crates/dasein/tests/conscious_field_feedback.rs
git commit -F - <<'EOF'
feat(dasein): close SelfField conscious feedback

SelfField currently scores care from keywords only. Inject the Fabric reader
and apply bounded monotonic modulation while preserving degraded fallback.

- read the latest field by request session
- raise care without relaxing restrictions
- cover high, low, empty, absent, and failed reads
EOF
```

---

### Task 3: Bind the production reader and correct session identity

**Files:**
- Create: `crates/executive/src/service/conscious_context_slot.rs`
- Modify: `crates/executive/src/service/mod.rs:1-25`
- Modify: `crates/executive/src/service/conscious_core_ports.rs:148-168`
- Modify: `crates/executive/src/service/context_assembler.rs:1-65`
- Modify: `crates/executive/src/service/conscious_workspace.rs:25-38,274-287`
- Modify: `crates/executive/src/service/conscious_core_coordinator.rs:25-30,736-760`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs:93-123,672-697`
- Modify: `crates/executive/src/service/turn_pipeline.rs:104-136,213-224`
- Test: `crates/executive/tests/turn_pipeline_order.rs`

- [ ] **Step 1: Add failing ordering tests**

Make `current()` return `session-field`; assert:

```rust
assert_eq!(events, vec!["session.current", "self.review:session-field", "session.begin_user"]);
```

Also assert a denied review never calls `begin_user`.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p executive --test turn_pipeline_order --no-fail-fast`

Expected: FAIL because review uses thread ID.

- [ ] **Step 3: Implement a once-bound port slot**

```rust
#[derive(Default)]
pub struct ConsciousContextSlot {
    inner: parking_lot::RwLock<Option<Arc<dyn fabric::LatestConsciousContextPort>>>,
}

impl ConsciousContextSlot {
    pub fn bind(&self, reader: Arc<dyn fabric::LatestConsciousContextPort>) -> anyhow::Result<()> {
        let mut inner = self.inner.write();
        anyhow::ensure!(inner.is_none(), "conscious context reader already bound");
        *inner = Some(reader);
        Ok(())
    }
}
```

Implement the Fabric trait by cloning the bound reader before awaiting. Remove the duplicate Executive trait and change all imports/tests to `fabric::LatestConsciousContextPort`. Inject the slot into SelfField before the Dasein handle exists; bind the registry exactly once after registry construction.

- [ ] **Step 4: Use `sessions.current()` before review**

Build `fabric::Context` with the returned session ID. Keep `begin_user()` after allow, so denied messages are not persisted.

- [ ] **Step 5: Verify Executive boundaries**

Run: `cargo test -p executive --test turn_pipeline_order --test conscious_workspace_production --test agent_agora_projection --no-fail-fast`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/conscious_context_slot.rs crates/executive/src/service/mod.rs crates/executive/src/service/conscious_core_ports.rs crates/executive/src/service/context_assembler.rs crates/executive/src/service/conscious_workspace.rs crates/executive/src/service/conscious_core_coordinator.rs crates/executive/src/impl/daemon/bootstrap/request.rs crates/executive/src/service/turn_pipeline.rs crates/executive/tests/turn_pipeline_order.rs crates/executive/tests/conscious_workspace_production.rs crates/executive/tests/agent_agora_projection.rs
git commit -F - <<'EOF'
feat(executive): wire production conscious field reads

SelfField is constructed before the recurrent workspace. Add a once-bound
adapter and use the real session identity before policy review.

- move latest-context consumers to Fabric
- bind the workspace after Dasein construction
- avoid persisting denied messages
EOF
```

---

### Task 4: Add bounded metrics and typed modulation trace

**Files:**
- Create: `crates/fabric/src/types/conscious_field_metrics.rs`
- Modify: `crates/fabric/src/types/mod.rs:1-46`
- Modify: `crates/fabric/src/lib.rs:89-107`
- Modify: `crates/fabric/src/types/conscious_core_trace.rs:10-65`
- Modify: `crates/executive/src/service/conscious_core_coordinator.rs:108-180,300-360`
- Test: `crates/executive/tests/functional_indicators.rs`

- [ ] **Step 1: Write deterministic metric tests**

```rust
#[test]
fn lineage_reset_reduces_lagged_mutual_information() {
    let continuous = FieldMetricHistory::from_snapshots(continuous_fixture()).unwrap();
    let reset = FieldMetricHistory::from_snapshots(reset_fixture()).unwrap();
    assert!(continuous.lagged_mutual_information(1).unwrap()
        > reset.lagged_mutual_information(1).unwrap());
}

#[test]
fn history_is_bounded_and_quiet_tail_converges() {
    let mut history = FieldMetricHistory::default();
    for epoch in 1..=80 { history.push(quiet_snapshot(epoch)).unwrap(); }
    assert_eq!(history.len(), 64);
    assert!(history.indicators().attractor_converged);
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p fabric conscious_field_metrics --lib`

Expected: FAIL because the module is absent.

- [ ] **Step 3: Implement the metric engine**

Define `FieldMetricSnapshot`, `FieldMetricIndicators`, and a 64-entry `VecDeque` history. Quantize `[0,1]` into 16 bins with `min(floor(value * 16), 15)`. Compute empirical MI as `sum pxy * ln(pxy/(px*py))`, L1 update delta, bounded entropy proxies, and cosine alignment (`None` for zero norm). Use an eight-sample quiet convergence window and `1e-4` epsilon. The module owns no clock/I/O.

- [ ] **Step 4: Extend trace and coordinator recording**

Add:

```rust
FieldModulation {
    mode: ConsciousArbitrationMode,
    decision: FieldDecisionKind,
    reason: FieldDecisionReason,
    operation_id: String,
    call_id: String,
    broadcast_epoch: Option<u64>,
    baseline: Option<f64>,
    effective: Option<f64>,
    delta: Option<f64>,
    metric_ref: String,
},
```

Push one snapshot after each durable completed broadcast. Expose read-only indicators. Store no prompts/tool inputs.

- [ ] **Step 5: Verify AC-F.1/2**

Run: `cargo test -p fabric conscious_field_metrics --lib && cargo test -p executive --test functional_indicators --no-fail-fast`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fabric/src/types/conscious_field_metrics.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs crates/fabric/src/types/conscious_core_trace.rs crates/executive/src/service/conscious_core_coordinator.rs crates/executive/tests/functional_indicators.rs
git commit -F - <<'EOF'
feat(conscious-core): measure bounded field dynamics

R3 requires falsifiable field evidence before enforcement. Add deterministic
bounded metrics and typed pre-execution trace data.

- measure convergence, update deltas, mutual information, and alignment
- cap history at 64 snapshots
- exclude prompts and hidden reasoning
EOF
```

---

### Task 5: Add same-turn batch planning to Fabric and Cognit

**Files:**
- Modify: `crates/fabric/src/include/turn.rs:98-121,131-155`
- Modify: `crates/cognit/src/harness/linear/tool_exec.rs:16-31,200-224,356-363`
- Modify: `crates/cognit/src/harness/session.rs:365-387`
- Modify: `crates/executive/src/service/turn_service.rs:164-203`
- Create: `crates/cognit/tests/conscious_batch_order.rs`

- [ ] **Step 1: Write order tests**

```rust
#[tokio::test]
async fn enforce_applies_exact_stable_permutation() {
    let ids = run_with_plan(ConsciousArbitrationMode::Enforce, vec!["c", "a", "b"]).await;
    assert_eq!(ids, vec!["c", "a", "b"]);
}

#[tokio::test]
async fn observe_and_invalid_plans_keep_provider_order() {
    assert_eq!(run_with_plan(ConsciousArbitrationMode::Observe, vec!["c", "a", "b"]).await,
               vec!["a", "b", "c"]);
    assert_eq!(run_with_plan(ConsciousArbitrationMode::Enforce, vec!["a", "a", "b"]).await,
               vec!["a", "b", "c"]);
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p cognit --test conscious_batch_order --no-fail-fast`

Expected: FAIL because no batch planner exists.

- [ ] **Step 3: Add the identity-default service method**

```rust
async fn plan_capability_batch(
    &self,
    calls: Vec<CapabilityCall>,
) -> anyhow::Result<CapabilityBatchPlan> {
    Ok(CapabilityBatchPlan::identity(&calls))
}
```

Delegate it through `RecordingTurnServices`.

- [ ] **Step 4: Plan once and validate locally**

Extend `run_streaming` with a planner closure receiving the complete collected tuple slice. Convert tuples to `CapabilityCall` in `session.rs`. Apply a plan only if it is a valid exact permutation; otherwise warn and retain provider order. `Observe` always retains provider order; `Enforce` applies the stable permutation.

- [ ] **Step 5: Verify Cognit regression**

Run: `cargo test -p cognit --test conscious_batch_order --test cognitive_session --test facade_contract --no-fail-fast`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fabric/src/include/turn.rs crates/cognit/src/harness/linear/tool_exec.rs crates/cognit/src/harness/session.rs crates/executive/src/service/turn_service.rs crates/cognit/tests/conscious_batch_order.rs
git commit -F - <<'EOF'
feat(cognit): support governed capability batch order

Cognit executes tool batches in provider order, preventing R3 arbitration.
Add an identity-default planner and validate every permutation before use.

- plan collected batches once
- reorder only in enforce mode
- preserve provider order on failure
EOF
```

---

### Task 6: Derive real action salience and typed decisions

**Files:**
- Create: `crates/executive/src/service/conscious_field.rs`
- Modify: `crates/executive/src/service/mod.rs:1-25`
- Modify: `crates/fabric/src/types/conscious_core.rs:201-229,263-296`
- Modify: `crates/executive/src/service/dasein_workspace_adapter.rs:26-45`
- Modify: `crates/executive/tests/context_assembler.rs:35-55`
- Modify: `crates/executive/tests/memory_workspace_entry.rs:40-60`
- Modify: `crates/executive/src/service/conscious_core_coordinator.rs:395-428`
- Modify: `crates/executive/src/service/conscious_action.rs:1-205`
- Modify: `crates/executive/src/service/governed_capability.rs:70-96`
- Test: `crates/executive/tests/conscious_action_outcome.rs`

- [ ] **Step 1: Write failing derived-signal tests**

```rust
assert_eq!(proposal.salience.urgency, 0.90);
assert!(proposal.confidence < 1.0);
assert!(matches!(decision,
    GovernedActionDecision::Defer { reason: FieldDecisionReason::Negated, .. }));
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p executive --test conscious_action_outcome --no-fail-fast`

Expected: FAIL because proposal salience is all maximum.

- [ ] **Step 3: Implement pure policy helpers**

```rust
pub fn proposal_salience(readout: &ConsciousFieldReadout) -> (f32, SalienceVector) {
    let confidence = (0.5 + 0.5 * readout.salience.confidence).clamp(0.0, 1.0);
    (confidence, SalienceVector {
        urgency: readout.concern_urgency.max(readout.salience.urgency),
        ..readout.salience
    })
}

pub fn should_defer(readout: &ConsciousFieldReadout, selected: bool)
    -> Option<FieldDecisionReason> {
    if matches!(readout.care_action, Some(CareActionKind::Negate)) {
        Some(FieldDecisionReason::Negated)
    } else if !selected {
        Some(FieldDecisionReason::LostCompetition)
    } else { None }
}
```

Extend `StructuredSelfView` with `#[serde(default)] care_concerns: Vec<CareConcernFrame>` while retaining its existing `concerns: Vec<String>` compatibility projection. Populate both from the same `CareStructureSnapshot.concerns` in `DaseinWorkspaceAdapter`: text consumers keep `purpose`, while arbitration receives `purpose + urgency`. Validate both collections against `MAX_SELF_VIEW_ITEMS` and finite `[0,1]` urgency. Add `care_concerns: vec![]` to the two existing test literals. Replace the coordinator's hard-coded `0.7` with `care_concerns` and remove `max_salience()`.

- [ ] **Step 4: Return typed `Proceed`/`Defer`**

Add `GovernedActionDecision::{Proceed, Defer}` with a bounded modulation snapshot, plus `observe_modulation`. Keep `observe_outcome` only for executed permit-bearing results. Empty/error field uses legacy proceed and no enforcement.

- [ ] **Step 5: Verify recurrence**

Run: `cargo test -p executive --test conscious_action_outcome --test conscious_core_recurrence --no-fail-fast`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fabric/src/types/conscious_core.rs crates/executive/src/service/conscious_field.rs crates/executive/src/service/mod.rs crates/executive/src/service/dasein_workspace_adapter.rs crates/executive/src/service/conscious_core_coordinator.rs crates/executive/src/service/conscious_action.rs crates/executive/src/service/governed_capability.rs crates/executive/tests/conscious_action_outcome.rs crates/executive/tests/context_assembler.rs crates/executive/tests/memory_workspace_entry.rs
git commit -F - <<'EOF'
feat(conscious-core): derive action arbitration from field state

Action proposals use maximum salience and cannot express a conservative
defer. Derive bounded precision from R1 care state and return typed decisions.

- remove constant salience and urgency
- distinguish proceed, negate, and lost competition
- separate modulation evidence from permit outcomes
EOF
```

---

### Task 7: Enforce soft defer after authorization

**Files:**
- Modify: `crates/executive/src/service/governed_capability.rs:109-190`
- Create: `crates/executive/tests/conscious_arbitration.rs`
- Modify: `crates/executive/tests/governed_capability_path.rs`

- [ ] **Step 1: Write side-effect and safety tests**

```rust
#[tokio::test]
async fn enforce_negate_defers_without_inner_call() {
    let result = enforce_negate_fixture().invoke(call()).await;
    let body: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(body["code"], "consciousness_deferred");
    assert_eq!(body["retryable"], true);
    assert_eq!(inner_call_count(), 0);
    assert_eq!(result.usage, UsageReport::default());
    assert!(result.audit_id.is_none());
}

#[tokio::test]
async fn authorization_denial_wins_over_direct_care() {
    let result = denied_authority_with_direct_field().invoke(call()).await;
    assert!(result.output.starts_with("capability authorization denied:"));
    assert_eq!(action_selection_count(), 0);
    assert_eq!(inner_call_count(), 0);
}
```

Also test Observe executes and records `WouldDefer`; empty context matches legacy output.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p executive --test conscious_arbitration --test governed_capability_path --no-fail-fast`

Expected: FAIL because the inner invoker is always called.

- [ ] **Step 3: Implement mode-aware execution**

Keep authorization first. For Enforce/Defer, persist modulation evidence then return JSON containing `code`, `retryable`, bounded `reason`, and epoch with default usage/no audit. Never call inner admission/execution. For Observe/Defer, record `WouldDefer` and execute normally. Trace failure is fail-closed before execution in Enforce and warning-only in Observe.

- [ ] **Step 4: Verify AC-R3.1/2/3**

Run: `cargo test -p executive --test conscious_arbitration --test governed_capability_path --test conscious_action_outcome --no-fail-fast`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/executive/src/service/governed_capability.rs crates/executive/tests/conscious_arbitration.rs crates/executive/tests/governed_capability_path.rs
git commit -F - <<'EOF'
feat(executive): honor conscious soft-defer decisions

The governed invoker executes even when workspace selection rejects an action.
Honor enforced defers while keeping trusted authorization first.

- return retryable deferred results without side effects
- execute would-defer only in observe mode
- preserve safety and empty-context behavior
EOF
```

---

### Task 8: Compose observe-first production batch arbitration

**Files:**
- Modify: `crates/executive/src/service/conscious_workspace.rs:55-180,290-305`
- Modify: `crates/executive/src/service/daemon_react.rs:17-127`
- Modify: `crates/executive/src/service/turn_pipeline.rs:294-340`
- Modify: `crates/executive/src/impl/daemon/mod.rs:34-56`
- Modify: `crates/executive/src/core/runtime_core.rs:66-122`
- Modify: `crates/executive/src/user_runtime/mod.rs:57-100`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs:672-697`
- Test: `crates/executive/tests/conscious_arbitration.rs`

- [ ] **Step 1: Add mode and stable-order tests**

Assert default `Observe`; explicit `Enforce`; invalid values fail; priorities `0.9,0.9,0.2` produce stable order `first-high,second-high,low`.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p executive --test conscious_arbitration --no-fail-fast`

Expected: FAIL because production has no typed mode/planner.

- [ ] **Step 3: Thread configuration**

Add `conscious_arbitration_mode: ConsciousArbitrationMode` to Executive daemon config. Parse `ALETHEON_CONSCIOUS_ARBITRATION_MODE` as only `observe|enforce`; default `observe` in both runtime builders. Thread it through `ConsciousCoreConfig`, registry, action snapshots, and batch plans.

- [ ] **Step 4: Implement stable workspace planning**

Read one projection per batch, derive bounded priorities, sort descending with original index as tie-breaker, and return one decision per call. Empty/error returns identity. Pass the planner through `DaemonStreamingTurnContext`; planning must not authorize or execute.

- [ ] **Step 5: Verify production paths**

Run: `cargo test -p executive --test conscious_arbitration --test turn_pipeline_order --test turn_service_equivalence --no-fail-fast`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/conscious_workspace.rs crates/executive/src/service/daemon_react.rs crates/executive/src/service/turn_pipeline.rs crates/executive/src/impl/daemon/mod.rs crates/executive/src/core/runtime_core.rs crates/executive/src/user_runtime/mod.rs crates/executive/src/impl/daemon/bootstrap/request.rs crates/executive/tests/conscious_arbitration.rs
git commit -F - <<'EOF'
feat(runtime): compose observe-first conscious arbitration

Thread one typed mode and one workspace-backed planner through production while
keeping observe as the safe rollout default.

- plan stable same-turn order from one projection
- validate explicit enforce configuration
- preserve identity order on degraded context
EOF
```

---

### Task 9: Complete acceptance and regression

**Files:**
- Modify: `crates/executive/tests/functional_indicators.rs`
- Modify: `crates/executive/tests/conscious_arbitration.rs`
- Modify: `docs/plans/2026-07-17-conscious-r2-r3-production-arbitration-design.md:3-20,253-276`
- Modify: `docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:3-5`
- Modify: `docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:3-5`

- [ ] **Step 1: Name all acceptance assertions**

Ensure tests explicitly cover `AC_R2_1`, `AC_R2_2`, `AC_R3_1`, `AC_R3_2`, `AC_R3_3`, `AC_F_1`, `AC_F_2`, and `AC_F_3`. AC-F.3 must deserialize and compare mode, decision, reason, call, operation, epoch, and metric reference.

- [ ] **Step 2: Run targeted suites**

```bash
cargo test -p fabric --all-targets --no-fail-fast
cargo test -p dasein --all-targets --no-fail-fast
cargo test -p cognit --all-targets --no-fail-fast
cargo test -p executive --test conscious_arbitration --test conscious_action_outcome --test functional_indicators --test governed_capability_path --test turn_pipeline_order --no-fail-fast
```

Expected: all PASS.

- [ ] **Step 3: Run strict checks**

```bash
cargo fmt --all -- --check
cargo clippy -p fabric -p dasein -p cognit -p executive --all-targets --all-features -- -D warnings
cargo check --workspace --all-targets
```

Expected: exit 0 without warnings.

- [ ] **Step 4: Run architecture and workspace regression**

Run the exact CI architecture gates, then the workspace suite:

```bash
bash tests/architecture_check.sh
bash tests/architecture_path_inventory.sh
bash scripts/architecture-check.sh
cargo test --workspace --all-targets --no-fail-fast
```

Expected: all PASS. If Rust 1.88 is unavailable locally, record the exact unavailable command and require equivalent CI before merge; do not claim local completion.

- [ ] **Step 5: Reconcile plan status after green validation**

Set the approved design to `Implemented and validated`, add commit range/commands, and mark both DeepSeek detailed plans implemented without deleting requirement history.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/tests/functional_indicators.rs crates/executive/tests/conscious_arbitration.rs docs/plans/2026-07-17-conscious-r2-r3-production-arbitration-design.md docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md
git commit -F - <<'EOF'
test(conscious-core): validate R2/R3 production arbitration

Consolidate cross-domain evidence for fallback, safety, side-effect freedom,
field invariants, and explainable modulation before reconciling plan status.

- cover every R2, R3, and metric criterion
- verify strict package and workspace regressions
- record implementation evidence in source plans
EOF
```

---

## Final completion checklist

- [ ] The context port exists only in Fabric; Dasein has no Executive dependency.
- [ ] SelfField uses real session identity and empty/error equals baseline.
- [ ] Field modulation is monotonic and never relaxes safety.
- [ ] Metrics history is bounded and excludes prompts/tool inputs.
- [ ] Observe is default; Enforce is explicit.
- [ ] Reorder is stable and accepts only exact permutations.
- [ ] Enforced defer invokes neither admission nor executor.
- [ ] Authorization remains before consciousness.
- [ ] Every modulation has typed causal/metric evidence.
- [ ] Targeted tests, Clippy, fmt, architecture, and workspace tests pass.
- [ ] The unrelated untracked `lock` file remains untouched.
