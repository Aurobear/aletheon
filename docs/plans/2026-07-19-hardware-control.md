# Hardware Control Platform Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an independent, production-grade Hardware Control Platform for Aletheon: stable device identity, typed commands, telemetry schemas, control leases, and a non-bypassable safety model. Sim-first — the deterministic simulator is the *executable specification* of the safety model. No LLM ever writes raw bytes to a bus; every actuation flows through schema validation, policy, lease arbitration, deadline/sequence checks, and a local Safety Supervisor that stays authoritative even when Aletheon is offline.
**Architecture:** Three-layer control plane, transport-agnostic core. `hardware-api` (domain types, traits, state machines, error taxonomy — zero transport deps) ← `hardware-broker` (registry, provider lifecycle, lease arbitration, schema/policy validation, sequence/deadline guards, telemetry QoS + Agora/Artifact fan-out) ← providers (`hardware-sim` first; later `hardware-ros2`/`hardware-serial`/`hardware-can`/`hardware-gpio`). Executive/Cognit reach hardware only through **Kernel Capability** (`hardware.observe:` / `hardware.command:` / `hardware.stop:` / `hardware.calibrate:` / `hardware.admin:`) and governed Host primitives — never provider-native APIs. Real-time control, drivers, and final safety adjudication live in the device/robot Edge Runtime, outside this platform.
**Tech Stack:** Rust; later ROS 2 (rclrs/DDS), SocketCAN, serialport, Linux GPIO chardev v2.
**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，不要用裸 cargo。
**依赖:** 独立于 coding-agent 线；建议 coding 线 Wave 0-4 稳定后启动。仅通过 Kernel Capability 与 Host 原语连接。
---

## Grounding: current state (confirms the gap)

- **String-based transport only.** `crates/fabric/src/include/body.rs:18-28` — `Action { name: String, parameters: serde_json::Value, requires_sandbox: bool, timeout: Option<Duration> }`. This is a generic Runtime envelope: no stable device identity, no typed command schema, no telemetry schema, no lease, no monotonic deadline, no sequence number, no safety state. It cannot statically forbid a dangerous actuation.
- **No hardware domain types exist.** `grep -rn "DeviceId|DeviceManifest|HardwareBroker" crates/` → zero matches. `Cargo.toml:3-18` workspace `members` has no `hardware-*` crate.
- **Capability model to bind against.** `crates/fabric/src/types/capability.rs:30-37` `Capability { name, level: CapabilityLevel, description }`; `CapabilityLevel` (`crates/fabric/src/types/capability.rs:13-26`) = ReadOnly→SandboxWrite→SystemChange→Destructive→SelfModify. Invocation authority is `CapabilityAuthority { action, requested_scope: CapabilityScope, risk: RiskLevel, lease: Option<LeaseRequest>, sandbox, ... }` (`crates/fabric/src/include/turn.rs:52-66`). `CapabilityScope { allowed_paths, allowed_targets, max_runtime_ms, max_output_bytes }` (`crates/fabric/src/types/admission.rs:50-59`). The invoker is `DefaultCapabilityInvoker` (`crates/kernel/src/capability/mod.rs:25`).
- **Existing lease primitive (reuse the shape, not the semantics).** `crates/kernel/src/admission/lease.rs` `InMemoryResourceLeaseManager` — exclusive-by-default, time-limited, explicit release or expiry; fabric side `LeaseRequest`/`ResourceLeaseId` (`crates/fabric/src/types/admission.rs:200,216`). Hardware `ControlLease` needs richer semantics (deadman heartbeat, fail-safe on disconnect, capability-scoped mode) so it lives in `hardware-api`, but D0 leases can delegate TTL bookkeeping to this manager where convenient.
- **Monotonic time already present.** `crates/fabric/src/types/time.rs` exports `MonoTime`/`MonoDeadline` (`crates/fabric/src/lib.rs:297`); kernel `chronos` clock (`crates/kernel/src/chronos/system_clock.rs`). Use `MonoTime` as the `MonotonicInstant` in command/telemetry/lease types; sim supplies a fixed-seed deterministic clock.

