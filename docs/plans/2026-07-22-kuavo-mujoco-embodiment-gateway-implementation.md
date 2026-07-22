# Kuavo MuJoCo Embodiment Gateway Implementation Plan

> **For agentic workers:** Execute this plan task-by-task in order. Update each checkbox only after its command has produced the stated result. Do not use subagents unless the operator explicitly enables them.

**Goal:** Build a local `aletheon-kuavo-bridge` project and an Aletheon gRPC provider that replace the P1 simulator provider without exposing ROS/Kuavo types above Hardware.

**Architecture:** A vendor-neutral protobuf/gRPC service runs in a Python `rospy` sidecar. A Rust adapter in `hardware` converts between the wire contract and Fabric DTOs. Kuavo-specific topic/service/action mappings remain inside the Bridge and target the standard ROS Noetic MuJoCo launch.

**Tech Stack:** Python 3.8-compatible source, `rospy`, `grpcio`, `protobuf`, `grpcio-tools`, pytest; Rust 1.85+, `tonic`, `prost`, Tokio; ROS Noetic; Kuavo MuJoCo.

**Approved design:** `docs/plans/2026-07-22-kuavo-mujoco-embodiment-gateway-design.md`

---

## 0. Execution contract

### Repositories and branch ownership

| Repository | Path | Branch | Owned scope |
|---|---|---|---|
| Aletheon | `/home/aurobear/Workspace/aletheon` | create from current `origin/dev` | protocol source copy, Rust provider, typed config, bootstrap, tests, docs |
| Bridge | `/home/aurobear/Workspace/aletheon-kuavo-bridge` | initialize `dev`, then feature branch | Python gRPC server, ROS adapter, scripts, tests |
| Kuavo | `/home/aurobear/Workspace/kuavo-ros-control` | read-only `dev` | interface discovery and MuJoCo runtime only |

Do not modify `kuavo-ros-control`. Do not create git worktrees inside any repository. Do not copy its source or private ROS message definitions into Aletheon.

All Aletheon Cargo commands must use:

```bash
bash scripts/cargo-agent.sh <cargo arguments>
```

Never invoke `cargo` directly. Use the narrowest package/test target. Only the final verification task may use workspace-wide checks.

### Required stop conditions

Stop the current task and report evidence instead of guessing when:

1. standard MuJoCo launch does not become healthy;
2. a required ROS interface is absent or its actual type conflicts with the mapping manifest;
3. protobuf generated types diverge between Python and Rust;
4. a movement cannot be confirmed stopped from a fresh state source;
5. any test would need a fixed success response or unimplemented placeholder;
6. a proposed change requires modifying Kuavo MPC/WBC/controller code.

### Commit format

Every non-trivial commit must contain a conventional subject, a blank line, problem/solution context, and concrete bullets. Inspect `git diff --cached` before every commit.

---

## Stage G1 — Wire contract and generated-code reproducibility

### Task 1: Create the Bridge repository skeleton

**Files:**
- Create repository: `/home/aurobear/Workspace/aletheon-kuavo-bridge`
- Create: `pyproject.toml`
- Create: `.gitignore`
- Create: `README.md`
- Create directories listed below

- [ ] **Step 1: Verify the destination is absent or empty**

```bash
test ! -e /home/aurobear/Workspace/aletheon-kuavo-bridge \
  || test -z "$(find /home/aurobear/Workspace/aletheon-kuavo-bridge -mindepth 1 -maxdepth 1 -print -quit)"
```

Expected: exit 0. If non-empty, stop and inspect ownership; never delete it.

- [ ] **Step 2: Create the project tree**

```bash
mkdir -p /home/aurobear/Workspace/aletheon-kuavo-bridge/{proto/aletheon/embodiment/gateway/v1,src/aletheon_kuavo_bridge/{generated,providers/kuavo_noetic/skills},config/skills,launch,scripts,tests/{unit,contract,integration,fault_injection},docs}
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
git init -b dev
```

- [ ] **Step 3: Write packaging metadata**

`pyproject.toml`:

```toml
[build-system]
requires = ["setuptools>=68", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "aletheon-kuavo-bridge"
version = "0.1.0"
requires-python = ">=3.8"
dependencies = [
  "grpcio>=1.62,<2",
  "protobuf>=4.25,<6",
  "PyYAML>=6,<7",
]

[project.optional-dependencies]
dev = [
  "grpcio-tools>=1.62,<2",
  "pytest>=8,<9",
  "pytest-asyncio>=0.23,<1",
  "ruff>=0.5,<1",
  "mypy>=1.10,<2",
]

[project.scripts]
aletheon-kuavo-bridge = "aletheon_kuavo_bridge.server:main"

[tool.setuptools.packages.find]
where = ["src"]

[tool.pytest.ini_options]
testpaths = ["tests"]
asyncio_mode = "auto"

[tool.ruff]
target-version = "py38"
line-length = 100

[tool.mypy]
python_version = "3.8"
strict = true
```

