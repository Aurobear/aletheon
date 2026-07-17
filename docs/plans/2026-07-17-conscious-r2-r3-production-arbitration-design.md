# Conscious-Core R2/R3 Production Arbitration Design

> **Status:** Approved design; implementation not started
> **Date:** 2026-07-17
> **Source requirements:**
> - R2 field feedback and fallback: `docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:13-25`
> - R3 arbitration, metrics, and acceptance: `docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:15-39`
> - Cross-batch invariants and rollout: `docs/plans/deepseek/2026-07-17-conscious-core-engineering-plan.md:233-258`

## 1. Decision summary

R2 and R3 will use the Dasein/Agora field as their only consciousness signal. `CapabilityAuthority.risk` remains an independent safety input and is not converted into a consciousness signal.

The read contract moves to Fabric while its production implementation remains in Executive. Dasein's `SelfField` consumes the Fabric contract without depending on Executive. R3 first performs batch ranking, then applies per-call soft-defer arbitration; the existing authorization/admission/execution pipeline remains the final authority.

Production rollout is observe-first:

1. R2 field feedback and field metrics are enabled with strict empty/error fallback.
2. R3 ships in `Observe` mode by default and records what it would reorder or defer.
3. `Enforce` is an explicit configuration choice after the observed decision distribution is accepted.

This design satisfies the requirement that the field may only tighten behavior and may never relax safety (`docs/plans/deepseek/2026-07-17-conscious-core-engineering-plan.md:209-217`).

## 2. Requirement-to-code reconciliation

| Requirement description | Current code reality | Agree? / resolution |
|---|---|---|
| Inject the existing latest-conscious-context read port into `SelfField` (`docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:8-16`) | The trait exists in Executive at `crates/executive/src/service/conscious_core_ports.rs:156-162`, while Dasein depends on Fabric and Kernel only at `crates/dasein/Cargo.toml:9-12`. | No. Move the contract to Fabric and keep Executive implementations at `crates/executive/src/service/conscious_workspace.rs:274-287` and `crates/executive/src/service/conscious_core_coordinator.rs:736-760`. |
| Empty context must preserve the exact R2-predecessor behavior (`docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:19-25`) | The port returns `Result<ConsciousContextProjection>`, and emptiness is represented by `latest_broadcast: None` at `crates/fabric/src/types/conscious_core.rs:263-291`. | Yes after clarifying semantics: absent reader, read error, or `latest_broadcast: None` all use the unmodified baseline path. |
| Use `CareAction` and concern urgency rather than constants (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:15-20`) | R1 emits a Fabric `CareActionKind` at `crates/dasein/src/dasein/reducer.rs:408-427`; action confidence/salience remain constant at `crates/executive/src/service/conscious_action.rs:109-126`; concern urgency remains `0.7` at `crates/executive/src/service/conscious_core_coordinator.rs:395-415`. | No today. R2/R3 will read the R1 decision and the selected concern urgency from the latest broadcast and remove the two constant projections. |
| Reorder competing calls in the same turn (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:16-20`) | Cognit currently executes collected tool calls sequentially in provider order at `crates/cognit/src/harness/linear/tool_exec.rs:200-224` and `crates/cognit/src/harness/linear/tool_exec.rs:356-363`; `TurnServices` exposes only single-call invocation at `crates/fabric/src/include/turn.rs:103-121`. | No. Add a Fabric-owned batch planning contract and let Cognit apply the returned stable order before invoking each call. |
| A selected action must affect execution (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:16-20`) | Executive selects at `crates/executive/src/service/governed_capability.rs:148-163` but always calls the inner invoker at `crates/executive/src/service/governed_capability.rs:164-172`. | No. Return a typed proceed/defer decision and skip the inner invocation only for enforced defer. |
| Consciousness cannot bypass safety (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:28-39`) | Authorization currently runs before conscious action selection at `crates/executive/src/service/governed_capability.rs:134-163`. | Yes. Preserve this order and never turn an authorization/admission rejection into an allow. |

The discrepancies above are resolved by the architecture and signal-source decisions in this document; no requirement is silently replaced.

## 3. Alternatives considered

### 3.1 Recommended: Fabric contract, Executive adapter, Dasein consumer

- Fabric owns portable context, arbitration, trace, and batch-plan data contracts.
- Executive owns access to the workspace registry, orchestration, and production composition.
- Dasein reads the port through `SelfFieldConfig` and remains independent of Executive.
- Cognit receives only a neutral batch plan and does not know about Dasein or Executive.