**Conclusion:** adding a few `Action.name`s to `BodyRuntime` cannot claim hardware support. The three new crates below are required.

---

## D0 — Hardware API + deterministic simulator (kernel capability observe-only)

**D0 concrete target = the minimal vertical slice** (source doc §13): a virtual mobile robot in `hardware-sim` driven end-to-end through `hardware-broker`:

```text
Simulator virtual mobile robot
  -> pose/battery/health telemetry (TelemetryEnvelope stream)
  -> acquire / renew / release ControlLease (deadman heartbeat)
  -> typed NavigateTo + ControlledStop (schema-validated TypedCommand)
  -> deadline + sequence + CommandReceipt
  -> disconnect / lease-expiry fault injection -> fail-safe (SafeHold)
  -> Agora summary + Artifact event log
```

Build only what this slice needs. Do NOT implement ROS/serial/CAN/GPIO, calibration, firmware, or multi-device federation in D0.

### D0.1 — Scaffold the three crates

- [ ] Create `crates/hardware-api/` (`Cargo.toml`, `src/lib.rs`). Deps: `serde`, `uuid`, `thiserror`, `async-trait`, and `fabric` **only** for `MonoTime`/`MonoDeadline` (no ROS/serial/CAN/vendor deps, ever — enforce in D0.7 architecture gate).
- [ ] Create `crates/hardware-broker/` (`Cargo.toml`, `src/lib.rs`). Deps: `hardware-api`, `tokio`, `tracing`, `serde_json`, `fabric` (for Capability/Agora/Artifact glue).
- [ ] Create `crates/hardware-sim/` (`Cargo.toml`, `src/lib.rs`). Deps: `hardware-api`, `tokio`, `rand` (seeded), `tracing`.
- [ ] Register all three in `Cargo.toml:3-18` workspace `members`. Stage `Cargo.lock` alongside (tracked; drift is silent — see MEMORY "Commit Cargo.lock with deps").
- [ ] Verify: `bash scripts/cargo-agent.sh test -p hardware-api` (empty crate compiles).

### D0.2 — Core identity + manifest types (`hardware-api`)

**Files:** `crates/hardware-api/src/device.rs`, `crates/hardware-api/src/lib.rs`

Reuse the source doc §5 types verbatim:

```rust
pub struct DeviceId(uuid::Uuid);              // stable identity, NOT /dev/ttyUSB0 or a ROS node name
pub struct DeviceUri { pub namespace: DeviceNamespace, pub provider: ProviderId, pub path: Vec<String> }
pub enum DeviceNamespace { Simulation, Lab, Production }   // never share default write perms across these
pub enum DeviceClass { Robot, Actuator, Sensor, Camera, Audio, ComputeAccelerator, Bus, Composite }

pub struct DeviceManifest {
    pub id: DeviceId, pub uri: DeviceUri, pub class: DeviceClass,
    pub model: String, pub firmware: Option<String>,
    pub capabilities: Vec<DeviceCapability>,
    pub command_schemas: Vec<CommandSchemaRef>,
    pub telemetry_schemas: Vec<TelemetrySchemaRef>,
    pub safety_profile: SafetyProfileRef,
    pub calibration: CalibrationState,        // D0: enum { Unknown } stub
    pub trust: DeviceTrust,
}
```

- [ ] Define `DeviceId`, `DeviceUri`, `DeviceNamespace`, `DeviceClass`, `ProviderId`, `DeviceManifest`, and stub refs (`DeviceCapability`, `CommandSchemaRef`, `TelemetrySchemaRef`, `SafetyProfileRef`, `CalibrationState`, `DeviceTrust`). Derive `Debug, Clone, Serialize, Deserialize, PartialEq`.
- [ ] Constructor `DeviceUri::to_capability_path()` → `"/robot/sim/mobile-01/navigation"` form, for binding to Kernel `hardware.command:/...` (source doc §10.2).
- [ ] Unit test: `DeviceUri` round-trips to/from capability path; `DeviceNamespace::Production` cannot be constructed from a Simulation URI accidentally (type-level distinct).

### D0.3 — State machines (`hardware-api`)

