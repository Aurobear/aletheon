# P5 HIL and Production Gates Implementation Plan

> **For agentic workers:** Run in Goal mode only after the listed prerequisites. Never test a new control path first on a person-accessible robot.

**Goal:** Prevent unverified software, credentials, policies or network behavior from reaching a real Kuavo robot.

**Architecture:** Hardware and Executive enforce typed namespace/startup gates, Bridge provides local watchdog/E-stop integration, and signed HIL evidence plus local operator arming is required before a Production Provider becomes ready.

**Tech Stack:** Existing Rust governance and config, Bridge gRPC/ROS adapter, Linux traffic control/netem in an isolated HIL network, systemd credentials, signed JSON evidence.

---

## Preconditions

- [ ] P2 live MuJoCo and P3 OutcomeVerifier acceptance are green.
- [ ] Obtain a dedicated HIL bench with physical E-stop, exclusion zone and named human safety owner.
- [ ] Record robot model/serial, controller version, ROS interface inventory and approved network segment.
- [ ] Do not run production tasks unattended; Goal mode may automate tests only within the approved HIL envelope.

## Task 1: Namespace and deployment contracts

**Files:**
- Modify: `crates/hardware/src/device.rs`
- Create: `crates/hardware/src/deployment_gate.rs`
- Test: `crates/hardware/tests/deployment_gate.rs`

- [ ] Extend namespace to Simulation/Hil/Production with stable serde and no Default for Production.
- [ ] Define `DeploymentGateInput` containing device ID/serial, endpoint identity, namespace, skill manifest digest, limits digest, evidence digest and expiry.
- [ ] Test simulation defaults, explicit HIL, production missing fields, namespace/credential mismatch, expired evidence and wrong device serial.
- [ ] Implement fail-closed validation; run Hardware test and commit `feat(hardware): enforce deployment namespaces`.

## Task 2: Production typed config and credential separation

**Files:**
- Modify verified Executive embodiment config location from P2
- Modify: `config/production.toml.example`
- Update schema snapshots/tests
- Test: `crates/executive/tests/production_embodiment_config.rs`

- [ ] Test production absent/false by default; production requires `namespace="production"`, explicit device/serial, TLS endpoint, credential reference, allowlist, limit profile and gate evidence path.
- [ ] Reject inline private keys/tokens, loopback simulation identity reused for production, wildcard skills and values exceeding compiled maxima.
- [ ] Resolve credentials only through existing host-owned secret loader/systemd credentials; redact diagnostics.
- [ ] Run config tests; commit `feat(config): fail closed for production embodiment`.

## Task 3: Signed HIL evidence format

**Files:**
- Create: `crates/fabric/src/types/hil_evidence.rs`
- Create: `crates/platform/src/hil_evidence_verifier.rs`
- Test: `crates/platform/tests/hil_evidence_verifier.rs`

- [ ] Define canonical JSON report with schema version, device/serial, software commits, manifest/limits digests, test cases, measured stop latencies, result, issued/expiry and signer key ID.
- [ ] Test canonicalization, signature, tamper, wrong signer/device/digest, expiry and unknown schema.
- [ ] Verify against an allowlisted public key loaded by host configuration; never allow self-signed runtime keys.
- [ ] Run tests; commit `feat(platform): verify signed HIL gate evidence`.

## Task 4: Independent EmergencyStop contract

**Files:**
- Create: `crates/fabric/src/types/emergency_stop.rs`
- Create: `crates/hardware/src/emergency_stop.rs`
- Test: `crates/hardware/tests/emergency_stop.rs`

- [ ] Define states Armed/Triggered/Latched/ResetRequired; only local trusted adapter can transition ResetRequired→Armed.
- [ ] Test priority over all operations, exactly-once latch, concurrent trigger, restart persistence, remote reset rejection and audit event.
- [ ] Ensure triggering does not wait on ordinary Broker/operation locks.
- [ ] Run tests; commit `feat(safety): add latched emergency stop authority`.

## Task 5: Bridge watchdog and physical E-stop adapter

**Files in `aletheon-kuavo-bridge`:**
- Create: `src/aletheon_kuavo_bridge/safety/watchdog.py`
- Create: `src/aletheon_kuavo_bridge/safety/emergency_stop.py`
- Create: `src/aletheon_kuavo_bridge/safety/__init__.py`
- Modify Kuavo Provider only at the verified local stop interfaces
- Test: `tests/fault_injection/test_watchdog.py`, `test_emergency_stop.py`

- [ ] Inject monotonic clock and test missed heartbeat, stale state, ROS disconnect, lease expiry and process restart.
- [ ] Trigger the verified local stop path without Aletheon/gRPC availability; latch state to owner-only durable runtime storage.
- [ ] Expose status but no remote reset RPC. Reset requires a local command that checks physical channel clear and operator identity.
- [ ] Run Bridge checks; commit `feat(safety): add local HIL watchdog and E-stop latch`.