`.gitignore`:

```gitignore
.venv/
__pycache__/
*.py[cod]
.pytest_cache/
.mypy_cache/
.ruff_cache/
dist/
build/
*.egg-info/
.coverage
artifacts/
```

- [ ] **Step 4: Add package markers and verify installation**

Create empty `__init__.py` files under `src/aletheon_kuavo_bridge`, `generated`, `providers`, `providers/kuavo_noetic`, and `providers/kuavo_noetic/skills`.

```bash
python3 -m venv .venv
.venv/bin/pip install -e '.[dev]'
.venv/bin/python -c 'import aletheon_kuavo_bridge'
```

Expected: exit 0.

- [ ] **Step 5: Commit**

Suggested subject: `chore: scaffold the Kuavo embodiment bridge`

### Task 2: Define the complete versioned protobuf contract

**Files:**
- Create: `proto/aletheon/embodiment/gateway/v1/gateway.proto`
- Create: `scripts/generate-proto.sh`
- Test: `tests/contract/test_generated_contract.py`

- [ ] **Step 1: Write the contract test**

```python
from aletheon_kuavo_bridge.generated import gateway_pb2


def test_execute_event_has_exactly_one_payload() -> None:
    event = gateway_pb2.ExecuteSkillEvent(
        result=gateway_pb2.SkillResult(
            operation_id="00000000-0000-4000-8000-000000000001",
            device_id="kuavo-mujoco-01",
            skill_id="kuavo.stance",
            outcome=gateway_pb2.SKILL_OUTCOME_SUCCEEDED,
        )
    )
    assert event.WhichOneof("event") == "result"


def test_health_state_numbers_are_stable() -> None:
    assert gateway_pb2.HEALTH_STATE_READY == 1
    assert gateway_pb2.HEALTH_STATE_DEGRADED == 2
    assert gateway_pb2.HEALTH_STATE_UNAVAILABLE == 3
```

- [ ] **Step 2: Confirm it fails before generation**

```bash
.venv/bin/pytest tests/contract/test_generated_contract.py -q
```

Expected: FAIL because `gateway_pb2` does not exist.

- [ ] **Step 3: Write `gateway.proto`**

The file must use `syntax = "proto3"`, package `aletheon.embodiment.gateway.v1`, and define:

```proto
syntax = "proto3";

package aletheon.embodiment.gateway.v1;

import "google/protobuf/struct.proto";

service EmbodimentGateway {
  rpc GetCapabilities(GetCapabilitiesRequest) returns (GetCapabilitiesResponse);
  rpc Snapshot(SnapshotRequest) returns (SnapshotResponse);
  rpc ListSkills(ListSkillsRequest) returns (ListSkillsResponse);
  rpc ExecuteSkill(ExecuteSkillRequest) returns (stream ExecuteSkillEvent);
  rpc Cancel(CancelRequest) returns (CancelResponse);
  rpc SafeStop(SafeStopRequest) returns (SafeStopResponse);
  rpc Health(HealthRequest) returns (HealthResponse);
}

enum HealthState { HEALTH_STATE_UNSPECIFIED = 0; HEALTH_STATE_READY = 1; HEALTH_STATE_DEGRADED = 2; HEALTH_STATE_UNAVAILABLE = 3; }
enum RiskClass { RISK_CLASS_UNSPECIFIED = 0; RISK_CLASS_READ = 1; RISK_CLASS_LOW = 2; RISK_CLASS_MEDIUM = 3; RISK_CLASS_HIGH = 4; }
enum SkillOutcome { SKILL_OUTCOME_UNSPECIFIED = 0; SKILL_OUTCOME_SUCCEEDED = 1; SKILL_OUTCOME_FAILED = 2; SKILL_OUTCOME_CANCELLED = 3; SKILL_OUTCOME_TIMED_OUT = 4; }
enum ErrorCode { ERROR_CODE_UNSPECIFIED = 0; ERROR_CODE_INVALID_ARGUMENT = 1; ERROR_CODE_UNSUPPORTED_VERSION = 2; ERROR_CODE_UNKNOWN_DEVICE = 3; ERROR_CODE_UNKNOWN_SKILL = 4; ERROR_CODE_CONFLICT = 5; ERROR_CODE_NOT_READY = 6; ERROR_CODE_STALE_STATE = 7; ERROR_CODE_DEADLINE_EXCEEDED = 8; ERROR_CODE_PROVIDER_DISCONNECTED = 9; ERROR_CODE_INTERNAL = 10; }

message RequestMeta { string protocol_version = 1; string request_id = 2; string trace_id = 3; int64 deadline_unix_ms = 4; }
message ErrorDetail { ErrorCode code = 1; string category = 2; string message = 3; bool retryable = 4; }
message EvidenceRef { string kind = 1; string uri = 2; }
message DeviceRef { string device_id = 1; }
message Empty {}

message GetCapabilitiesRequest { RequestMeta meta = 1; }
message GetCapabilitiesResponse { string protocol_version = 1; string provider_id = 2; repeated string device_ids = 3; uint32 max_message_bytes = 4; uint32 max_progress_hz = 5; }

message SnapshotRequest { RequestMeta meta = 1; string device_id = 2; }
message Observation { string schema = 1; uint32 schema_version = 2; string source = 3; uint64 sequence = 4; int64 source_unix_ms = 5; int64 received_unix_ms = 6; int64 valid_until_unix_ms = 7; float confidence = 8; string frame_ref = 9; google.protobuf.Struct payload = 10; repeated EvidenceRef evidence = 11; bool stale = 12; }
message SnapshotResponse { repeated Observation observations = 1; ErrorDetail error = 2; }

message ListSkillsRequest { RequestMeta meta = 1; string device_id = 2; }
message SkillDescriptor { string skill_id = 1; string device_id = 2; string summary = 3; google.protobuf.Struct input_schema = 4; RiskClass risk = 5; uint64 timeout_ms = 6; bool cancellable = 7; repeated string preconditions = 8; repeated string success_criteria = 9; }
message ListSkillsResponse { repeated SkillDescriptor skills = 1; ErrorDetail error = 2; }

message ExecuteSkillRequest { RequestMeta meta = 1; string operation_id = 2; string device_id = 3; string skill_id = 4; google.protobuf.Struct parameters = 5; int64 lease_expires_unix_ms = 6; }
message Accepted { int64 accepted_unix_ms = 1; }
message SkillProgress { string operation_id = 1; string skill_id = 2; float fraction = 3; string note = 4; int64 at_unix_ms = 5; }
message SkillResult { string operation_id = 1; string skill_id = 2; string device_id = 3; SkillOutcome outcome = 4; string failure_reason = 5; uint64 duration_ms = 6; repeated EvidenceRef evidence = 7; }
message ExecuteSkillEvent { oneof event { Accepted accepted = 1; SkillProgress progress = 2; SkillResult result = 3; ErrorDetail error = 4; } }

message CancelRequest { RequestMeta meta = 1; string operation_id = 2; string device_id = 3; }
message CancelResponse { bool acknowledged = 1; ErrorDetail error = 2; }
message SafeStopRequest { RequestMeta meta = 1; string device_id = 2; string reason = 3; }
message SafeStopResponse { bool applied = 1; ErrorDetail error = 2; }
message HealthRequest { RequestMeta meta = 1; }
message ComponentHealth { string component = 1; HealthState state = 2; string detail = 3; int64 observed_unix_ms = 4; }
message HealthResponse { HealthState state = 1; repeated ComponentHealth components = 2; ErrorDetail error = 3; }
```

- [ ] **Step 4: Add deterministic generation**

`scripts/generate-proto.sh` must resolve repository root, invoke `.venv/bin/python -m grpc_tools.protoc`, output Python files into `src/aletheon_kuavo_bridge/generated`, and rewrite the generated absolute `import gateway_pb2` to a package-relative import. It must exit non-zero when `.venv` is absent.

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PY="$ROOT/.venv/bin/python"
test -x "$PY" || { echo "missing .venv; run scripts/bootstrap.sh" >&2; exit 2; }
OUT="$ROOT/src/aletheon_kuavo_bridge/generated"
PROTO_DIR="$ROOT/proto/aletheon/embodiment/gateway/v1"
"$PY" -m grpc_tools.protoc \
  -I "$PROTO_DIR" \
  --python_out="$OUT" \
  --grpc_python_out="$OUT" \
  "$PROTO_DIR/gateway.proto"
sed -i 's/^import gateway_pb2 as gateway__pb2$/from . import gateway_pb2 as gateway__pb2/' "$OUT/gateway_pb2_grpc.py"
```

- [ ] **Step 5: Generate and pass tests**

```bash
chmod +x scripts/generate-proto.sh
scripts/generate-proto.sh
.venv/bin/pytest tests/contract/test_generated_contract.py -q
```

Expected: 2 passed.

- [ ] **Step 6: Commit**

Suggested subject: `feat(protocol): define embodiment gateway v1`

---

## Stage G2 — Bridge core without ROS

### Task 3: Typed configuration and fail-closed skill manifest

**Files:**
- Create: `src/aletheon_kuavo_bridge/config.py`
- Create: `config/bridge.example.yaml`
- Create: `config/skills/kuavo_mujoco.yaml`
- Test: `tests/unit/test_config.py`

- [ ] **Step 1: Write tests for valid config and four rejection cases**

Tests must assert: default bind is `127.0.0.1`; duplicate skill IDs fail; unknown handler fails; configured limits above hard limits fail; a movement skill without a timeout fails.

- [ ] **Step 2: Implement immutable dataclasses and loader**

Use `@dataclass(frozen=True)` for `ServerConfig`, `SafetyLimits`, `SkillConfig`, and `BridgeConfig`. Use `yaml.safe_load`. Hard limits are constants:

```python
HARD_MAX_LINEAR_MPS = 0.25
HARD_MAX_ANGULAR_RPS = 0.5
HARD_MAX_DURATION_MS = 3000
KNOWN_HANDLERS = frozenset({"stance", "move_base_timed", "stop", "execute_arm_action"})
```

The example config must set device ID `kuavo-mujoco-01`, provider ID `kuavo-noetic-mujoco`, snapshot rate 5 Hz, progress rate 10 Hz, state freshness 500 ms, and localhost port 50051. The default manifest registers only `kuavo.stance`, `kuavo.move_base_timed`, and `kuavo.stop`. Do not register arm actions before Task 12 discovery.

- [ ] **Step 3: Verify**

```bash
.venv/bin/pytest tests/unit/test_config.py -q
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

