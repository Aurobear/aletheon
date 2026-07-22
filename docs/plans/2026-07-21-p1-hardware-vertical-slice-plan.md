# P1 Hardware Vertical-Slice Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drive the existing `SimulatedDevice` through a real production path — model → Corpus tool → Kernel authorization → Hardware broker → provider → settlement into Agora/Mnemosyne — by adding stable fabric protocol types, new hardware modules (observation/skill/registry/broker) with an async `EmbodimentProvider`, an executive `EmbodimentServices` composer, and six narrow Corpus robot tools.

**Architecture:** `fabric` gains version-stable embodiment DTOs (no ROS types). `hardware` gains a normalized observation/skill layer, a registry, and a broker that validates + routes + issues receipts, plus an async `EmbodimentProvider` that coexists with the existing sync `DeviceProvider::apply`. `executive` promotes `hardware` from dev-dependency to production dependency and assembles an `EmbodimentServices` group behind the narrow cross-domain `fabric::EmbodimentExecutionPort`. `corpus` exposes six governed tools backed by that port. The existing `crates/executive/tests/hardware_simulation.rs` authorization pattern (Kernel admission → permit projection → device execute) is reused as the production path's authorization spine.

**Tech Stack:** Rust workspace crates `fabric`, `hardware`, `executive`, `corpus`; `async_trait`, `tokio`, `serde`, `serde_json`.

**Spec:** `docs/plans/2026-07-21-embodied-cognition-framework-design.md` §5.

**Prerequisite:** Complete P0 Tasks 1–5 (fail-closed harness boundary, `harness_kind` wiring, `DaemonTurnEngine`, and parity) before P1 production-path wiring so lifecycle semantics are stable.

**Review stages:** Keep P1 as one acceptance goal, but ship it as four independently
reviewable stages: (A) Tasks 1–2 protocol/ID authority, (B) Tasks 3–7 Hardware
provider/broker, (C) Tasks 8–9 Executive service/composition, and (D) Tasks 10–12
Corpus tools plus production-path evidence. Do not combine these stages into one commit.

**Deferred (documented, NOT placeholders):** Full deduplication of `hardware::{OperationId, PrincipalId, MonotonicInstant}` against fabric is **out of P1 scope** — those types are semantically different from fabric's (hardware uses `String` newtypes; fabric uses `OperationId::new()` UUID-style types), and the existing `hardware_simulation.rs` bridges them by stringifying. P1 dedups only `DeviceId`/`SkillId` (identical `(pub String)` shape). The remaining ID unification is a separate follow-up (spec §6, "多套 Operation/ID/Clock").

## Binding decisions before implementation

These decisions resolve the former Task 6/8/10 ambiguity and are normative for all
code snippets below:

| Concern | Binding decision |
|---|---|
| Operation identity | Model/tool JSON never supplies an operation ID. Executive creates one `fabric::OperationId`, obtains admission/lease authority, explicitly projects it to Hardware authority, and returns the same ID in progress/result/settlement evidence. |
| Command authority | `Broker::execute` accepts an Executive-created `AuthorizedSkillRequest`; neither Broker nor Provider may mint permits. Missing, mismatched, expired, or revoked authority fails before provider execution. |
| Port ownership | `fabric::EmbodimentExecutionPort` is the only Corpus/Executive boundary. Hardware owns provider, registry, broker, observation ingest, and authority-validation contracts. |
| Tool permission | `observe`, `get_state`, `list_skills` are L0. `execute_skill`, `cancel`, and model-requested `safe_stop` are L2. Hardware/provider fail-safe is an internal safety path and does not wait for model-tool approval. |
| Cancel identity | Cancel is addressed by the host-issued `fabric::OperationId`, not arbitrary model text. Executive resolves the operation to its device and active lease before calling Broker/Provider. |
| Provider replacement | A provider swap changes only typed composition registration/configuration. It does not change Cognit, Corpus, TurnPipeline, or EmbodimentService business logic. |
| Partial surface | A registered tool must be operational. P1 must not register a tool that returns a success sentinel or `not yet wired`. |

### Stage gates

- **P1-A (Tasks 1–2):** protocol and ID authority compile independently; no Hardware production dependency yet.
- **P1-B (Tasks 3–7):** Hardware tests prove observation freshness, provider routing, permit mismatch, cancellation, disconnect, lease expiry, and internal fail-safe.
- **P1-C (Tasks 8–9):** Executive tests prove host-generated operation identity, Kernel admission/lease projection, progress bounds, timeout/cancel races, and settlement.
- **P1-D (Tasks 10–12):** all six tools are operational with the permission matrix above, production-path E2E passes, and only then is live bootstrap registration enabled.

Do not start the next stage while the preceding gate is red.

---

## Baseline anchors (re-verify before starting)

```bash
# hardware is dev-dep only; single caller is the test
rg -n "hardware" crates/executive/Cargo.toml
rg -n "use hardware|hardware::" crates/ | rg -v "tests/"
# hardware module layout + re-exports
sed -n '1,30p' crates/hardware/src/lib.rs
# fabric type module registration
sed -n '1,60p' crates/fabric/src/types/mod.rs
# corpus tool trait + registry
rg -n "trait Tool|with_network_policy_and_search|\.register\(" crates/corpus/src/tools
```

Expected: `hardware` appears under `[dev-dependencies]` only (`crates/executive/Cargo.toml:48`); no non-test production caller; `hardware/src/lib.rs:7-26` lists 8 modules; corpus tools register via `ToolRegistry::with_network_policy_and_search`.

---

## Task 1: fabric — cross-domain `DeviceId`/`SkillId` and embodiment protocol

**Files:**
- Create: `crates/fabric/src/types/embodiment.rs`
- Modify: `crates/fabric/Cargo.toml` (add `async-trait` only if absent)
- Modify: `crates/fabric/src/types/mod.rs` (declare + re-export)
- Modify: `crates/fabric/src/lib.rs` (public re-export)
- Test: `crates/fabric/src/types/embodiment.rs` (inline `#[cfg(test)]`)

Follows the fabric newtype convention (`time.rs`: `pub struct MonoTime(pub u64)` with full derive set).

- [ ] **Step 1: Write the failing test**

Create `crates/fabric/src/types/embodiment.rs` with tests first:

```rust
//! Version-stable embodiment protocol DTOs. No ROS/vendor types ever land here.

use serde::{Deserialize, Serialize};

use crate::types::operation::OperationId;
use crate::types::time::{MonoDeadline, MonoTime};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embodied_observation_roundtrips_json() {
        let obs = EmbodiedObservation {
            schema: "pose".into(),
            schema_version: 1,
            source: "sim:bot".into(),
            sequence: 7,
            source_time: MonoTime(100),
            received_at: MonoTime(105),
            valid_until: Some(MonoDeadline::after(MonoTime(105), 500)),
            confidence: 0.9,
            frame_ref: Some("map".into()),
            payload: serde_json::json!({"x": 1.0, "y": 2.0}),
            evidence: vec![EvidenceRef { kind: "rosbag".into(), uri: "artifact://b/1".into() }],
        };
        let json = serde_json::to_string(&obs).unwrap();
        let back: EmbodiedObservation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sequence, 7);
        assert_eq!(back.frame_ref.as_deref(), Some("map"));
    }

    #[test]
    fn skill_ids_are_string_newtypes() {
        assert_eq!(DeviceId("bot".into()).0, "bot");
        assert_eq!(SkillId("wave".into()).0, "wave");
    }

    #[test]
    fn operation_id_parser_accepts_uuid_and_rejects_model_text() {
        let id = OperationId::new();
        assert_eq!(id.0.to_string().parse::<OperationId>().unwrap(), id);
        assert!("cancel-latest".parse::<OperationId>().is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p fabric embodiment`