**Files:** `crates/hardware-api/src/state.rs`

Source doc §5.3:

```text
Unknown -> Discovered -> Identified -> Ready
                          -> Untrusted        Ready -> Degraded -> Faulted -(reset+checks)-> Ready
Actuator extra: Safe/Armed/Active/Stopping/EStopped   (EStopped recovers only via local procedure)
```

- [ ] `DeviceState` enum + `ActuatorState` enum. Implement `fn can_transition(&self, to) -> bool` rejecting illegal edges.
- [ ] `EStopped` has NO transition reachable from an Agent-issued command — only a `LocalRecovery` marker (Agent must not hold a generic "clear e-stop" capability, source doc §5.3/§10.2).
- [ ] Property test (proptest or hand-rolled): random transition sequences never reach `Ready` from `EStopped` without `LocalRecovery`; never skip `Identified`.

### D0.4 — Command, lease, telemetry, safety types (`hardware-api`)

**Files:** `crates/hardware-api/src/command.rs`, `lease.rs`, `telemetry.rs`, `safety.rs`, `error.rs`

Source doc §7 / §8, using `fabric::MonoTime` as `MonotonicInstant`:

```rust
// command.rs
pub struct TypedCommand {
    pub command_id: CommandId, pub device: DeviceId,
    pub schema: CommandSchemaId, pub payload: ValidatedPayload,   // opaque, built only by validator
    pub sequence: u64,
    pub issued_at: MonoTime, pub deadline: MonoTime,
    pub idempotency_key: Option<IdempotencyKey>, pub requested_ack: AckLevel,
}
pub enum AckLevel { Accepted, Started, Completed }
pub struct CommandReceipt { pub command_id: CommandId, pub sequence: u64, pub outcome: CommandOutcome, pub acked_at: MonoTime }

// lease.rs
pub struct ControlLease {
    pub lease_id: LeaseId, pub holder: ActorId, pub device: DeviceId,
    pub capabilities: CapabilitySet, pub mode: LeaseMode,          // Shared | Exclusive
    pub expires_at: MonoTime, pub deadman: DeadmanPolicy,
}
pub struct LeaseToken(/* opaque, signed-ish handle */);
pub struct DeadmanPolicy { pub heartbeat_interval: Duration, pub grace: Duration }

// telemetry.rs
pub struct TelemetryEnvelope {
    pub device: DeviceId, pub stream: StreamId, pub schema: TelemetrySchemaId,
    pub sequence: u64, pub source_time: DeviceTime, pub receive_time: MonoTime,
    pub quality: DataQuality, pub payload: BytesOrArtifactRef,     // keep source_time != receive_time
}

// safety.rs — the four-level stop hierarchy (source doc §7.3), MUST stay distinct
pub enum StopRequest { CancelTask, ControlledStop, SafeHold, EmergencyStop }
pub struct StopReceipt { pub requested: StopRequest, pub reached: StopOutcome }  // "task cancelled" != "device stopped"

// error.rs
pub enum HardwareError { NotFound, LeaseDenied, LeaseExpired, SchemaViolation{..}, DeadlineExceeded,
                         SequenceRegression, NamespaceMismatch, ProviderUnavailable, SafetyRefused{..}, Disconnected }
```

- [ ] Define all of the above with derives. `ValidatedPayload` is constructible **only** inside the broker validator (private field / builder in `hardware-broker`) — this is the type-level guarantee that raw JSON never reaches a provider (source doc §7.1, §11.3).
- [ ] `StopRequest` variants documented so UI/logs can never equate CancelTask with EmergencyStop (source doc §14).
- [ ] Unit tests for `HardwareError` display + serde.

### D0.5 — Provider + Broker traits (`hardware-api` traits, `hardware-broker` impl)

**Files:** `crates/hardware-api/src/provider.rs` (traits), `crates/hardware-broker/src/{registry,lease,validate,route,telemetry_bus}.rs`

Source doc §6.1:

```rust
#[async_trait]
pub trait HardwareProvider: Send + Sync {
    async fn probe(&self) -> Result<ProviderManifest, HardwareError>;
    async fn discover(&self, query: DeviceQuery) -> Result<Vec<DeviceManifest>, HardwareError>;
    async fn observe(&self, device: DeviceId, request: ObserveRequest) -> Result<TelemetryStream, HardwareError>;
    async fn execute(&self, lease: LeaseToken, command: TypedCommand) -> Result<CommandReceipt, HardwareError>;
    async fn stop(&self, lease: LeaseToken, request: StopRequest) -> Result<StopReceipt, HardwareError>;
}
```

- [ ] `HardwareProvider` trait in `hardware-api`; `TelemetryStream` = `tokio::sync::mpsc`/`Stream` of `TelemetryEnvelope`.
- [ ] `Broker` in `hardware-broker`: holds `HashMap<ProviderId, Arc<dyn HardwareProvider>>` + `Registry` (endpoint→stable `DeviceId` binding) + `LeaseArbiter` (deadman TTL, exclusive for actuator, shared for observe) + `CommandValidator` (JSON → `TypedCommand`, schema range/unit/precondition checks, builds `ValidatedPayload`) + `SequenceGuard` (monotonic per (device,lease); reject regression/replay) + `DeadlineGuard`.
- [ ] Broker `observe/execute/stop` path: capability check → namespace isolation (sim config cannot dispatch to production, source doc §6.2/§14) → lease arbitration → schema validation → sequence/deadline → provider call → `CommandReceipt` + audit.
- [ ] Telemetry bus: subscription declares reliability/max-latency/queue-len/drop-policy/sample-rate; on overflow apply drop policy (never block the caller / Executive event loop, source doc §8.2/§11.3). Fan-out: Level 2 summaries→Agora, Level 3 blobs→Artifact (D0 = in-memory sinks + trait seams; real Agora/Artifact wiring in D1).
- [ ] Tests: lease arbitration (exclusive denies 2nd holder; shared allows N observers); sequence regression rejected; deadline-exceeded rejected; namespace mismatch rejected; unvalidated JSON cannot construct `ValidatedPayload` (compile-fail or private-constructor test).

### D0.6 — Simulator: the executable safety spec (`hardware-sim`)

**Files:** `crates/hardware-sim/src/{clock,mobile_robot,faults}.rs`

Source doc §9.1 — the sim is not a demo; it is the safety model.

- [ ] `SimClock`: fixed-seed deterministic monotonic clock; all sim time advances explicitly (`tick(dt)`), enabling reproducible tests.
- [ ] `MobileRobotSim` implements `HardwareProvider`: exposes one `Robot`-class device; telemetry streams `pose`, `battery`, `health`; accepts `NavigateTo{x,y,yaw}` and `ControlledStop` typed commands; models motion toward target over ticks.
- [ ] Fault injection knobs: `disconnect`, `latency`, `reorder`, `duplicate`, `sensor_freeze`, `stuck_actuator`, `limit_violation`, `lease_expiry`, `estop`. Each replayable from a seed + event log.
- [ ] **Fail-safe behavior:** on disconnect / lease expiry / deadline miss, device transitions to `SafeHold` (not "continue last command", source doc §3/§7.3) and emits a health event.

### D0.7 — Kernel capability binding (observe-only default)

**Files:** `crates/hardware-broker/src/capability.rs`, plus a gate in the repo's architecture-status check

- [ ] Map `hardware.observe:/...`, `hardware.command:/...`, `hardware.stop:/...` onto `fabric::Capability` (`crates/fabric/src/types/capability.rs:30`) with `CapabilityLevel`: observe=`ReadOnly`, command=`SystemChange`, stop=`SystemChange`, admin/calibrate=`Destructive`. `hardware.command` does NOT imply `hardware.admin` or estop-reset (source doc §10.2).
- [ ] Broker's Kernel entrypoint accepts a `CapabilityAuthority` (`crates/fabric/src/include/turn.rs:52`); D0 grants **observe-only** by default — command/stop require an explicit lease + capability grant in the test harness.
- [ ] Architecture gate: assert `hardware-api` has no ROS/serial/CAN/vendor dependency (source doc §4 "hardware-api 必须保持传输无关"). Add to existing arch-status validation.