Suggested subject: `feat(config): validate bridge and skill policy`

### Task 4: Operation registry with idempotency and exactly-once terminal state

**Files:**
- Create: `src/aletheon_kuavo_bridge/operation_registry.py`
- Test: `tests/unit/test_operation_registry.py`

- [ ] **Step 1: Write tests**

Cover legal transitions, illegal transition rejection, same ID/same payload replay, same ID/different payload conflict, cancel after terminal, and two concurrent terminal writes where exactly one succeeds.

- [ ] **Step 2: Implement**

Define `OperationState` enum with `RECEIVED`, `VALIDATED`, `ACCEPTED`, `EXECUTING`, `CANCELLING`, `SUCCEEDED`, `FAILED`, `CANCELLED`, `TIMED_OUT`, `REJECTED`. Store SHA-256 of canonical JSON payload, timestamps, result, cancellation `asyncio.Event`, and an `asyncio.Lock` per operation. `begin()` returns either a new record or the matching existing record; it raises `OperationConflict` for hash mismatch. `finish()` returns `False` when already terminal.

- [ ] **Step 3: Verify**

```bash
.venv/bin/pytest tests/unit/test_operation_registry.py -q
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

Suggested subject: `feat(runtime): add idempotent operation registry`

### Task 5: Observation aggregation and freshness

**Files:**
- Create: `src/aletheon_kuavo_bridge/observation.py`
- Test: `tests/unit/test_observation.py`

- [ ] **Step 1: Write deterministic fake-clock tests**

Assert monotonic sequence, content deduplication, at most 5 Hz emission, stale after 500 ms, and that full joint arrays are replaced by count/min/max/fault summary in the cognitive snapshot.

- [ ] **Step 2: Implement `ObservationAggregator`**

Inject `now_ms: Callable[[], int]`; never call wall clock directly in domain methods. Hash canonical payloads for dedupe. Keep only the latest bounded snapshot and no unbounded history.

- [ ] **Step 3: Verify and commit**

```bash
.venv/bin/pytest tests/unit/test_observation.py -q
```

Suggested subject: `feat(observation): aggregate bounded fresh snapshots`

### Task 6: Provider abstraction and Fake Provider

**Files:**
- Create: `src/aletheon_kuavo_bridge/providers/base.py`
- Create: `src/aletheon_kuavo_bridge/providers/fake.py`
- Test: `tests/contract/test_provider_contract.py`

- [ ] **Step 1: Define protocol and contract suite**

The async Protocol must expose `snapshot`, `list_skills`, `execute_skill`, `cancel`, `safe_stop`, and `health`. `execute_skill` accepts an immutable request plus an async event sink. The reusable contract suite must verify result identity, bounded progress fractions, cancel idempotency, safe-stop idempotency, unknown skill rejection, and failure when health is unavailable.

- [ ] **Step 2: Implement Fake Provider without success sentinels**

The Fake Provider must model position and mode, advance state only for registered semantic skills, and derive results from state transitions. It must support injected disconnect and stale-state faults.

- [ ] **Step 3: Verify and commit**

```bash
.venv/bin/pytest tests/contract/test_provider_contract.py -q
```

Suggested subject: `test(provider): establish reusable embodiment contract`

### Task 7: Safety supervisor

**Files:**
- Create: `src/aletheon_kuavo_bridge/safety.py`
- Test: `tests/fault_injection/test_safety.py`

- [ ] **Step 1: Write tests**

Assert SafeStop blocks new movement, signals active operations, invokes cancel, publishes/requests stop through the provider, confirms fresh stable state, stays unavailable when confirmation fails, and is idempotent under concurrent calls.

- [ ] **Step 2: Implement `SafetySupervisor`**

Use a dedicated lock not shared with ordinary operation execution. Return a typed `SafetyReceipt(applied, confirmed, reason, at_ms)`. Never invoke shutdown/reboot commands.

- [ ] **Step 3: Verify and commit**

```bash
.venv/bin/pytest tests/fault_injection/test_safety.py -q
```

Suggested subject: `feat(safety): enforce local fail-safe supervision`

### Task 8: gRPC server over the Provider contract

**Files:**
- Create: `src/aletheon_kuavo_bridge/grpc_service.py`
- Create: `src/aletheon_kuavo_bridge/server.py`
- Test: `tests/contract/test_grpc_service.py`

- [ ] **Step 1: Write in-process gRPC tests for all seven RPCs**

Use an ephemeral loopback port and Fake Provider. Assert version rejection, deadline rejection, capability response, snapshot, skill list, accepted/progress/result ordering, cancel, SafeStop, and health mapping. Assert error details contain no request payload.

- [ ] **Step 2: Implement service mapping**

Validate `protocol_version == "1.0"`, UUID operation IDs, non-empty device/skill IDs, deadline and lease before provider calls. Map domain exceptions to `ErrorDetail` and appropriate gRPC status without parsing exception display text. Bound outbound progress to configured Hz and always send terminal result.

- [ ] **Step 3: Implement server entrypoint**

`main()` accepts `--config`, loads config, constructs the selected provider, binds only the configured address, installs SIGINT/SIGTERM graceful shutdown, and returns non-zero when startup discovery fails. It must not launch MuJoCo itself.

- [ ] **Step 4: Verify and commit**

```bash
.venv/bin/pytest tests/contract/test_grpc_service.py -q
```

Suggested subject: `feat(grpc): serve the generic embodiment gateway`

---

## Stage G3 — Kuavo ROS Noetic MuJoCo adapter

### Task 9: Capture an authoritative ROS interface inventory

**Files:**
- Create: `scripts/discover-kuavo-interfaces.sh`
- Create: `docs/kuavo-mujoco-interface-inventory.md`
- Create: `config/kuavo_mujoco_interfaces.yaml`

- [ ] **Step 1: Start the standard simulation manually**

```bash
cd /home/aurobear/Workspace/kuavo-ros-control
source devel/setup.bash
roslaunch humanoid_controllers load_kuavo_mujoco_sim.launch
```

Expected: MuJoCo, controller, MPC and WBC remain running. If build artifacts/environment are missing, record the exact prerequisite and stop this task rather than changing Kuavo source.

- [ ] **Step 2: Inventory interfaces from the running graph**

The discovery script must save sorted outputs of `rosnode list`, `rostopic list`, `rosservice list`, and `rostopic type`/`rosservice type` for configured candidates. It must verify `/cmd_vel` is `geometry_msgs/Twist` and discover at least one fresh state source. Do not infer interface types from filenames alone.

- [ ] **Step 3: Write the checked inventory**

The Markdown inventory records command, observed type, publisher/subscriber or service provider, update rate, freshness behavior, and whether the interface is approved for `snapshot`, `stance`, `timed_move`, `stop`, or named arm action. The YAML contains only interfaces actually observed.

- [ ] **Step 4: Commit**

Suggested subject: `docs(kuavo): lock the MuJoCo ROS interface inventory`

### Task 10: ROS discovery and health adapter

**Files:**
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/discovery.py`
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/mappings.py`
- Test: `tests/unit/test_ros_discovery.py`

- [ ] **Step 1: Test with an injected ROS facade**

Assert ready only when ROS Master, required nodes, `/cmd_vel` type, and state source match the inventory; degraded for optional arm action absence; unavailable for missing control/state interfaces; recovery requires a fresh rediscovery generation.

- [ ] **Step 2: Implement without importing ROS in unit-test modules**

Put `rospy` imports behind `RosFacade` construction so normal pytest runs outside ROS. The production facade uses `rosgraph.Master`, `rospy.get_published_topics`, and service lookup. Return structured `DiscoveryReport` rather than booleans.

- [ ] **Step 3: Verify and commit**

```bash
.venv/bin/pytest tests/unit/test_ros_discovery.py -q
```

Suggested subject: `feat(kuavo): discover required MuJoCo interfaces`

### Task 11: Snapshot, stance, bounded movement and stop handlers

**Files:**
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/state.py`
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/stance.py`
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/move_base_timed.py`
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/stop.py`
- Create: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/provider.py`
- Test: `tests/unit/test_kuavo_provider.py`