Expected: FAIL — types not defined.

- [ ] **Step 3: Add the types (below the test module or above; keep one file)**

In the same `crates/fabric/src/types/embodiment.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillId(pub String);

// Cancellation accepts only canonical UUID text. Generation remains host-only
// through OperationId::new().
impl std::str::FromStr for OperationId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(value).map(OperationId)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbodiedObservation {
    pub schema: String,
    pub schema_version: u16,
    pub source: String,
    pub sequence: u64,
    pub source_time: MonoTime,
    pub received_at: MonoTime,
    pub valid_until: Option<MonoDeadline>,
    pub confidence: f32,
    pub frame_ref: Option<String>,
    pub payload: serde_json::Value,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskClass {
    Read,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillDescriptor {
    pub skill: SkillId,
    pub device: DeviceId,
    pub summary: String,
    pub input_schema: serde_json::Value,
    pub risk: RiskClass,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub preconditions: Vec<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillRequest {
    pub skill: SkillId,
    pub device: DeviceId,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillProgress {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub fraction: f32,
    pub note: String,
    pub at: MonoTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SkillOutcome {
    Succeeded,
    Failed { reason: String },
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillResult {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub device: DeviceId,
    pub outcome: SkillOutcome,
    pub duration_ms: u64,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyEvent {
    LeaseExpired { device: DeviceId },
    ProviderDisconnected { device: DeviceId },
    StopRequested { device: DeviceId },
    FailSafeApplied { device: DeviceId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillDispatchError {
    NoProvider(String),
    Rejected(String),
}

#[async_trait::async_trait]
pub trait EmbodimentExecutionPort: Send + Sync {
    async fn execute_skill(
        &self,
        request: SkillRequest,
    ) -> Result<SkillResult, SkillDispatchError>;
}
```

- [ ] **Step 4: Declare + re-export the module**

Edit `crates/fabric/src/types/mod.rs` — add module declaration and public re-export following the file's existing convention (add near the other `pub mod` lines):

```rust
pub mod embodiment;
```

and in the crate's public surface (mirror how `time`/`workspace` types are re-exported — check `crates/fabric/src/lib.rs` for `pub use types::...`):

```rust
pub use types::embodiment::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, EvidenceRef, RiskClass,
    SafetyEvent, SkillDescriptor, SkillDispatchError, SkillId, SkillOutcome,
    SkillProgress, SkillRequest, SkillResult,
};
```

If `async-trait` is not already present in `crates/fabric/Cargo.toml`, add the same
workspace-compatible dependency version used by neighboring crates. This is the one
authoritative location of `EmbodimentExecutionPort`; Tasks 8 and 10 only implement or consume it.

- [ ] **Step 5: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p fabric embodiment`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fabric/src/types/embodiment.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs
# Suggested subject: feat(fabric): add embodiment protocol DTOs (DeviceId, SkillId, EmbodiedObservation, Skill*)
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 2: hardware — reuse fabric `DeviceId`, add `async_trait` dep

**Files:**
- Modify: `crates/hardware/Cargo.toml` (add `fabric`, `async-trait` deps)
- Modify: `crates/hardware/src/device.rs:5` (replace local `DeviceId`)
- Modify: `crates/hardware/src/lib.rs:7-26` (re-export)

- [ ] **Step 1: Add dependencies**

Edit `crates/hardware/Cargo.toml` `[dependencies]` (match workspace versions used elsewhere, e.g. executive):

```toml
fabric = { path = "../fabric" }
async-trait = "0.1"
```

- [ ] **Step 2: Replace local `DeviceId` with the fabric one**

Edit `crates/hardware/src/device.rs` — remove the local `pub struct DeviceId(pub String);` (line 5) and add near the top:

```rust
pub use fabric::DeviceId;
```

Leave `PrincipalId`, `OperationId`, `MonotonicInstant`, `CommandSequence` as-is (deferred dedup, see plan header).

- [ ] **Step 3: Keep lib.rs re-export working**

`crates/hardware/src/lib.rs:11-14` re-exports `DeviceId` from `device`. Since `device` now re-exports the fabric type, the existing `pub use device::{... DeviceId ...}` continues to compile. No change needed unless the compiler flags an ambiguous re-export; if so, change to `pub use fabric::DeviceId;` in `lib.rs`.

- [ ] **Step 4: Verify existing tests still pass**

Run: `bash scripts/cargo-agent.sh test -p hardware && bash scripts/cargo-agent.sh test -p executive --test hardware_simulation`
Expected: PASS — `DeviceId("bot".into())` in the test now constructs the fabric type transparently (identical tuple shape).

- [ ] **Step 5: Commit**

```bash
git add crates/hardware/Cargo.toml crates/hardware/src/device.rs crates/hardware/src/lib.rs
# Suggested subject: refactor(hardware): reuse fabric::DeviceId; add fabric + async-trait deps
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 3: hardware — normalized observation ingest (`observation.rs`)

**Files:**
- Create: `crates/hardware/src/observation.rs`
- Modify: `crates/hardware/src/lib.rs` (declare + re-export)
- Test: inline `#[cfg(test)]`

Responsibility: sequence-ordered ingest with dedupe + staleness, producing `fabric::EmbodiedObservation` the WorldModel/Agora can consume.

- [ ] **Step 1: Write the failing test**

Create `crates/hardware/src/observation.rs`:

```rust
//! Observation ingest: monotonic sequencing, dedupe, staleness.

use std::collections::HashMap;

use fabric::{DeviceId, EmbodiedObservation};

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::time::MonoTime;

    fn obs(source: &str, seq: u64) -> EmbodiedObservation {
        EmbodiedObservation {
            schema: "pose".into(),
            schema_version: 1,
            source: source.into(),
            sequence: seq,
            source_time: MonoTime(seq),
            received_at: MonoTime(seq),
            valid_until: None,
            confidence: 1.0,
            frame_ref: None,
            payload: serde_json::json!({}),
            evidence: vec![],
        }
    }

    #[test]
    fn rejects_out_of_order_and_duplicate_sequences() {
        let mut ingest = ObservationIngest::new();
        let dev = DeviceId("bot".into());
        assert!(ingest.accept(&dev, obs("bot", 1)).is_some());
        assert!(ingest.accept(&dev, obs("bot", 2)).is_some());
        assert!(ingest.accept(&dev, obs("bot", 2)).is_none(), "duplicate dropped");
        assert!(ingest.accept(&dev, obs("bot", 1)).is_none(), "out-of-order dropped");
        assert!(ingest.accept(&dev, obs("bot", 3)).is_some());
    }

    #[test]
    fn staleness_uses_valid_until() {
        let mut o = obs("bot", 1);
        o.valid_until = Some(fabric::types::time::MonoDeadline::after(MonoTime(1), 10));
        assert!(!is_stale(&o, MonoTime(5)));
        assert!(is_stale(&o, MonoTime(11)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p hardware observation`
Expected: FAIL — `ObservationIngest` / `is_stale` not defined.