This preserves dependency direction and supports real same-turn reorder.

### 3.2 Executive-only adapter around `SelfField`

This avoids moving the trait but leaves `SelfField::review()` unaware of the field. It would make Executive duplicate or post-process care scoring, contrary to the R2 requirement that the shared field modulate SelfField itself. Rejected.

### 3.3 Add a Dasein-to-Executive dependency

This would let Dasein import the current trait directly, but reverses the existing composition boundary and creates a dependency cycle risk. Rejected.

### 3.4 Use `CapabilityAuthority.risk` as the R3 signal

Risk is already a trusted safety input in `CapabilityAuthority` (`crates/fabric/src/include/turn.rs:50-67`). Reusing it as consciousness would conflate safety policy with care-field state and could make `Direct` appear to weaken a high-risk decision. Rejected. Risk remains a separate safety lower bound.

## 4. Architecture

```text
 Dasein reducer
   CareDecision + CareConcern urgency
             |
             v
      Agora competition/broadcast
             |
             v
 Fabric LatestConsciousContextPort
        |                    |
        | R2 read            | R3 read
        v                    v
 Dasein SelfField       Executive field arbitrator
 baseline + bounded      batch rank + per-call defer
 monotonic modulation           |
                               v
 Cognit stable batch order -> authorization -> admission -> tool execution
                                      ^
                                      |
                         existing discrete safety authority
```

Dependency direction remains:

```text
Fabric contracts <- Dasein policy
Fabric contracts <- Cognit runtime
Fabric + Dasein + Cognit <- Executive composition
```

No path writes back from `SelfField::review()` into Agora, so the R2 signal remains a one-way read as required by `docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:23-25`.

## 5. Shared field projection

### 5.1 Port ownership and composition

`LatestConsciousContextPort` moves from Executive to Fabric without changing its async read semantics. Executive's workspace registry and coordinator continue implementing it. `SelfFieldConfig` gains an optional `Arc<dyn LatestConsciousContextPort>`; `SelfField` stores the optional reader without changing its eight policy layers.

The production turn pipeline obtains the current session before review through the non-mutating `TurnSessionStatePort::current()` implementation at `crates/executive/src/service/turn_runtime_ports.rs:372-378`. It constructs `fabric::Context.session_id` from that session rather than the thread ID currently used at `crates/executive/src/service/turn_pipeline.rs:114-136`. `begin_user()` remains after an allowed review at `crates/executive/src/service/turn_pipeline.rs:213-224`, so a denied message is not persisted merely to discover its workspace.

Other SelfField callers already carrying a session ID continue to use `Context.session_id`; absence of a matching workspace degrades to baseline.

### 5.2 Deterministic field readout

For a valid latest broadcast, derive a bounded field precision `p` from selected broadcast records only:

```text
u = maximum selected CareConcern urgency, or 0
s = maximum selected candidate (urgency, self_relevance), or 0
a = selected CareDecision weight, or 0

CareDecision weights:
  Direct     = 0.25
  Deliberate = 0.60
  Wait       = 0.75
  Negate     = 1.00

p = clamp(max(u, s, a), 0, 1)
```

The fixed weights express increasing conservatism, not execution privilege. `Negate` supplies the strongest tightening signal, while `Direct` never lowers the baseline or bypasses safety.

### 5.3 R2 care/attention modulation

Let `b` be the existing keyword care score. With a usable broadcast:

```text
effective_care = clamp(b + (1 - b) * 0.25 * p, 0, 1)
modulation     = effective_care - b
```

The coefficient is deliberately bounded to 25% of the remaining headroom. The result is monotonic (`effective_care >= b`), so field feedback can increase attention or require more confirmation but cannot reduce an existing restriction.

When the reader is absent, the read fails, the projection fails validation, or `latest_broadcast` is `None`, `effective_care` is exactly `b`; the pre-R2 permission, narrative, and attention path is used without additional rounding or thresholds. This is the exact fallback required by AC-R2.2 (`docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:19-21`). A read failure is logged as degraded diagnostics but does not fail the user's request.

## 6. R3 arbitration

### 6.1 Modes

Fabric defines a serialized `ConsciousArbitrationMode`:

- `Observe` (default): compute and trace reorder/defer decisions, but preserve provider order and execute calls normally.
- `Enforce`: apply stable reorder and return deferred results without executing deferred calls.