- [ ] **Step 1: Test handlers through an injected ROS facade and fake clock**

Assert clamping rejects rather than silently truncates; timed movement publishes at a bounded rate and always ends with configured repeated zero commands; cancel/deadline/lease/disconnect take the same stop path; stance succeeds only after a fresh stable window; stale state fails; Provider contract suite passes.

- [ ] **Step 2: Implement mappings from the observed inventory**

Use `geometry_msgs.msg.Twist` only inside the production ROS facade. The handler must use monotonic time for durations. Put zero-command repetition count and interval in validated config. Never expose raw ROS messages through Provider return types.

- [ ] **Step 3: Verify outside ROS**

```bash
.venv/bin/pytest tests/unit/test_kuavo_provider.py tests/contract/test_provider_contract.py -q
```

Expected: all tests pass with Fake ROS.

- [ ] **Step 4: Commit**

Suggested subject: `feat(kuavo): add bounded MuJoCo movement provider`

### Task 12: Gate the named arm action on real discovery

**Files:**
- Create only if verified: `src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/execute_arm_action.py`
- Modify only if verified: `config/skills/kuavo_mujoco.yaml`
- Test: `tests/integration/test_arm_action.py`

- [ ] **Step 1: Query the actual service/action**

Use the inventory task to verify the running MuJoCo graph exposes the intended interface and inspect its request/response type. Execute one known-safe action manually and confirm fresh state/feedback.