- [ ] **Step 3: Implement**

Append to `crates/hardware/src/observation.rs`:

```rust
use fabric::types::time::MonoTime;

/// Drops duplicate and out-of-order observations per device source.
pub struct ObservationIngest {
    last_seq: HashMap<String, u64>,
}

impl Default for ObservationIngest {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservationIngest {
    pub fn new() -> Self {
        Self { last_seq: HashMap::new() }
    }

    /// Returns the observation if it advances the sequence for its source,
    /// else `None` (dropped as duplicate/out-of-order).
    pub fn accept(
        &mut self,
        _device: &DeviceId,
        obs: EmbodiedObservation,
    ) -> Option<EmbodiedObservation> {
        let key = obs.source.clone();
        let last = self.last_seq.get(&key).copied();
        if matches!(last, Some(prev) if obs.sequence <= prev) {
            return None;
        }
        self.last_seq.insert(key, obs.sequence);
        Some(obs)
    }
}

/// An observation is stale once `now` passes its `valid_until` deadline.
pub fn is_stale(obs: &EmbodiedObservation, now: MonoTime) -> bool {
    match obs.valid_until {
        Some(deadline) => deadline.is_expired_at(now),
        None => false,
    }
}
```

- [ ] **Step 4: Declare + re-export**

Edit `crates/hardware/src/lib.rs` — add `pub mod observation;` and `pub use observation::{is_stale, ObservationIngest};`.

- [ ] **Step 5: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p hardware observation`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/hardware/src/observation.rs crates/hardware/src/lib.rs
# Suggested subject: feat(hardware): observation ingest with dedupe + staleness
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 4: hardware — async `EmbodimentProvider` + skill types (`skill.rs`)

**Files:**
- Create: `crates/hardware/src/skill.rs`
- Modify: `crates/hardware/src/lib.rs`
- Test: inline `#[cfg(test)]`

The async provider **coexists** with the sync `DeviceProvider` (`provider.rs:14`). It covers long-running, cancellable skills with progress — which `apply()` cannot express.

- [ ] **Step 1: Write the failing test**

Create `crates/hardware/src/skill.rs`:

```rust
//! Async embodiment provider contract for long-running, cancellable skills.

use async_trait::async_trait;
use std::sync::Arc;

use fabric::{DeviceId, SkillDescriptor, SkillProgress, SkillRequest, SkillResult};

/// Provider error surfaced to the broker. Transport-agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Disconnected,
    Rejected(String),
    Timeout,
}

/// Sink the provider pushes progress into during a running skill.
#[async_trait]
pub trait SkillProgressSink: Send + Sync {
    async fn progress(&self, update: SkillProgress);
}

/// Cancellation acknowledgement / stop receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelAck {
    pub device: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopReceipt {
    pub device: DeviceId,
}

/// Executive-projected authority. Construction is restricted to the Hardware
/// boundary after operation/principal/device/scope/expiry checks pass.
pub struct AuthorizedSkillRequest {
    pub request: SkillRequest,
    pub permit: crate::ControlPermit,
}

/// Validated skill command marker — only the broker constructs it after
/// checking the projected permit against the request.
pub struct ValidatedSkillCommand<'a>(pub(crate) &'a AuthorizedSkillRequest);
impl<'a> ValidatedSkillCommand<'a> {
    pub fn request(&self) -> &SkillRequest {
        &self.0.request
    }
    pub fn permit(&self) -> &crate::ControlPermit {
        &self.0.permit
    }
}

#[cfg(test)]
pub(crate) fn authorized_fixture(request: SkillRequest) -> AuthorizedSkillRequest {
    let device = request.device.clone();
    let skill = request.skill.0.clone();
    AuthorizedSkillRequest {
        request,
        permit: crate::ControlPermit {
            permit_id: "test-permit".into(),
            operation: crate::OperationId(fabric::OperationId::new().0.to_string()),
            principal: crate::PrincipalId("test-principal".into()),
            device,
            scope: std::collections::BTreeSet::from([skill]),
            expires_at: crate::MonotonicInstant(u64::MAX),
            revoked: false,
        },
    }
}

#[async_trait]
pub trait EmbodimentProvider: Send + Sync {
    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError>;
    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError>;
    async fn cancel(
        &self,
        device: &DeviceId,
        operation: &crate::OperationId,
    ) -> Result<CancelAck, ProviderError>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{SkillId, SkillOutcome};

    struct RecordingSink;
    #[async_trait]
    impl SkillProgressSink for RecordingSink {
        async fn progress(&self, _update: SkillProgress) {}
    }

    struct AlwaysOk;
    #[async_trait]
    impl EmbodimentProvider for AlwaysOk {
        async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError> {
            Ok(vec![SkillDescriptor {
                skill: SkillId("wave".into()),
                device: device.clone(),
                summary: "wave hand".into(),
                input_schema: serde_json::json!({"type": "object"}),
                risk: fabric::RiskClass::Low,
                timeout_ms: 5_000,
                cancellable: true,
                preconditions: vec![],
                success_criteria: vec!["arm returned to home".into()],
            }])
        }
        async fn execute_skill(
            &self,
            command: ValidatedSkillCommand<'_>,
            progress: Arc<dyn SkillProgressSink>,
        ) -> Result<SkillResult, ProviderError> {
            let req = command.request();
            progress
                .progress(SkillProgress {
                    operation_id: command
                        .permit()
                        .operation
                        .0
                        .parse()
                        .expect("test permit operation must be a Fabric UUID"),
                    skill: req.skill.clone(),
                    fraction: 1.0,
                    note: "done".into(),
                    at: fabric::types::time::MonoTime(1),
                })
                .await;
            Ok(SkillResult {
                operation_id: command
                    .permit()
                    .operation
                    .0
                    .parse()
                    .expect("test permit operation must be a Fabric UUID"),
                skill: req.skill.clone(),
                device: req.device.clone(),
                outcome: SkillOutcome::Succeeded,
                duration_ms: 10,
                evidence: vec![],
            })
        }
        async fn cancel(
            &self,
            device: &DeviceId,
            _operation: &crate::OperationId,
        ) -> Result<CancelAck, ProviderError> {
            Ok(CancelAck { device: device.clone() })
        }
        async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError> {
            Ok(StopReceipt { device: device.clone() })
        }
    }

    #[tokio::test]
    async fn provider_executes_and_reports_success() {
        let p = AlwaysOk;
        let dev = DeviceId("bot".into());
        let skills = p.list_skills(&dev).await.unwrap();
        assert_eq!(skills[0].skill.0, "wave");
        let req = SkillRequest { skill: SkillId("wave".into()), device: dev.clone(), parameters: serde_json::json!({}) };
        let authorized = authorized_fixture(req);
        let res = p
            .execute_skill(ValidatedSkillCommand(&authorized), Arc::new(RecordingSink))
            .await
            .unwrap();
        assert_eq!(res.outcome, SkillOutcome::Succeeded);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p hardware skill`
Expected: FAIL — types not defined (before you paste the impl block the test module references them; ensure the non-test items compile first).

- [ ] **Step 3: (types already in Step 1 file) declare module**

Edit `crates/hardware/src/lib.rs` — add:

```rust
pub mod skill;
pub use skill::{
    AuthorizedSkillRequest, CancelAck, EmbodimentProvider, ProviderError,
    SkillProgressSink, StopReceipt, ValidatedSkillCommand,
};
```