There is no mode that permits consciousness to relax authorization, admission, sandbox, budget, lease, or tool policy.

### 6.2 Derived action salience

Action proposals no longer use `confidence = 1.0` and an all-maximum salience vector. The arbitrator derives proposal precision from the same field projection:

- `urgency`: selected concern urgency.
- `self_relevance`: selected broadcast self-relevance.
- `goal_relevance`: maximum selected goal relevance.
- `prediction_error`, `affect_intensity`, `novelty`, and `social_relevance`: corresponding bounded maxima from selected candidates.
- `confidence`: `0.5 + 0.5 * max(selected candidate confidence)`, clamped to `[0,1]`; absent context uses the legacy proposal unchanged and bypasses arbitration.

The hard-coded `CareConcernFrame.urgency = 0.7` is replaced by the originating concern's bounded urgency. Where an older signal contains no urgency, the compatibility projection uses the selected candidate's urgency rather than inventing a new constant.

### 6.3 Batch planning and stable reorder

Fabric adds a batch-planning request/result to `TurnServices`. The default implementation returns the input order unchanged, preserving compatibility for exec, tests, and runtimes without a conscious core.

For a multi-call assistant response, Cognit submits call descriptors as one batch before executing any call. Executive returns:

- one entry per input `call_id`;
- a stable priority order (higher field-adjusted precision first, original index as the tie-breaker);
- a per-call observe/enforce decision and reason code;
- the projection epoch used to decide.

Cognit validates the plan as an exact permutation: no missing, duplicate, or injected call IDs. An invalid or failed plan falls back to provider order and emits degraded diagnostics. In `Observe`, Cognit always retains provider order even when the plan records a proposed reorder. In `Enforce`, it applies the valid order.

Batch planning does not execute, authorize, or mutate calls. Each reordered call still goes through the normal single-call invocation boundary.

### 6.4 Per-call proceed/defer

`GovernedActionLoop::select_action` returns a typed decision:

```text
Proceed { selected action context, modulation snapshot }
Defer   { reason, retryable, modulation snapshot }
```

A call is eligible for defer only when a valid field projection contains `CareDecision::Negate` or the submitted action loses Agora selection. Empty/degraded context always returns `Proceed` with the legacy behavior.

The invocation sequence is fixed:

```text
trusted authorization
  -> field selection/arbitration
     -> Observe: execute + trace would-defer
     -> Enforce/Proceed: execute
     -> Enforce/Defer: trace + structured result, no inner invocation
  -> existing admission / permit / executor for executed calls
```