- [ ] **Step 2A: If verified, implement allow-listed actions**

Register only the names manually proven in this environment. Reject every other name. Test success from observed response plus state evidence, cancel behavior where supported, and timeout.

- [ ] **Step 2B: If not verified, keep the skill unregistered**

Record the observed absence/type mismatch in the inventory. This is a valid G3 outcome and must not block stance/timed movement/stop, but P2 cannot claim arm-action coverage.

- [ ] **Step 3: Commit evidence and any verified implementation**

Suggested subject when implemented: `feat(kuavo): expose verified named arm actions`

---

## Stage G4 — Aletheon generic gRPC Provider

### Task 13: Add Rust protobuf generation to Hardware

**Files:**
- Modify: `/home/aurobear/Workspace/aletheon/Cargo.toml`
- Modify: `/home/aurobear/Workspace/aletheon/crates/hardware/Cargo.toml`
- Create: `/home/aurobear/Workspace/aletheon/crates/hardware/build.rs`
- Create: `/home/aurobear/Workspace/aletheon/crates/hardware/proto/aletheon/embodiment/gateway/v1/gateway.proto`
- Create: `/home/aurobear/Workspace/aletheon/crates/hardware/src/grpc/mod.rs`
- Test: `/home/aurobear/Workspace/aletheon/crates/hardware/tests/grpc_contract.rs`

- [ ] **Step 1: Copy the reviewed proto byte-for-byte and test its hash**

The contract test compares SHA-256 of the Bridge canonical proto and Hardware copy when the sibling repository is present; CI always compares against a committed expected hash. This prevents independent edits.

- [ ] **Step 2: Add dependencies**

Workspace dependencies: `tonic` with transport, `prost`, `prost-types`; Hardware build dependency `tonic-build`. Pin mutually compatible current versions selected by `cargo metadata`; commit `Cargo.lock`. Do not download dependencies in CI before the repository change has been reviewed.

- [ ] **Step 3: Generate client types**

`build.rs` compiles only the one proto and emits rerun directives. `grpc/mod.rs` uses `tonic::include_proto!("aletheon.embodiment.gateway.v1")` inside a private `wire` module.

- [ ] **Step 4: Verify**

```bash
cd /home/aurobear/Workspace/aletheon
bash scripts/cargo-agent.sh test -p hardware --test grpc_contract
```

Expected: PASS.

- [ ] **Step 5: Commit**

Suggested subject: `feat(hardware): generate embodiment gateway client contract`

### Task 14: Implement wire/domain conversions and stable error mapping

**Files:**
- Create: `crates/hardware/src/grpc/convert.rs`
- Create: `crates/hardware/src/grpc/error.rs`
- Modify: `crates/hardware/src/grpc/mod.rs`
- Test: `crates/hardware/tests/grpc_conversion.rs`

- [ ] **Step 1: Write table-driven tests**

Cover every risk, outcome and error enum; JSON Struct round-trip; timestamp bounds; missing required identity; unknown enum rejection; failure reason preservation; evidence refs; and ensure no conversion depends on Display strings.

- [ ] **Step 2: Implement explicit `TryFrom` conversions**

Do not implement protobuf types outside `hardware::grpc`. Map error codes directly to `ProviderError::{Disconnected,Rejected,Timeout}` while retaining category/code in structured tracing fields. Unknown wire enum values are protocol rejection, not defaults.

- [ ] **Step 3: Verify and commit**

```bash
bash scripts/cargo-agent.sh test -p hardware --test grpc_conversion
```

Suggested subject: `feat(hardware): map gateway wire types to domain DTOs`

### Task 15: Implement `GrpcEmbodimentProvider`

**Files:**
- Create: `crates/hardware/src/grpc/provider.rs`
- Modify: `crates/hardware/src/lib.rs`
- Test: `crates/hardware/tests/grpc_provider.rs`

- [ ] **Step 1: Write a tonic in-process fixture**

Test observe/get_state/list_skills, execute accepted/progress/result, progress forwarding, cancel, safe_stop, version mismatch, connect failure, RPC deadline, stream ending without terminal, wrong operation identity, and lease expiry projection.