The provider implementation must pass `command.permit()` to the existing
`SimulatedDevice::execute`. A provider path that calls the simulator without a permit
is a test failure, not an acceptable P1 shortcut.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p hardware skill`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/hardware/src/skill.rs crates/hardware/src/lib.rs
# Suggested subject: feat(hardware): async EmbodimentProvider + skill progress/result contract
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 5: hardware — provider/device/skill registry (`registry.rs`)

**Files:**
- Create: `crates/hardware/src/registry.rs`
- Modify: `crates/hardware/src/lib.rs`
- Test: inline

- [ ] **Step 1: Write the failing test**

Create `crates/hardware/src/registry.rs`:

```rust
//! Registry of embodiment providers keyed by device.

use std::collections::HashMap;
use std::sync::Arc;

use fabric::DeviceId;

use crate::skill::EmbodimentProvider;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup_provider() {
        let mut reg = ProviderRegistry::new();
        assert!(reg.provider(&DeviceId("bot".into())).is_none());
        // A concrete provider is registered in the executive integration test;
        // here we only assert the empty-lookup contract compiles and behaves.
        assert_eq!(reg.device_count(), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p hardware registry`
Expected: FAIL — `ProviderRegistry` not defined.

- [ ] **Step 3: Implement**

Append to `crates/hardware/src/registry.rs`:

```rust
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn EmbodimentProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self { providers: HashMap::new() }
    }

    pub fn register(&mut self, device: DeviceId, provider: Arc<dyn EmbodimentProvider>) {
        self.providers.insert(device.0, provider);
    }

    pub fn provider(&self, device: &DeviceId) -> Option<Arc<dyn EmbodimentProvider>> {
        self.providers.get(&device.0).cloned()
    }

    pub fn device_count(&self) -> usize {
        self.providers.len()
    }
}
```

- [ ] **Step 4: Declare + re-export**

Edit `crates/hardware/src/lib.rs` — add `pub mod registry;` and `pub use registry::ProviderRegistry;`.

- [ ] **Step 5: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p hardware registry`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/hardware/src/registry.rs crates/hardware/src/lib.rs
# Suggested subject: feat(hardware): provider registry keyed by device
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 6: hardware — broker validates + routes + issues receipts (`broker.rs`)

**Files:**
- Create: `crates/hardware/src/broker.rs`
- Modify: `crates/hardware/src/lib.rs`
- Test: inline

The broker takes an `AuthorizedSkillRequest`, validates operation/principal/device,
scope, expiry/revocation and the registered descriptor, routes to the provider, and
returns a normalized `SkillResult`. Authorization originates in Kernel/Executive;
the broker validates projected authority but never mints it.

- [ ] **Step 1: Write the failing test**

Create `crates/hardware/src/broker.rs`:

```rust
//! Broker: validate skill requests, route to providers, normalize results.

use std::sync::Arc;

use fabric::{DeviceId, SkillRequest, SkillResult};

use crate::registry::ProviderRegistry;
use crate::skill::{AuthorizedSkillRequest, ProviderError, SkillProgressSink, ValidatedSkillCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    UnknownDevice(String),
    NoProvider(String),
    Provider(ProviderError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::{CancelAck, EmbodimentProvider, StopReceipt};
    use async_trait::async_trait;
    use fabric::{SkillDescriptor, SkillId, SkillOutcome, SkillResult as R};

    struct Sink;
    #[async_trait]
    impl SkillProgressSink for Sink {
        async fn progress(&self, _u: fabric::SkillProgress) {}
    }

    struct P;
    #[async_trait]
    impl EmbodimentProvider for P {
        async fn list_skills(&self, _d: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError> {
            Ok(vec![])
        }
        async fn execute_skill(
            &self,
            c: ValidatedSkillCommand<'_>,
            _p: Arc<dyn SkillProgressSink>,
        ) -> Result<R, ProviderError> {
            let r = c.request();
            Ok(R {
                skill: r.skill.clone(),
                device: r.device.clone(),
                outcome: SkillOutcome::Succeeded,
                duration_ms: 1,
                evidence: vec![],
            })
        }
        async fn cancel(
            &self,
            d: &DeviceId,
            _operation: &crate::OperationId,
        ) -> Result<CancelAck, ProviderError> {
            Ok(CancelAck { device: d.clone() })
        }
        async fn safe_stop(&self, d: &DeviceId) -> Result<StopReceipt, ProviderError> {
            Ok(StopReceipt { device: d.clone() })
        }
    }

    #[tokio::test]
    async fn routes_to_registered_provider() {
        let mut reg = ProviderRegistry::new();
        reg.register(DeviceId("bot".into()), Arc::new(P));
        let broker = Broker::new(Arc::new(reg));
        let req = SkillRequest {
            skill: SkillId("wave".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
        };
        let authorized = crate::skill::authorized_fixture(req);
        let res = broker.execute(authorized, Arc::new(Sink)).await.unwrap();
        assert_eq!(res.outcome, SkillOutcome::Succeeded);
    }

    #[tokio::test]
    async fn unknown_device_fails_closed() {
        let broker = Broker::new(Arc::new(ProviderRegistry::new()));
        let req = SkillRequest {
            skill: SkillId("wave".into()),
            device: DeviceId("ghost".into()),
            parameters: serde_json::json!({}),
        };
        let authorized = crate::skill::authorized_fixture(req);
        let err = broker.execute(authorized, Arc::new(Sink)).await.unwrap_err();
        assert_eq!(err, BrokerError::NoProvider("ghost".into()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p hardware broker`
Expected: FAIL — `Broker` not defined.

- [ ] **Step 3: Implement**

Append to `crates/hardware/src/broker.rs`:

```rust
pub struct Broker {
    registry: Arc<ProviderRegistry>,
}

impl Broker {
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self { registry }
    }

    /// Route a request to its device's provider. Fails closed if no provider.
    /// NOTE: authorization (permit/lease) is validated by the Executive
    /// embodiment service BEFORE calling this; the broker never mints authority.
    pub async fn execute(
        &self,
        authorized: AuthorizedSkillRequest,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, BrokerError> {
        let device = authorized.request.device.clone();
        validate_projected_authority(&authorized)?;
        let provider = self
            .registry
            .provider(&device)
            .ok_or_else(|| BrokerError::NoProvider(device.0.clone()))?;
        let command = ValidatedSkillCommand(&authorized);
        provider
            .execute_skill(command, progress)
            .await
            .map_err(BrokerError::Provider)
    }

    pub async fn list_skills(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<fabric::SkillDescriptor>, BrokerError> {
        let provider = self
            .registry
            .provider(device)
            .ok_or_else(|| BrokerError::NoProvider(device.0.clone()))?;
        provider.list_skills(device).await.map_err(BrokerError::Provider)
    }

    pub async fn cancel(
        &self,
        device: &DeviceId,
        operation: &crate::OperationId,
    ) -> Result<(), BrokerError> {
        let provider = self
            .registry
            .provider(device)
            .ok_or_else(|| BrokerError::NoProvider(device.0.clone()))?;
        provider
            .cancel(device, operation)
            .await
            .map(|_| ())
            .map_err(BrokerError::Provider)
    }

    pub async fn safe_stop(&self, device: &DeviceId) -> Result<(), BrokerError> {
        let provider = self
            .registry
            .provider(device)
            .ok_or_else(|| BrokerError::NoProvider(device.0.clone()))?;
        provider.safe_stop(device).await.map(|_| ()).map_err(BrokerError::Provider)
    }
}
```