### D0 acceptance (source doc §12 D0 验收)

- [ ] End-to-end vertical-slice test (`hardware-broker/tests/vertical_slice.rs`): discover robot → observe pose/battery → acquire exclusive lease → renew via deadman heartbeat → issue `NavigateTo` (schema-validated, sequenced, deadlined) → receive `CommandReceipt` → `ControlledStop` → release lease. All against `hardware-sim`, no real hardware.
- [ ] Fault tests prove **fail-safe without real hardware**: (a) lease expiry → `SafeHold`; (b) disconnect mid-command → `SafeHold` + `Disconnected` receipt, not continued motion; (c) safe stop reachable. This is the D0 gate: "可在无真实硬件情况下验证租约过期、断连和安全停止".
- [ ] Every command traces to actor→capability→lease→schema→receipt (audit line asserted in test, source doc §11.3).
- [ ] `bash scripts/cargo-agent.sh test -p hardware-api && ... -p hardware-broker && ... -p hardware-sim` all green.

---

## D1 — Linux read-only discovery + telemetry (no actuator writes)

**Files (new):** `crates/hardware-broker/src/discovery/linux.rs`, `crates/hardware-broker/src/registry/binding.rs`, Agora/Artifact adapters `crates/hardware-broker/src/sink/{agora,artifact}.rs`

- [ ] Read-only enumeration of serial ports, CAN interfaces, GPIO chips, cameras (V4L2), compute accelerators — emitted as `Discovered` device manifests. No `execute` path opened.
- [ ] Stable-binding workflow (source doc §5.1): endpoint (serial no. / bus id) → admin-approved `DeviceId`; a runtime endpoint change (hot-plug) must NOT silently re-bind to the old identity.
- [ ] Wire the D0 telemetry sinks to real Agora (Level 2 state/events/summaries) and Artifact (Level 3 blobs). Backpressure test: a telemetry flood drops per policy and never enters Agora message bodies (source doc §8.2/§11.3).
- [ ] Persistent `DeviceRegistry` (manifest store with source/version; broker-discovered runtime facts cannot raise the admin-approved safety ceiling, source doc §5.2).

**Acceptance (§12 D1):** hot-plug does not cause identity mis-binding; high-frequency streams stay out of Agora bodies; only health/telemetry exposed, zero actuator write paths.

---

## D2 — ROS 2 simulation

**Files (new):** `crates/hardware-ros2/` (`Cargo.toml`, `src/{graph,topic,service,action,lifecycle,qos}.rs`), implements `HardwareProvider`.