- [ ] **Step 2: Implement typed provider config**

Define `GrpcProviderConfig { endpoint: http::Uri, protocol_version: String, connect_timeout: Duration, request_timeout: Duration, max_decoding_message_size: usize }`. Constructor connects explicitly and runs capabilities handshake. No environment reads and no default remote endpoint.

- [ ] **Step 3: Implement `EmbodimentProvider`**

Use the Hardware permit operation ID and lease expiry; never accept an operation ID from model parameters. Forward bounded progress to `SkillProgressSink`. Require exactly one terminal result and validate its device/skill/operation identity.

- [ ] **Step 4: Verify and commit**

```bash
bash scripts/cargo-agent.sh test -p hardware --test grpc_provider
```

Suggested subject: `feat(hardware): add generic gRPC embodiment provider`

### Task 16: Add typed Executive configuration and bootstrap selection

**Files:**
- Modify: `crates/executive/src/core/config/integrations.rs`
- Modify: `crates/executive/src/core/config/mod.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/embodiment.rs`
- Modify call site: `crates/executive/src/impl/daemon/bootstrap/request.rs`
- Update config examples/schema snapshots discovered by existing config tests
- Test: `crates/executive/tests/embodiment_provider_config.rs`

- [ ] **Step 1: Write config tests**

Assert default remains simulator, explicit `grpc` requires endpoint/device ID, non-loopback plaintext endpoint is rejected, unknown keys fail, secrets are not accepted in this config, and invalid configuration fails before daemon startup.

- [ ] **Step 2: Add tagged provider config**

Use a serde-tagged enum equivalent to:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmbodimentProviderConfig {
    Simulator { device_id: String },
    Grpc { device_id: String, endpoint: String, connect_timeout_ms: u64, request_timeout_ms: u64 },
}
```

Default is `Simulator { device_id: "bot" }`. Parsing and endpoint policy belong in config/bootstrap, not Hardware domain methods.

- [ ] **Step 3: Make bootstrap asynchronous and select provider**

Build a single registry and register either `SimulatedEmbodiment` or connected `GrpcEmbodimentProvider`. Preserve the existing Broker, Kernel admission, progress and settlement path. Log provider kind/device and a redacted endpoint authority only.

- [ ] **Step 4: Verify and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test embodiment_provider_config
bash scripts/cargo-agent.sh test -p executive --test embodiment_production_path
```

Expected: PASS; existing simulator behavior unchanged.

Suggested subject: `feat(executive): select typed embodiment providers at bootstrap`

---

## Stage G5/G6 — Cross-repository fault and end-to-end acceptance

### Task 17: Bridge integration harness and operational scripts

**Files:**
- Create: Bridge `scripts/bootstrap.sh`
- Create: Bridge `scripts/check.sh`
- Create: Bridge `scripts/run-mujoco-e2e.sh`
- Create: Bridge `launch/bridge.launch`
- Create: Bridge `docs/operations.md`

- [ ] **Step 1: Implement idempotent bootstrap**

Create/update `.venv`, install locked project dependencies, generate protobuf, and verify ROS Python imports after sourcing a caller-provided ROS setup path. Do not use `sudo` or modify Kuavo.

- [ ] **Step 2: Implement check script**

Run Ruff, mypy, all non-ROS pytest suites, proto regeneration diff check, config validation, and shell syntax checks. It must return non-zero on any failure.

- [ ] **Step 3: Implement E2E script**

Accept `--kuavo-workspace`, `--bridge-config`, and `--no-gui`. Start processes in a temporary runtime directory, wait with explicit deadlines, record PIDs, and terminate only processes it started. Sequence: ROS core/standard MuJoCo launch, discovery, Bridge, health, snapshot, list skills, stance, timed move, cancel case, SafeStop. Save bounded logs and a JSON result artifact.

- [ ] **Step 4: Document exact operator flow and commit**

Suggested subject: `feat(ops): add deterministic MuJoCo bridge lifecycle`

### Task 18: Fault-injection acceptance

**Files:**
- Create: Bridge `tests/fault_injection/test_grpc_disconnect.py`
- Create: Bridge `tests/fault_injection/test_ros_disconnect.py`
- Create: Bridge `tests/fault_injection/test_lease_expiry.py`
- Create: Aletheon `crates/executive/tests/embodiment_grpc_faults.rs`

- [ ] **Step 1: Verify client disconnect**

Drop the gRPC stream during timed movement. Assert movement stops no later than the lease/deadline boundary and terminal evidence remains in the Bridge registry.

- [ ] **Step 2: Verify ROS disconnect/recovery**

Use a test-controlled ROS facade for deterministic CI and repeat manually against MuJoCo by stopping only the ROS process started by the E2E harness. Assert unavailable, refusal of new skills, SafeStop attempt, and mandatory rediscovery generation on recovery.

- [ ] **Step 3: Verify Aletheon timeout/cancel/settlement**