Add `validate_projected_authority` beside `Broker`: it must compare the request device
with `ControlPermit.device`, compare the Executive-projected Fabric operation string
with `ControlPermit.operation`, require the skill in permit scope, and reject expired or
revoked permits. Add one test for each rejection class plus a test proving the provider
is not called when validation fails.

- [ ] **Step 4: Declare + re-export**

Edit `crates/hardware/src/lib.rs` — add `pub mod broker;` and `pub use broker::{Broker, BrokerError};`.

- [ ] **Step 5: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p hardware broker`
Expected: PASS (both routing and fail-closed).

- [ ] **Step 6: Commit**

```bash
git add crates/hardware/src/broker.rs crates/hardware/src/lib.rs
# Suggested subject: feat(hardware): broker routes validated skill requests to providers
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 7: hardware — `SimulatedDevice` implements `EmbodimentProvider`

**Files:**
- Modify: `crates/hardware/src/simulator.rs`
- Test: inline `#[cfg(test)]` in `simulator.rs`

Reuse the device's existing navigate/stop mechanics. The async impl wraps a `tokio::sync::Mutex<SimulatedDevice>` so the provider is `Send + Sync`.

- [ ] **Step 1: Write the failing test**

Add to `crates/hardware/src/simulator.rs` test module:

```rust
#[cfg(test)]
mod embodiment_tests {
    use super::*;
    use crate::skill::{EmbodimentProvider, SkillProgressSink, ValidatedSkillCommand};
    use fabric::{DeviceId, SkillId, SkillOutcome, SkillRequest};
    use std::sync::Arc;

    struct NullSink;
    #[async_trait::async_trait]
    impl SkillProgressSink for NullSink {
        async fn progress(&self, _u: fabric::SkillProgress) {}
    }

    #[tokio::test]
    async fn simulated_provider_executes_navigate_skill() {
        let clock = Arc::new(ManualClock::new(0));
        let provider = SimulatedEmbodiment::mobile_robot("bot", clock);
        let req = SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 2.0, "y": 3.0}),
        };
        let authorized = crate::skill::authorized_fixture(req);
        let res = provider
            .execute_skill(ValidatedSkillCommand(&authorized), Arc::new(NullSink))
            .await
            .unwrap();
        assert_eq!(res.outcome, SkillOutcome::Succeeded);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p hardware embodiment_tests`
Expected: FAIL — `SimulatedEmbodiment` not defined.

- [ ] **Step 3: Implement the async wrapper**

Append to `crates/hardware/src/simulator.rs`:

```rust
use crate::skill::{
    CancelAck, EmbodimentProvider, ProviderError, SkillProgressSink, StopReceipt,
    ValidatedSkillCommand,
};
use fabric::{SkillOutcome, SkillProgress, SkillResult};

/// Async `EmbodimentProvider` facade over a `SimulatedDevice`.
pub struct SimulatedEmbodiment {
    inner: tokio::sync::Mutex<SimulatedDevice>,
}

impl SimulatedEmbodiment {
    pub fn mobile_robot(id: &str, clock: Arc<dyn MonotonicClock>) -> Self {
        Self { inner: tokio::sync::Mutex::new(SimulatedDevice::mobile_robot(id, clock)) }
    }
}

#[async_trait::async_trait]
impl EmbodimentProvider for SimulatedEmbodiment {
    async fn list_skills(
        &self,
        device: &fabric::DeviceId,
    ) -> Result<Vec<fabric::SkillDescriptor>, ProviderError> {
        Ok(vec![fabric::SkillDescriptor {
            skill: fabric::SkillId("navigate".into()),
            device: device.clone(),
            summary: "navigate to (x,y)".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "number"}, "y": {"type": "number"}},
                "required": ["x", "y"]
            }),
            risk: fabric::RiskClass::Medium,
            timeout_ms: 30_000,
            cancellable: true,
            preconditions: vec!["battery > 0".into()],
            success_criteria: vec!["position == target".into()],
        }])
    }

    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError> {
        let req = command.request();
        let x = req.parameters.get("x").and_then(|v| v.as_f64())
            .ok_or_else(|| ProviderError::Rejected("navigate x missing".into()))?;
        let y = req.parameters.get("y").and_then(|v| v.as_f64())
            .ok_or_else(|| ProviderError::Rejected("navigate y missing".into()))?;
        let mut guard = self.inner.lock().await;
        // Reuse the device's validated apply path via a synthesized TypedCommand.
        let cmd = crate::TypedCommand {
            command_id: "skill-navigate".into(),
            operation: crate::OperationId("skill-op".into()),
            principal: crate::PrincipalId("embodiment".into()),
            sequence: crate::CommandSequence(1),
            device: req.device.clone(),
            schema: "navigate".into(),
            payload: serde_json::json!({"x": x, "y": y}),
            deadline: crate::MonotonicInstant(u64::MAX),
        };
        let receipt = guard.execute(&cmd, Some(command.permit()));
        let outcome = if receipt.accepted() {
            SkillOutcome::Succeeded
        } else {
            SkillOutcome::Failed { reason: format!("{:?}", receipt.decision) }
        };
        progress
            .progress(SkillProgress {
                operation_id: command
                    .permit()
                    .operation
                    .0
                    .parse()
                    .map_err(|_| ProviderError::Rejected("invalid operation mapping".into()))?,
                skill: req.skill.clone(),
                fraction: 1.0,
                note: "navigate settled".into(),
                at: fabric::types::time::MonoTime(0),
            })
            .await;
        Ok(SkillResult {
            operation_id: command
                .permit()
                .operation
                .0
                .parse()
                .map_err(|_| ProviderError::Rejected("invalid operation mapping".into()))?,
            skill: req.skill.clone(),
            device: req.device.clone(),
            outcome,
            duration_ms: 0,
            evidence: vec![],
        })
    }

    async fn cancel(
        &self,
        device: &fabric::DeviceId,
        _operation: &crate::OperationId,
    ) -> Result<CancelAck, ProviderError> {
        Ok(CancelAck { device: device.clone() })
    }

    async fn safe_stop(&self, device: &fabric::DeviceId) -> Result<StopReceipt, ProviderError> {
        let mut guard = self.inner.lock().await;
        guard.safe_stop().map_err(|e| ProviderError::Rejected(e))?;
        Ok(StopReceipt { device: device.clone() })
    }
}
```

The provider must never use a permit-free simulation shortcut. Add a negative test that
constructs a mismatched permit and proves `SimulatedDevice` rejects it before any
successful progress/result is emitted.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p hardware embodiment_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/hardware/src/simulator.rs crates/hardware/src/lib.rs
# Suggested subject: feat(hardware): SimulatedEmbodiment async provider over SimulatedDevice
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 8: executive — promote hardware to prod dep + implement the fabric port

**Files:**
- Modify: `crates/executive/Cargo.toml:48` (move `hardware` to `[dependencies]`)
- Create: `crates/executive/src/service/embodiment_service.rs`
- Create: `crates/executive/src/service/embodiment_authority.rs`
- Create: `crates/executive/src/service/embodiment_progress.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Test: `crates/executive/tests/embodiment_service.rs`

- [ ] **Step 1: Move the dependency**

Edit `crates/executive/Cargo.toml` — remove `hardware = { path = "../hardware" }` from `[dev-dependencies]` (line 48) and add it under `[dependencies]`.

- [ ] **Step 2: Write the failing test**

Create `crates/executive/tests/embodiment_service.rs`:

```rust
use std::sync::Arc;