## Task 6: Production skill allowlist and limit profiles

**Files:**
- Create: Bridge `config/skills/kuavo_hil.yaml`
- Create: Bridge `config/skills/kuavo_production.yaml`
- Modify config validation
- Test: Bridge `tests/unit/test_production_manifest.py`

- [ ] Production manifest contains only stance, stop, safe stop and one explicitly named reviewed low-risk action; no timed base movement initially.
- [ ] Test wildcard, unknown handler, simulation limit inheritance, manifest digest mismatch and runtime mutation rejection.
- [ ] Make manifest immutable after readiness; changes require process restart and new gate evidence.
- [ ] Commit `feat(policy): enforce production robot allowlist`.

## Task 7: Network fault-injection harness

**Files in Bridge:**
- Create: `tests/hil/network_fault_matrix.yaml`
- Create: `scripts/run-hil-network-matrix.sh`
- Create: `tests/hil/test_network_faults.py`

- [ ] Define exact cases: latency 50/100/250ms, jitter 20/50ms, loss 1/5/20%, duplicate 1/5%, reorder 5/20%, hard disconnect, half-open, gRPC restart, ROS restart, lease expiry and stale observation.
- [ ] Run only inside a dedicated network namespace/container using `tc netem`; script must refuse the default host namespace and require the HIL device allowlist.
- [ ] For each case measure detection time, final command time, controller safe-state time and total stop latency; fail above the safety-owner-approved threshold encoded in the signed test profile.
- [ ] Restore qdisc/network in a trap and verify cleanup; never reboot host/robot automatically.
- [ ] Commit `test(hil): add deterministic network fault matrix`.

## Task 8: Startup gate composition

**Files:**
- Modify Executive embodiment bootstrap from P2
- Create: `crates/executive/src/impl/daemon/bootstrap/production_embodiment.rs`
- Test: `crates/executive/tests/production_embodiment_gate.rs`

- [ ] Test gate order and ensure Provider connection/arming never occurs before all checks pass.
- [ ] Require config, credentials, device identity handshake, manifest/limits digest, signed evidence, E-stop self-test and local operator arming receipt.
- [ ] Report sanitized failure component in health; never downgrade failed production to simulator silently.
- [ ] Run test; commit `feat(executive): gate production robot startup`.

## Task 9: Audit chain and retention

**Files:**
- Create: `crates/fabric/src/types/robot_audit.rs`
- Create/extend the verified Executive audit repository
- Test: `crates/executive/tests/robot_audit_chain.rs`

- [ ] Record goal, operation, attempt, permit, lease, manifest digest, device receipt, verification, recovery, SafeStop/E-stop and operator arming IDs.
- [ ] Test append-only hash chain, conflicting replay, restart, redaction, bounded retention and export integrity.
- [ ] Exclude credentials, raw images and high-frequency state.
- [ ] Run test; commit `feat(audit): preserve robot governance chain`.

## Task 10: HIL dry-run acceptance

- [ ] With actuators disabled or robot secured, verify identity, state, E-stop, watchdog and allowlist.
- [ ] Enable only the reviewed low-risk action under exclusion-zone procedures.
- [ ] Run every network matrix case and physical/software E-stop case with human safety owner present.
- [ ] Generate canonical report, review measurements, sign with approved offline key and store public evidence artifact.
- [ ] A failed case produces no signed passing evidence and blocks Task 11.

## Task 11: Production minimum-capability acceptance

- [ ] Verify default config refuses Production and HIL/simulation credentials cannot authenticate.
- [ ] Start with signed, unexpired evidence and local operator arming.
- [ ] Exercise health/snapshot/stance/reviewed action/stop/safe stop; then physical E-stop.
- [ ] Confirm remote E-stop reset is impossible and process restart retains latch.
- [ ] Confirm P3 verifies actual state and audit chain links all identities.
- [ ] Stop on first unexplained behavior; do not automatically retry a physical motion.

## Task 12: Final validation and release gate

- [ ] Run focused Hardware/Fabric/Platform/Executive suites via cargo-agent and Bridge checks.
- [ ] Run workspace build/test only as verification owner.
- [ ] Re-run config negative tests, evidence tamper tests and network cleanup checks.
- [ ] Security review must approve credential boundaries, mTLS identity, allowlist, E-stop authority and audit redaction.
- [ ] Produce rollback/runbook: disarm, stop Bridge, revoke credential, restore simulation default; no computer reboot required.
- [ ] Production remains disabled until repository review, HIL evidence review and named human approval are all complete.