- [ ] Graph discovery → provider endpoints (NOT stable `DeviceId` directly, source doc §9.2). Topic=telemetry, Service=short request, Action=long task with feedback+cancel.
- [ ] Managed-node lifecycle → provider health/ready ([ROS 2 lifecycle](https://design.ros2.org/articles/node_lifecycle.html)). Explicit QoS config, no defaults. SROS2/DDS identity bridged to (not replaced by) Aletheon capability.
- [ ] Connect to Gazebo or an existing robot sim. High-level navigate/mode commands with feedback, cancel, timeout. rosbag/MCAP → Artifact, never Agora bodies.
- [ ] Lock ROS distro + QoS/security config into a compatibility matrix at implementation time.

**Acceptance (§12 D2):** in sim, complete "acquire lease → execute → feedback → cancel/complete → release" through the ROS 2 provider using the SAME broker command/lease path proven in D0.

---

## D3 — Serial + CAN real devices (non-dangerous fixtures only) — hardware-gated, lighter

**Files (new):** `crates/hardware-serial/src/{transport,framing,crc,ack}.rs`, `crates/hardware-can/src/{socketcan,filter,isotp}.rs`.

- [ ] Serial: Host provides port+permission only; provider owns baud/framing/CRC/handshake/identity. Bounded parser buffers (reject over-long / unterminated frames). Non-idempotent writes not auto-retried by default (source doc §9.3).
- [ ] CAN: SocketCAN first ([kernel SocketCAN](https://docs.kernel.org/networking/can.html)); layer raw-frame provider vs device-protocol provider; filters, error frames, bus-off/restart, CAN-FD, optional ISO-TP. Send perms scoped by interface + CAN ID/range + frame type. bus-off/error frames → health/fault, not just logs.
- [ ] Only loopback + non-dangerous fixtures; motors NOT connected. Bus permission, rate-limit, logging, fault injection.

**Acceptance (§12 D3):** malformed frames, bus-off, unplug/replug, and non-idempotent-retry behavior are all predictable and covered by contract tests.

---

## D4 — Controlled actuators (lease + deadman + double-layer safety) — hardware-gated, lighter

**Files (new):** `crates/hardware-api/src/actuator.rs` (typed actuator command schemas, hard limits), broker `edge_lease.rs`.

- [ ] Typed actuator commands with hard limits, deadman, watchdog. Exclusive lease, `ControlledStop`, `SafeHold`.
- [ ] Double-layer lease validation: Broker AND Edge Runtime both verify (cloud judgement cannot override edge, source doc §7.2). Human approval + on-site safety flow for high-risk devices.

**Acceptance (§12 D4):** network loss, process crash, and lease expiry each drive the device into the expected safe state (extends D0 fail-safe tests onto real actuators).

---

## D5 — Robot HIL + Lab — hardware-gated, lighter

- [ ] One target robot only (no multi-vendor spread). Integrate its local Safety Supervisor, mode management, status summaries.
- [ ] HIL + `Lab` namespace, operator checklist, fault-recovery drills, full evidence bundle.

**Acceptance (§12 D5):** sustained supervised operation passing the mandated fault scenarios before any production canary.

---

## D6 — Multi-device + remote sites — hardware-gated, lighter

- [ ] Device federation, edge broker, offline policy, certificate rotation. Bandwidth tiering, disconnect caching, time-sync quality. Cross-site capability/lease must NOT implicitly inherit (source doc §12 D6).

---

## Verification system (all phases)

**Test pyramid (source doc §11.1):** Schema/property tests → Provider contract tests → Deterministic simulator + fault injection → SIL (real protocol stack) → HIL (non-dangerous fixture) → Lab supervised → Production canary. D0-D2 live in the bottom three tiers (pure sim/SIL, CI-runnable); D3+ add real-protocol/HIL tiers gated on hardware availability.

**Mandatory fault tests (source doc §11.2)** — implement in `hardware-sim` fault injection from D0, extend per phase:
- [ ] Lease expiry, out-of-order renew, duplicate holder.
- [ ] Stale command, deadline exceeded, sequence regression, duplicate send.
- [ ] Broker crash, provider crash, network partition, device restart.
- [ ] Telemetry flood, queue overflow, reordering, clock jump.
- [ ] Sensor freeze, NaN/out-of-range, stale calibration.
- [ ] Actuator no-response, partial completion, lost stop-ack.
- [ ] E-stop trigger, insufficient recovery conditions, bad reset request.
- [ ] sim/lab/production namespace confusion.
- [ ] Wrong device binding, hot-plug identity drift.

**Production gates (source doc §11.3)** — must all hold before any real-actuator (D4+) rollout:
- [ ] Model cannot produce a bus write that skipped schema+policy validation.
- [ ] Lease expiry and broker disconnect have provable fail-safe.
- [ ] Local Safety Supervisor does not depend on Aletheon being online.
- [ ] Every command traces to actor / capability grant / lease / schema / receipt.
- [ ] High-risk paths have HIL + human-supervision records.
- [ ] Raw telemetry cannot overwhelm the Agent control plane.
- [ ] Device/provider/firmware/schema compatibility matrix is queryable.

## Explicitly out of scope (source doc §14)

LLM never writes `/dev/tty*`/`can0`/GPIO lines directly · arbitrary JSON is never a type-safe command · cloud Agent never owns hard-real-time or final safety · no multi-vendor robots in v1 · "ROS topic received a message" is NOT a completion criterion · CancelTask ≠ ControlledStop ≠ EmergencyStop · sim/lab/production never share default write perms.