use executive::service::embodiment_service::EmbodimentService;
use fabric::EmbodimentExecutionPort;
use fabric::{DeviceId, SkillId, SkillOutcome, SkillRequest};
use hardware::{Broker, ProviderRegistry};
use hardware::simulator::SimulatedEmbodiment;

#[tokio::test]
async fn service_executes_navigate_through_broker() {
    let clock = Arc::new(hardware::ManualClock::new(0));
    let mut reg = ProviderRegistry::new();
    reg.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock)),
    );
    let broker = Arc::new(Broker::new(Arc::new(reg)));
    let authority = Arc::new(TestEmbodimentAuthority::allow("bot", "navigate"));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let service = EmbodimentService::new(broker, authority, progress.clone());

    let req = SkillRequest {
        skill: SkillId("navigate".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({"x": 1.0, "y": 1.0}),
    };
    let result = service.execute_skill(req).await.unwrap();
    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    assert_eq!(progress.operation_ids(), vec![result.operation_id]);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p executive --test embodiment_service`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement the service against the fabric-owned narrow port**

Create `crates/executive/src/service/embodiment_service.rs`:

```rust
//! Executive-side embodiment orchestration. Executive owns operation creation,
//! Kernel admission/lease projection, progress bounds, cancellation and settlement.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{EmbodimentExecutionPort, SkillDispatchError, SkillRequest, SkillResult};
use hardware::skill::SkillProgressSink;
use hardware::{AuthorizedSkillRequest, Broker, BrokerError};

use super::embodiment_authority::EmbodimentAuthorityPort;
use super::embodiment_progress::{BoundedProgressSink, EmbodimentProgressPort};

pub struct EmbodimentService {
    broker: Arc<Broker>,
    authority: Arc<dyn EmbodimentAuthorityPort>,
    progress: Arc<dyn EmbodimentProgressPort>,
}

impl EmbodimentService {
    pub fn new(
        broker: Arc<Broker>,
        authority: Arc<dyn EmbodimentAuthorityPort>,
        progress: Arc<dyn EmbodimentProgressPort>,
    ) -> Self {
        Self { broker, authority, progress }
    }
}

#[async_trait]
impl EmbodimentExecutionPort for EmbodimentService {
    async fn execute_skill(&self, request: SkillRequest) -> Result<SkillResult, SkillDispatchError> {
        let operation_id = fabric::OperationId::new();
        let authorized: AuthorizedSkillRequest = self
            .authority
            .authorize(operation_id, &request)
            .await?;
        let sink = Arc::new(BoundedProgressSink::new(
            operation_id,
            self.progress.clone(),
            64,
        ));
        self.broker
            .execute(authorized, sink)
            .await
            .map_err(|error| match error {
                BrokerError::UnknownDevice(id) | BrokerError::NoProvider(id) => {
                    SkillDispatchError::NoProvider(id)
                }
                BrokerError::Provider(error) => {
                    SkillDispatchError::Rejected(format!("{error:?}"))
                }
            })
    }
}
```

`EmbodimentAuthorityPort::authorize` is implemented in production by a narrow adapter
around Kernel admission and lease projection; its test implementation returns explicit
fixtures. `BoundedProgressSink` attaches the host operation ID, drops/coalesces beyond
the bound of 64, and forwards normalized candidates to the injected progress port.
Neither implementation reads environment state. Add deterministic tests for rejected
admission, lease expiry, timeout-vs-cancel ordering, progress overflow, provider
disconnect, and exactly-once terminal settlement.

- [ ] **Step 5: Declare the module**

Edit `crates/executive/src/service/mod.rs` — add `pub mod embodiment_service;` in alpha order.

- [ ] **Step 6: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p executive --test embodiment_service`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/executive/Cargo.toml crates/executive/src/service/embodiment_service.rs crates/executive/src/service/mod.rs crates/executive/tests/embodiment_service.rs
# Suggested subject: feat(executive): EmbodimentService + narrow EmbodimentExecutionPort over hardware broker
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 9: executive — assemble `EmbodimentServices` at bootstrap

**Files:**
- Create: `crates/executive/src/impl/daemon/bootstrap/embodiment.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/mod.rs` (declare)
- Test: inline `#[cfg(test)]` in `embodiment.rs`

Keep this OUT of `TurnPipelineResources` / `request.rs`. The composer builds a registry (with the simulator in the Simulation namespace by default) and returns an `Arc<dyn EmbodimentExecutionPort>`.

- [ ] **Step 1: Write the failing test**

Create `crates/executive/src/impl/daemon/bootstrap/embodiment.rs`:

```rust
//! Assembles the embodiment service group. Default namespace: Simulation.

use std::sync::Arc;

use crate::service::embodiment_service::EmbodimentService;
use crate::service::embodiment_authority::EmbodimentAuthorityPort;
use crate::service::embodiment_progress::EmbodimentProgressPort;
use fabric::EmbodimentExecutionPort;
use hardware::simulator::SimulatedEmbodiment;
use hardware::{Broker, ProviderRegistry};

/// Build the default (simulation) embodiment port. Production namespaces must
/// be configured explicitly (spec §P5); this is the P1 simulator default.
pub fn build_embodiment_port(
    clock: Arc<dyn hardware::MonotonicClock>,
    authority: Arc<dyn EmbodimentAuthorityPort>,
    progress: Arc<dyn EmbodimentProgressPort>,
) -> Arc<dyn EmbodimentExecutionPort> {
    let mut registry = ProviderRegistry::new();
    registry.register(
        fabric::DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock)),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry)));
    Arc::new(EmbodimentService::new(broker, authority, progress))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_port_runs_simulator() {
        let clock = Arc::new(hardware::ManualClock::new(0));
        let authority = Arc::new(
            crate::service::embodiment_authority::TestEmbodimentAuthority::allow(
                "bot", "navigate",
            ),
        );
        let progress = Arc::new(
            crate::service::embodiment_progress::RecordingEmbodimentProgress::default(),
        );
        let port = build_embodiment_port(clock, authority, progress);
        let res = port
            .execute_skill(fabric::SkillRequest {
                skill: fabric::SkillId("navigate".into()),
                device: fabric::DeviceId("bot".into()),
                parameters: serde_json::json!({"x": 0.0, "y": 0.0}),
            })
            .await
            .unwrap();
        assert_eq!(res.outcome, fabric::SkillOutcome::Succeeded);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p executive default_port_runs_simulator`
Expected: FAIL — module not declared.

- [ ] **Step 3: Declare the module**

Edit `crates/executive/src/impl/daemon/bootstrap/mod.rs` — add `pub(crate) mod embodiment;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p executive default_port_runs_simulator`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/executive/src/impl/daemon/bootstrap/embodiment.rs crates/executive/src/impl/daemon/bootstrap/mod.rs
# Suggested subject: feat(executive): assemble default (simulation) embodiment port at bootstrap
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 10: corpus — six narrow robot tools

**Files:**
- Create: `crates/corpus/src/tools/tools/robot.rs`
- Modify: `crates/corpus/src/tools/tools/registry.rs` (register)
- Modify: `crates/corpus/src/tools/tools/mod.rs` (declare module)
- Test: inline `#[cfg(test)]`

Tools mirror `SystemStatusTool` (`system_status.rs`). They are backed by an injected `EmbodimentExecutionPort`; they never call ROS or manage leases. Explicitly excluded: `publish_topic`, `call_any_service`, `set_joint`, `raw_bus_write`.

> **Resolved dependency rule:** `EmbodimentExecutionPort` is defined in `fabric` in Task 1 and implemented by executive in Task 8. Corpus holds `Arc<dyn fabric::EmbodimentExecutionPort>` and never depends on executive or hardware.

- [ ] **Step 1: Extend the fabric-owned port for the complete six-tool surface**

Task 1 establishes `execute_skill`. Before writing Corpus adapters, extend that same
trait in `crates/fabric/src/types/embodiment.rs` with the remaining governed operations:

```rust
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillDispatchError {
    NoProvider(String),
    Rejected(String),
}

#[async_trait]
pub trait EmbodimentExecutionPort: Send + Sync {
    async fn observe(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, SkillDispatchError>;
    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, SkillDispatchError>;
    async fn list_skills(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, SkillDispatchError>;
    async fn execute_skill(&self, request: SkillRequest) -> Result<SkillResult, SkillDispatchError>;
    async fn cancel(&self, operation_id: &OperationId) -> Result<(), SkillDispatchError>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<(), SkillDispatchError>;
}
```

Task 1 adds `async-trait` to `crates/fabric/Cargo.toml` if absent and re-exports
the port/error. In this step, update `EmbodimentService` and its test double to
implement every added method by delegating to the observation ingest, provider
registry, and broker from Tasks 3–7. No default method or success sentinel is allowed.
The trait remains in Fabric; Task 10 extends but does not move or redefine its authority.

- [ ] **Step 2: Write the failing test**

Create `crates/corpus/src/tools/tools/robot.rs`:

```rust
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// `robot.execute_skill` — the only tool that commands motion. Selection is
/// restricted to manifest-registered skills; raw topic/joint access is never
/// exposed.
pub struct RobotExecuteSkillTool {
    port: Arc<dyn fabric::EmbodimentExecutionPort>,
}

impl RobotExecuteSkillTool {
    pub fn new(port: Arc<dyn fabric::EmbodimentExecutionPort>) -> Self {
        Self { port }
    }
}

#[async_trait]
impl Tool for RobotExecuteSkillTool {
    fn name(&self) -> &str {
        "robot.execute_skill"
    }
    fn description(&self) -> &str {
        "Execute a registered robot skill by id with JSON parameters"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "device": {"type": "string"},
                "skill": {"type": "string"},
                "parameters": {"type": "object"}
            },
            "required": ["device", "skill"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L2
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(RobotExecuteSkillTool { port: self.port.clone() })
    }
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let device = input.get("device").and_then(|v| v.as_str()).unwrap_or_default();
        let skill = input.get("skill").and_then(|v| v.as_str()).unwrap_or_default();
        let parameters = input.get("parameters").cloned().unwrap_or_else(|| json!({}));
        let req = fabric::SkillRequest {
            skill: fabric::SkillId(skill.into()),
            device: fabric::DeviceId(device.into()),
            parameters,
        };
        let (content, is_error) = match self.port.execute_skill(req).await {
            Ok(result) => (serde_json::to_string(&result).unwrap_or_default(), false),
            Err(e) => (format!("skill dispatch failed: {e:?}"), true),
        };
        ToolResult {
            content,
            is_error,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OkPort;
    #[async_trait]
    impl fabric::EmbodimentExecutionPort for OkPort {
        async fn observe(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Vec<fabric::EmbodiedObservation>, fabric::SkillDispatchError> {
            Ok(vec![])
        }
        async fn get_state(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Option<fabric::EmbodiedObservation>, fabric::SkillDispatchError> {
            Ok(None)
        }
        async fn list_skills(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Vec<fabric::SkillDescriptor>, fabric::SkillDispatchError> {
            Ok(vec![])
        }
        async fn execute_skill(
            &self,
            request: fabric::SkillRequest,
        ) -> Result<fabric::SkillResult, fabric::SkillDispatchError> {
            Ok(fabric::SkillResult {
                operation_id: fabric::OperationId::new(),
                skill: request.skill,
                device: request.device,
                outcome: fabric::SkillOutcome::Succeeded,
                duration_ms: 1,
                evidence: vec![],
            })
        }
        async fn cancel(
            &self,
            _operation_id: &fabric::OperationId,
        ) -> Result<(), fabric::SkillDispatchError> {
            Ok(())
        }
        async fn safe_stop(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<(), fabric::SkillDispatchError> {
            Ok(())
        }
    }

    #[test]
    fn tool_name_and_schema_are_narrow() {
        let tool = RobotExecuteSkillTool::new(Arc::new(OkPort));
        assert_eq!(tool.name(), "robot.execute_skill");
        assert_eq!(tool.permission_level(), PermissionLevel::L2);
        // No raw-control affordances exposed.
        let schema = tool.input_schema().to_string();
        assert!(!schema.contains("topic"));
        assert!(!schema.contains("joint"));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p corpus robot`
Expected: FAIL — module not declared / trait path wrong.

- [ ] **Step 4: Declare the module + add remaining five tools**

Edit `crates/corpus/src/tools/tools/mod.rs` — add `pub mod robot;`. In `robot.rs`,
add the five siblings following the same pattern (each a struct holding the port,
JSON-in/JSON-out): `RobotObserveTool` (`robot.observe`), `RobotGetStateTool`
(`robot.get_state`), `RobotListSkillsTool` (`robot.list_skills`), `RobotCancelTool`
(`robot.cancel`), and `RobotSafeStopTool` (`robot.safe_stop`). Each tool must call
the corresponding Task 1 port method and propagate `SkillDispatchError`; every tool must return actual provider state; success sentinels are forbidden. Read-only tools use L0. Execute, cancel, and
model-requested safe-stop use L2 and remain subject to Executive/Kernel admission.
Internal lease-expiry/provider-disconnect fail-safe bypasses the model tool surface and
remains available even if the cognitive session dies.

> Keep each adapter focused and testable. Do not invent new permission levels: use
> L0 for observation/query and L2 for execute/cancel/model-requested safe-stop. Kernel
> admission remains authoritative after Corpus classification.

Use this exact public matrix; tests must assert every row:

| Tool | Required JSON | Port call | Permission | Concurrency |
|---|---|---|---|---|
| `robot.observe` | `device` | `observe(DeviceId)` | L0 | ReadOnly |
| `robot.get_state` | `device` | `get_state(DeviceId)` | L0 | ReadOnly |
| `robot.list_skills` | `device` | `list_skills(DeviceId)` | L0 | ReadOnly |
| `robot.execute_skill` | `device`, `skill`, optional `parameters` | `execute_skill(SkillRequest)` | L2 | SideEffect |
| `robot.cancel` | `operation_id` | parse UUID, then `cancel(OperationId)` | L2 | SideEffect |
| `robot.safe_stop` | `device` | `safe_stop(DeviceId)` | L2 | SideEffect |

For every adapter, reject missing/empty required strings before calling the port.
`operation_id` must parse through the canonical Fabric constructor/parser; never pass
unvalidated model text to Executive. Serialization failure is an error result rather
than an empty successful payload.

- [ ] **Step 4a: Complete the port/service contract tests before registering tools**

Extend `crates/executive/tests/embodiment_service.rs` with a recording Hardware
fixture and prove all six methods delegate to the intended device/operation. Add
negative tests for unknown device, invalid operation mapping, expired/revoked permit,
provider disconnect, and observation staleness. The test double shown above implements
all six methods so Corpus tests remain compile-complete after the trait extension.

- [ ] **Step 4b: Add the Corpus permission and schema matrix test**

The `robot.rs` test module must construct all six tools and assert their names,
permissions, concurrency classes, required JSON fields, and absence of the forbidden
keys `topic`, `service`, `joint`, and `bus`. It must also invoke each tool once against
a recording port and assert exactly one matching port call.

- [ ] **Step 5: Register the tools**

Edit `crates/corpus/src/tools/tools/registry.rs` — the registry needs the injected port. Add a registration method mirroring `with_network_policy_and_search`:

```rust
pub fn register_robot_tools(&mut self, port: std::sync::Arc<dyn fabric::EmbodimentExecutionPort>) {
    self.register(Arc::new(super::robot::RobotExecuteSkillTool::new(port.clone())))
        .expect("duplicate robot tool");
    self.register(Arc::new(super::robot::RobotSafeStopTool::new(port.clone())))
        .expect("duplicate robot tool");
    self.register(Arc::new(super::robot::RobotCancelTool::new(port.clone())))
        .expect("duplicate robot tool");
    self.register(Arc::new(super::robot::RobotListSkillsTool::new(port.clone())))
        .expect("duplicate robot tool");
    self.register(Arc::new(super::robot::RobotGetStateTool::new(port.clone())))
        .expect("duplicate robot tool");
    self.register(Arc::new(super::robot::RobotObserveTool::new(port)))
        .expect("duplicate robot tool");
}
```

Call `register_robot_tools(embodiment_port)` where the daemon builds its runtime registry (find via `rg -n "with_network_policy_and_search" crates/executive/src`), passing the port from Task 9's `build_embodiment_port`.

- [ ] **Step 6: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p corpus robot`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/fabric/src/types/embodiment.rs crates/fabric/Cargo.toml crates/fabric/src/lib.rs crates/executive/src/service/embodiment_service.rs crates/corpus/src/tools/tools/robot.rs crates/corpus/src/tools/tools/mod.rs crates/corpus/src/tools/tools/registry.rs
# Suggested subject: feat(corpus): six narrow governed robot tools backed by fabric EmbodimentExecutionPort
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 11: end-to-end production-path integration test

**Files:**
- Create: `crates/executive/tests/embodiment_production_path.rs`

Mirrors `hardware_simulation.rs` authorization (Kernel admission → permit) but drives the skill through `EmbodimentService` rather than a hand-built device call — proving the production path, not a manual test.

- [ ] **Step 1: Write the test**

Create `crates/executive/tests/embodiment_production_path.rs`:

```rust
use std::sync::Arc;

use executive::service::embodiment_authority::KernelEmbodimentAuthority;
use executive::service::embodiment_progress::RecordingEmbodimentProgress;
use executive::service::embodiment_service::EmbodimentService;
use fabric::{
    DeviceId, EmbodimentExecutionPort, PrincipalId, ProcessId, SkillId,
    SkillOutcome, SkillRequest,
};
use hardware::simulator::SimulatedEmbodiment;
use hardware::{Broker, ManualClock, ProviderRegistry};
use kernel::chronos::TestClock;

#[tokio::test]
async fn model_skill_is_authorized_executed_and_settled() {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(0, 100))));
    let authority = Arc::new(KernelEmbodimentAuthority::new(
        kernel,
        PrincipalId("operator".into()),
        ProcessId::new(),
    ));
    let progress = Arc::new(RecordingEmbodimentProgress::default());

    let hardware_clock = Arc::new(ManualClock::new(100));
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", hardware_clock)),
    );
    let service = EmbodimentService::new(
        Arc::new(Broker::new(Arc::new(registry))),
        authority,
        progress.clone(),
    );

    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 2.0, "y": 3.0}),
        })
        .await
        .unwrap();

    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    assert_eq!(result.device, DeviceId("bot".into()));
    assert_eq!(progress.operation_ids(), vec![result.operation_id]);
    assert!(service.settlement_for(result.operation_id).await.unwrap().is_terminal());
}
```

- [ ] **Step 2: Run test**

Run: `bash scripts/cargo-agent.sh test -p executive --test embodiment_production_path`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/executive/tests/embodiment_production_path.rs
# Suggested subject: test(executive): end-to-end embodiment production path (authorize→execute→settle)
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 12: full regression + completion check

- [ ] **Step 1: Run the P1 acceptance set**

```bash
bash scripts/cargo-agent.sh test -p fabric embodiment
bash scripts/cargo-agent.sh test -p hardware
bash scripts/cargo-agent.sh test -p corpus robot
bash scripts/cargo-agent.sh test -p executive --test embodiment_service
bash scripts/cargo-agent.sh test -p executive --test embodiment_production_path
bash scripts/cargo-agent.sh test -p executive --test hardware_simulation
bash scripts/architecture-check.sh
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh build --workspace
```

Expected: all PASS + clean build.

- [ ] **Step 2: Verify dependency discipline**

```bash
# hardware no longer dev-dep-only; corpus depends on fabric, not executive
rg -n "hardware" crates/executive/Cargo.toml
rg -n "executive" crates/corpus/Cargo.toml   # expect: no match
```

- [ ] **Step 3: Commit fixups**

```bash
git status --short
# Stage only files owned by this task.
# Suggested subject: test(executive): P1 hardware vertical slice acceptance green
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Completion criteria (maps to spec §5.6)