Run the Rust fixture against a controllable Bridge server. Assert Kernel-issued operation identity reaches the Bridge, timeout/cancel results settle once, and no direct Corpus/Executive Kuavo dependency exists.

- [ ] **Step 4: Run focused suites and commit in each repository**

```bash
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
.venv/bin/pytest tests/fault_injection -q

cd /home/aurobear/Workspace/aletheon
bash scripts/cargo-agent.sh test -p executive --test embodiment_grpc_faults
```

Suggested subjects: `test(safety): cover bridge disconnect fail-safe`; `test(executive): cover gRPC embodiment faults`.

### Task 19: Live MuJoCo vertical-slice acceptance

**Files:**
- Create: Bridge `tests/integration/test_mujoco_e2e.py`
- Create: Bridge `docs/acceptance/kuavo-mujoco-p2.md`

- [ ] **Step 1: Run Bridge E2E**

```bash
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
scripts/run-mujoco-e2e.sh \
  --kuavo-workspace /home/aurobear/Workspace/kuavo-ros-control \
  --bridge-config config/bridge.example.yaml
```

Expected: JSON artifact reports ready health, increasing snapshot sequence, only manifest skills, stance success, bounded movement followed by zero velocity/stable state, cancel success, and idempotent SafeStop.

- [ ] **Step 2: Run Aletheon through the real Bridge**

Start Aletheon with explicit gRPC embodiment config and invoke the existing Corpus robot tools in order: status/observe, list skills, stance, timed move, cancel case, safe stop. Capture operation IDs and verify Executive settlement logs/results.

- [ ] **Step 3: Write acceptance evidence**

Record exact commits, config with secrets omitted, commands, timings, observed ROS interfaces, results, failures, and artifact hashes. Do not claim arm-action acceptance unless Task 12 passed live.

- [ ] **Step 4: Commit**

Suggested subject: `test(e2e): accept the Kuavo MuJoCo embodiment path`

---

## Stage G7 — Final verification and handoff

### Task 20: Deterministic final validation

- [ ] **Step 1: Bridge validation**

```bash
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
scripts/check.sh
```

Expected: formatting/lint/type/unit/contract/fault suites all PASS and generated code clean.

- [ ] **Step 2: Narrow Aletheon validation**

```bash
cd /home/aurobear/Workspace/aletheon
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p hardware
bash scripts/cargo-agent.sh test -p executive --test embodiment_service
bash scripts/cargo-agent.sh test -p executive --test embodiment_production_path
bash scripts/cargo-agent.sh test -p executive --test embodiment_provider_config
bash scripts/cargo-agent.sh test -p executive --test embodiment_grpc_faults
```

Expected: all PASS.

- [ ] **Step 3: Architecture checks**

```bash
cd /home/aurobear/Workspace/aletheon
! rg -n 'kuavo|rospy|ros::|geometry_msgs' crates/cognit crates/corpus crates/executive/src/service
rg -n 'GrpcEmbodimentProvider' crates/hardware crates/executive/src/impl/daemon/bootstrap
git diff --check origin/dev...HEAD
```

Expected: first command finds no forbidden dependency; provider references exist only in Hardware/bootstrap; diff check is clean.

- [ ] **Step 4: Workspace verification by the designated owner only**

```bash
bash scripts/cargo-agent.sh build --workspace
bash scripts/cargo-agent.sh test --workspace
```

Expected: PASS. Do not run concurrently with another Executive/workspace build.

- [ ] **Step 5: Repository and deployment handoff**

For each repository, report branch, commits, changed files, validation commands/results, rollback instructions, remaining limitations, and whether live arm action was verified. Push/create PR only when explicitly requested; target Aletheon `dev`. Never modify or commit in `kuavo-ros-control`.

---

## Requirement-to-task traceability

| Design requirement | Tasks |
|---|---|
| Vendor-neutral gRPC API | 2, 8, 13–15 |
| Typed config/no hardcoded endpoint | 3, 15, 16 |
| Snapshot aggregation/freshness | 5, 11, 19 |
| White-listed semantic skills | 3, 11, 12 |
| Operation idempotency/exactly-once | 4, 8, 15, 18 |
| Cancel/deadline/lease | 4, 7, 8, 11, 15, 18 |
| Local SafeStop | 7, 11, 18, 19 |
| ROS disconnect/recovery | 9, 10, 18 |
| No ROS/Kuavo above Hardware | 13–16, 20 |
| Provider replacement without Cognit/Corpus change | 16, 19, 20 |
| No Kuavo core modifications | 0, 9–12, 20 |

## Definition of done

This plan is complete only when Tasks 1–20 are checked with evidence, both repositories are clean, focused and owner-approved workspace validation pass, and live MuJoCo demonstrates the governed path through Aletheon settlement. Fake ROS tests alone are not P2 completion. Missing live arm-action support must be reported honestly but does not invalidate the verified stance/timed-move/stop vertical slice.
