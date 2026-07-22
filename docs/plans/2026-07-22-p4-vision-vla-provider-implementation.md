# P4 Vision and VLA Provider Implementation Plan

> **For agentic workers:** Run in Goal mode after P3 acceptance. Execute sequentially and attach evidence to every checked item.

**Goal:** Add bounded RGB evidence and governed low-frequency skill proposals without allowing policies to bypass P3 verification.

**Architecture:** Fabric owns frame/proposal contracts, Dasein aggregates perception, a typed PolicyProvider proposes registered skills, and Executive routes proposals through existing Kernel and RobotHarness boundaries.

**Tech Stack:** Rust, serde, existing artifact/path/network policy, optional external gRPC policy service, P2 Bridge camera adapter.

---

## Task 1: Frame and perception contracts

**Files:**
- Create: `crates/fabric/src/types/frame.rs`
- Create: `crates/fabric/src/types/perception_observation.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Test: `crates/fabric/tests/frame_contract.rs`

- [ ] Test SHA-256 format, allowed `image/jpeg` and `image/png`, dimensions/byte limits, trusted artifact URI, source/received/freshness timestamps and serde stability.
- [ ] Implement `FrameRef` without image bytes and `PerceptionObservation` with bounded labels/summary/confidence/evidence.
- [ ] Reject data URI, HTTP URI, path traversal, zero dimensions, future-skew outside configured tolerance and expired frame.
- [ ] Run Fabric test; commit `feat(fabric): define bounded visual evidence`.

## Task 2: Artifact ownership and integrity

**Files:**
- Create: `crates/platform/src/artifact_store.rs`
- Test: `crates/platform/tests/artifact_store.rs`

- [ ] Confirm the current Platform crate has no artifact store, then declare `pub mod artifact_store` in `crates/platform/src/lib.rs`; Platform is the fixed owner for P4 artifacts.
- [ ] Test atomic write, hash verification, MIME/size allowlist, owner-only permissions, quota, expiry, traversal/symlink rejection and content-addressed dedupe.
- [ ] Implement `put/read_metadata/open_read` ports; never return an unrestricted filesystem path to model-facing code.
- [ ] Run focused test; commit `feat(platform): store bounded perception artifacts`.

## Task 3: Dasein visual aggregation

**Files:**
- Modify: `crates/dasein/src/impl/perception/event.rs`
- Modify: `crates/dasein/src/impl/perception/aggregator.rs`
- Test: `crates/dasein/tests/visual_perception.rs`

- [ ] Test dedupe by camera+hash, at most configured Hz, latest-frame selection, stale eviction, confidence merge and bounded event count.
- [ ] Add a visual event variant containing FrameRef and compact semantic metadata only.
- [ ] Confirm no image bytes are cloned into Agora/turn context.
- [ ] Run test; commit `feat(dasein): aggregate visual frame evidence`.

## Task 4: SkillProposal contract

**Files:**
- Create: `crates/fabric/src/types/skill_proposal.rs`
- Test: `crates/fabric/tests/skill_proposal.rs`

- [ ] Define `PolicyProvenance { provider, model, version, digest }` and `SkillProposal { skill, device, parameters, expected_outcome, confidence, frame_refs, provenance }`.
- [ ] Test confidence in [0,1], bounded frames ≤4, exact model digest, known protocol version, validated ExpectedOutcome and parameter size.
- [ ] Explicitly make raw joint/torque/topic fields unrepresentable.
- [ ] Run test; commit `feat(fabric): define governed skill proposals`.

## Task 5: PolicyProvider port and proposal validator

**Files:**
- Create: `crates/cognit/src/ports/policy_provider.rs`
- Create: `crates/cognit/src/harness/robot/proposal_validator.rs`
- Test: `crates/cognit/tests/skill_proposal.rs`

- [ ] Define async `propose(goal, observations, allowed_skills)` returning bounded proposals.
- [ ] Test unknown skill/device, schema mismatch, expired frame, low confidence, missing provenance, parameter overflow and valid proposal.
- [ ] Validate against live `ListSkills` descriptors; never trust provider-supplied schema.
- [ ] Run test; commit `feat(cognit): validate policy skill proposals`.

## Task 6: Generic external Policy gRPC adapter

**Files:**
- Create: `crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto`
- Create: `crates/cognit/src/impl/policy/grpc_provider.rs`
- Create: build generation following the reviewed P2 pattern
- Test: `crates/cognit/tests/grpc_policy_provider.rs`

- [ ] Protocol exposes Capabilities, Propose and Health only; no Execute RPC.
- [ ] Test loopback plaintext allowed, non-loopback plaintext rejected, deadline, size limit, version mismatch, unhealthy provider, unknown enum and provenance conversion.
- [ ] Implement typed config and explicit wire/domain conversions; no environment reads or string error classification.
- [ ] Run test; commit `feat(cognit): add external policy proposal provider`.

## Task 7: P2 Bridge RGB snapshot support

**Files in `aletheon-kuavo-bridge`:**
- Create: `providers/kuavo_noetic/camera.py`
- Modify: generic gateway observation mapping only; do not add image bytes
- Test: `tests/integration/test_camera_snapshot.py`

- [ ] Discover actual MuJoCo camera topic/type/rate; record it in the interface inventory. If absent, use a test camera node external to Kuavo rather than modifying Kuavo core.
- [ ] Write JPEG/PNG artifact atomically, compute hash, emit FrameRef metadata and enforce rate/size/freshness limits.
- [ ] Test corrupt frame, oversized frame, stale frame, camera disconnect and cleanup.
- [ ] Run Bridge checks and commit `feat(perception): expose bounded RGB frame evidence`.

## Task 8: RobotHarness policy integration

**Files:**
- Modify: `crates/cognit/src/harness/robot/session.rs`
- Modify: Executive RobotHarness composition
- Test: `crates/executive/tests/robot_policy_path.rs`

- [ ] Add policy proposal only in Plan; pass the validated proposal into the existing Authorize state.
- [ ] Test Kernel denial, proposal rejection, policy timeout, one replan, P3 verifier mismatch and SafeStop.
- [ ] Confirm policy cannot call EmbodimentExecutionPort directly.
- [ ] Run tests; commit `feat(robot): route policy proposals through governance`.

## Task 9: P4 E2E and privacy acceptance

- [ ] Start MuJoCo, Bridge camera adapter, Policy fixture and Aletheon RobotHarness.
- [ ] Execute fixed scenario: frame contains target marker, provider proposes one registered semantic skill, Kernel admits, P2 executes, P3 verifies.
- [ ] Negative cases: stale frame, altered hash, unknown skill, low confidence, Provider success/no state change.
- [ ] Search logs/databases for JPEG magic/base64 payload; expect none. Verify only URI/hash/metadata persist.
- [ ] Record policy digest, frame hash, operations and verification report.

## Task 10: Final P4 validation

- [ ] Run focused Fabric/Platform/Dasein/Cognit/Executive tests via cargo-agent and Bridge `scripts/check.sh`.
- [ ] Run workspace build/test only as verification owner.
- [ ] Confirm `rg -n 'execute_skill|cmd_vel|joint|torque' crates/cognit/src/impl/policy` finds no direct actuation path.
- [ ] Confirm P2/P3 acceptance remains green; report P5 as deferred.