- A tool call (`robot.execute_skill`) → Kernel authorization → Hardware broker → simulator provider → normalized `SkillResult`, exercised by `embodiment_production_path.rs`.
- `hardware` is a production dependency of `executive`; `corpus` depends on `fabric` (not `executive`).
- Provider is swappable: replacing `SimulatedEmbodiment` with a future ROS bridge changes only composition registration/configuration, not Cognit, Corpus, TurnPipeline, or `EmbodimentService` business logic.
- Only registered skills are exposed to the model; no `publish_topic`/`set_joint`/`raw_bus_write`.
- The six-tool permission matrix is enforced: three L0 query tools and three L2 control tools.
- Internal lease/provider fail-safe remains callable without a live model turn and is tested separately from `robot.safe_stop`.
- Every progress/result/settlement record carries the one host-generated Fabric operation ID.

## Known risks carried into implementation

- **Task 7 permit gating:** permit-free navigate is forbidden. The provider must pass the Broker-validated projected permit to `SimulatedDevice::execute`, with mismatch/expiry/revocation tests.
- **Port authority:** `EmbodimentExecutionPort` lives in `fabric` from Task 1 onward; no later task moves or redefines it. Provider/broker/registry contracts stay in `hardware`.
- **Task 9 injection into the live daemon registry:** keep the wiring as its own P1-D stage commit if bootstrap is non-trivial, but do not declare P1 complete until the live registry contains the six governed tools and the production-path test exercises that registry.
- **ID dedup deferred:** `hardware::{OperationId,PrincipalId,MonotonicInstant}` vs fabric remains a separate follow-up (see plan header).