Authorization intentionally remains first. Thus a safety-invalid call with `Direct` is denied before any consciousness decision, satisfying AC-R3.2 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:28-31`).

An enforced defer returns `CapabilityResult` with:

- the original `call_id`;
- `is_error = true`;
- a stable structured JSON payload with code `consciousness_deferred`, `retryable = true`, the bounded reason code, and projection epoch;
- default usage and no permit/audit identity, proving that admission/execution did not occur.

It is not sent through `observe_outcome`, because that path correctly requires an execution permit at `crates/executive/src/service/conscious_action.rs:172-181`. A separate pre-execution modulation observer records reorder/would-defer/defer decisions.

## 7. Metrics and audit trace

Metrics ship in the same batch as R3, as required by `docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:22-26`.

### 7.1 Bounded state history

Each conscious workspace maintains at most 64 metric snapshots. A snapshot contains only bounded numeric field state and causal identifiers:

- broadcast epoch and Dasein version;
- the eight salience dimensions;
- care-action category and concern urgency;
- update `L1` delta from the preceding snapshot;
- prediction/protention alignment input;
- no prompts, tool inputs, secrets, or hidden reasoning.

### 7.2 Computable indicators

- **Attractor boundedness:** every dimension must be finite in `[0,1]`; the rolling norm is bounded. A quiet window is converged when the final eight `L1` deltas are non-increasing within a fixed epsilon and the last delta is below the configured convergence threshold.
- **Lagged mutual information:** quantize each bounded dimension into 16 fixed bins and calculate empirical `I(S_t; S_{t+k})` over the 64-snapshot window. Tests compare the same deterministic continuous history with a history whose lineage is reset; the reset boundary must produce a lower value.
- **Field update/temporality:** record non-zero update deltas for changing inputs and confirm that repeated quiet cycles do not grow the delta.
- **Protention/action alignment:** compare the prior prediction/protention salience direction with the subsequent action proposal direction using bounded cosine alignment; report `None` when either vector has zero norm.
- **F/G proxies:** expose bounded belief entropy, lagged mutual information, and attractor/update measures as explicitly named proxies. Do not claim a complete free-energy-principle implementation.

### 7.3 Structured trace

Fabric's conscious trace schema gains a field-modulation event containing:

- mode (`observe` or `enforce`);
- decision (`reorder`, `would_defer`, `defer`, or `proceed`);
- bounded reason code;
- operation/call IDs and broadcast epoch;
- baseline, effective score, and modulation delta where applicable;
- metric snapshot/checksum reference.

Executed, permit-bearing outcomes continue to use the existing governed-action outcome path. Pre-execution modulation has its own typed event so deferred calls remain auditable without fabricating a permit.

## 8. Error handling and invariants

1. **No context, no change:** missing reader/workspace, port error, invalid projection, or empty broadcast preserves exact baseline scoring, provider order, and execution behavior.
2. **Only tighten:** field modulation never lowers care score, grants permission, reduces sandboxing, or converts a safety denial into an allow.
3. **Stable and bounded:** all field inputs are validated as finite `[0,1]` values; ranking uses original index as deterministic tie-breaker.
4. **No hidden side effects:** batch planning and deferred decisions do not call admission or the tool executor.
5. **Trace failure policy:** inability to persist a pre-execution trace fails closed only in `Enforce` mode; in `Observe` it logs degradation and preserves legacy execution.
6. **No feedback loop:** review reads the latest completed broadcast and never synchronously triggers a new conscious cycle.

## 9. Verification and acceptance mapping

| Acceptance criterion | Deterministic verification |
|---|---|
| AC-R2.1 (`docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:19-21`) | Same intent/baseline with low and high urgency projections; high urgency produces a strictly higher attention weight. |
| AC-R2.2 (`docs/plans/deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md:19-21`) | Snapshot/behavior comparison with absent reader, read error, and empty broadcast; verdict and attention equal the pre-R2 baseline. |
| AC-R3.1 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:28-30`) | `Enforce` + `Negate` + low-salience call returns `consciousness_deferred`; a spy invoker proves zero admission/executor calls. |
| AC-R3.2 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:29-31`) | Safety-invalid request with `Direct` remains denied by authority/admission; conscious state never reaches or changes the denial. |
| AC-R3.3 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:30-31`) | Empty/degraded context leaves provider order unchanged and invokes every call through the legacy path. |
| Same-turn reorder (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:16-20`) | Two or more calls with different derived priorities; observe records proposed order, enforce executes the stable priority order, ties retain provider order. |
| AC-F.1 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:32-34`) | Fixed-clock quiet cycles remain finite/bounded and meet the convergence predicate. |
| AC-F.2 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:32-34`) | Deterministic continuous vs lineage-reset histories; lagged mutual information drops at reset. |
| AC-F.3 (`docs/plans/deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md:32-34`) | Every reorder/would-defer/defer produces a validated typed modulation event with causal and metric references. |

Validation includes targeted Fabric, Dasein, Cognit, and Executive tests, workspace tests, strict Clippy, formatting, and the repository architecture checks on the pinned toolchain.

## 10. Delivery slices

1. **Contracts and R2:** move the read port to Fabric, inject it into SelfField, correct session identity, implement bounded modulation and exact fallback tests.
2. **Metrics and trace:** add bounded history, deterministic indicators, typed modulation trace, and acceptance fixtures before behavior enforcement.
3. **R3 observe:** derive real salience/urgency, add batch planning and per-call decisions, default to `Observe`, and gather decision distributions.
4. **R3 enforce capability:** implement no-side-effect deferred results and stable reorder behind explicit `Enforce` configuration; keep production default `Observe`.

Each slice must preserve the cross-batch invariants in `docs/plans/deepseek/2026-07-17-conscious-core-engineering-plan.md:250-258`.

## 11. Out of scope

- A complete continuous-field or free-energy-principle runtime (Phase F).
- Claims of phenomenal consciousness or qualia.
- Any mechanism that weakens authorization, permission, admission, sandbox, lease, budget, or tool policy.
- Persisting R1 care decisions into `SelfLedger`.
- Automatic promotion from `Observe` to `Enforce`; this remains an operator decision based on measured distributions.
