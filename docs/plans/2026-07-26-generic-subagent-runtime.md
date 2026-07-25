# Generic Subagent Runtime Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `agent_spawn` select a manifested runtime from profile and caller capability requirements, while retaining an optional constrained runtime override.

**Architecture:** Add a provider-neutral spawn intent at the Fabric boundary, resolve it in an Executive-owned broker, and then pass the existing concrete `AgentSpawnRequest` into the unchanged admission and lifecycle path. Runtime adapters publish truthful manifests; Corpus exposes only generic profile, capability, task, workspace, tool, and budget fields.

**Tech Stack:** Rust, Serde, Schemars, Tokio, existing Fabric/Runtime/Executive/Corpus crates, systemd installed-runtime acceptance.

---

## File Map

- Modify `crates/fabric/src/types/agent_control.rs`: shared runtime-selection vocabulary.
- Modify `crates/fabric/src/lib.rs`: export shared selection vocabulary.
- Modify `crates/runtime/Cargo.toml`: consume the lower-level Fabric contract.
- Modify `crates/runtime/src/manifest.rs`: selectable runtime metadata and workspace intent.
- Modify `crates/runtime/src/selector.rs`: deterministic selection decision and rejection evidence.
- Modify `crates/runtime/src/lib.rs`: export the new selection types.
- Modify `crates/fabric/src/types/agent_control.rs`: generic spawn intent and control-port method.
- Modify `crates/fabric/src/lib.rs`: export generic intent types.
- Modify `crates/executive/src/application/agent_control/execution.rs`: manifested catalog and broker resolution.
- Modify `crates/executive/src/application/agent_control/mod.rs`: resolve intent before canonical spawn.
- Modify `crates/executive/src/composition/config/mod.rs`: configurable profile runtime requirements.
- Modify `crates/executive/src/host/daemon/bootstrap/services.rs`: inject profile selection requirements.
- Modify `crates/executive/src/host/daemon/bootstrap/request.rs`: manifested registration for shipped runtimes.
- Modify `crates/executive/src/adapters/runtime/native_cognit.rs`: native runtime manifest.
- Modify `crates/executive/src/adapters/runtime/pi.rs`: Pi coding manifest.
- Modify `crates/corpus/src/tools/tools/agent_control.rs`: provider-neutral tool schema and execution.
- Modify `crates/corpus/tests/agent_control_tools.rs`: schema, generic spawn, and compatibility tests.
- Modify `crates/executive/tests/agent_control_spawn.rs`: broker/service contract tests.
- Modify `crates/executive/tests/pi_rpc_runtime.rs`: manifest selector compatibility.
- Modify `crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs`: configuration/bootstrap tests.
- Add `docs/testing/generic-subagent-runtime-acceptance.md`: installed acceptance evidence template.

### Task 1: Extend the runtime manifest without provider branches

**Files:**
- Modify: `crates/fabric/src/types/agent_control.rs`
- Modify: `crates/fabric/src/lib.rs`
- Modify: `crates/runtime/Cargo.toml`
- Modify: `crates/runtime/src/manifest.rs`
- Modify: `crates/runtime/src/lib.rs`
- Test: `crates/runtime/src/manifest.rs`

- [ ] **Step 1: Write failing serialization and validation tests**

Add tests that construct a manifest supporting read-only shared work and a
natural-language task, round-trip it through JSON, and reject an empty task
encoding set:

```rust
#[test]
fn selectable_manifest_round_trips_generic_constraints() {
    let manifest = RuntimeManifest {
        id: "analysis-a".into(),
        aliases: vec!["analysis".into()],
        display_name: "Analysis A".into(),
        capabilities: BTreeSet::from([
            RuntimeCapability::CodeRead,
            RuntimeCapability::CodeSearch,
        ]),
        interaction_modes: BTreeSet::from([InteractionMode::Resident]),
        workspace_modes: BTreeSet::from([WorkspaceMode::SharedReadOnly]),
        task_encodings: BTreeSet::from([TaskEncoding::NaturalLanguage]),
        tool_governance: ToolGovernance::Observed,
        priority: 10,
        max_context_tokens: Some(1_000_000),
        resource_requirements: Default::default(),
    };
    manifest.validate().unwrap();
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert_eq!(encoded["priority"], 10);
    assert_eq!(
        serde_json::from_value::<RuntimeManifest>(encoded)
            .unwrap()
            .workspace_modes,
        manifest.workspace_modes
    );
}

#[test]
fn selectable_manifest_requires_an_input_encoding() {
    let mut manifest = selectable_manifest_round_trip_fixture();
    manifest.task_encodings.clear();
    assert_eq!(
        manifest.validate().unwrap_err(),
        "runtime task encodings must not be empty"
    );
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
bash scripts/cargo-agent.sh test -p runtime manifest::tests -- --nocapture
```

Expected: compilation fails because `TaskEncoding`, `workspace_modes`,
`priority`, `max_context_tokens`, and `RuntimeManifest::validate` do not exist.

- [ ] **Step 3: Add the generic metadata**

Define the provider-neutral selection vocabulary in Fabric so Corpus,
Executive, and Runtime use one contract without making Fabric depend on the
higher-level Runtime crate:

```rust
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeCapability {
    CodeRead,
    CodeSearch,
    CodeEdit,
    Shell,
    Test,
    Git,
    Diagnostics,
    Browser,
    DeviceObserve,
    DeviceCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    OneShot,
    Resident,
    Steering,
    FollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    SharedReadOnly,
    SharedWritable,
    IsolatedWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaskEncoding {
    NaturalLanguage,
    StructuredJson,
}
```

Add `fabric = { path = "../fabric" }` to `crates/runtime/Cargo.toml`. Remove the
local `RuntimeCapability`, `InteractionMode`, `WorkspaceMode`, and
`TaskEncoding` definitions from Runtime, import them from Fabric, and re-export
them from `runtime::lib` for source compatibility using
`pub use fabric::AgentRuntimeCapability as RuntimeCapability`.

Replace singular workspace support with a set and add bounded selection
metadata:

```rust

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub id: String,
    pub aliases: Vec<String>,
    pub display_name: String,
    pub capabilities: BTreeSet<RuntimeCapability>,
    pub interaction_modes: BTreeSet<InteractionMode>,
    pub workspace_modes: BTreeSet<WorkspaceMode>,
    pub task_encodings: BTreeSet<TaskEncoding>,
    pub tool_governance: ToolGovernance,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub max_context_tokens: Option<u64>,
    #[serde(default)]
    pub resource_requirements: RuntimeResourceRequirements,
}

impl RuntimeManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("runtime id must not be empty".into());
        }
        if self.interaction_modes.is_empty() {
            return Err("runtime interaction modes must not be empty".into());
        }
        if self.workspace_modes.is_empty() {
            return Err("runtime workspace modes must not be empty".into());
        }
        if self.task_encodings.is_empty() {
            return Err("runtime task encodings must not be empty".into());
        }
        if self.max_context_tokens == Some(0) {
            return Err("runtime context limit must be nonzero".into());
        }
        self.resource_requirements.validate()?;
        Ok(())
    }
}
```

Export `TaskEncoding` with the existing runtime types from `lib.rs`.

- [ ] **Step 4: Run the focused tests**

Run the Task 1 command again.

Expected: all manifest tests pass.

- [ ] **Step 5: Commit the manifest contract**

```bash
git add crates/fabric/src/types/agent_control.rs crates/fabric/src/lib.rs \
  crates/runtime/Cargo.toml crates/runtime/src/manifest.rs crates/runtime/src/lib.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(runtime): describe selectable subagent contracts

Automatic subagent selection needs truthful workspace, input, priority, and
context metadata instead of concrete runtime-name branches.

- add generic workspace and task-encoding sets
- validate selectable manifest invariants
- expose bounded priority and context metadata
EOF
```

### Task 2: Return deterministic selection evidence

**Files:**
- Modify: `crates/runtime/src/selector.rs`
- Modify: `crates/runtime/src/lib.rs`
- Test: `crates/runtime/src/selector.rs`

- [ ] **Step 1: Add failing order-independence and override tests**

```rust
#[test]
fn selection_uses_priority_then_id_independent_of_insertion_order() {
    let low = manifest("z-low", 20);
    let high = manifest("a-high", 10);
    let request = RuntimeSelectionRequest {
        selector: RuntimeSelector::Auto,
        required_capabilities: vec![RuntimeCapability::CodeRead],
        interaction_mode: InteractionMode::Resident,
        workspace_mode: WorkspaceMode::SharedReadOnly,
        task_encoding: TaskEncoding::NaturalLanguage,
        max_input_tokens: 100,
    };
    let left = request.select([&low, &high]).unwrap();
    let right = request.select([&high, &low]).unwrap();
    assert_eq!(left.selected_runtime_id, "a-high");
    assert_eq!(left, right);
}

#[test]
fn alias_override_cannot_bypass_capabilities() {
    let runtime = manifest("analysis", 0);
    let mut request = selection_request();
    request.selector = RuntimeSelector::Alias("analysis".into());
    request.required_capabilities = vec![RuntimeCapability::CodeEdit];
    let error = request.select([&runtime]).unwrap_err();
    assert!(error.rejections[0].reasons.contains(&"missing capability: CodeEdit".into()));
}
```

- [ ] **Step 2: Run and verify failure**

```bash
bash scripts/cargo-agent.sh test -p runtime selector::tests -- --nocapture
```

Expected: compilation fails because selection request/decision types do not
exist.

- [ ] **Step 3: Implement request, decision, and bounded rejection types**

Add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSelectionRequest {
    pub selector: RuntimeSelector,
    pub required_capabilities: Vec<RuntimeCapability>,
    pub interaction_mode: InteractionMode,
    pub workspace_mode: WorkspaceMode,
    pub task_encoding: TaskEncoding,
    pub max_input_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCandidateRejection {
    pub runtime_id: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelectionDecision {
    pub selected_runtime_id: String,
    pub effective_capabilities: Vec<RuntimeCapability>,
    pub override_used: bool,
    pub reason: String,
    pub rejections: Vec<RuntimeCandidateRejection>,
}
```

Implement `select` as a pure function: validate all candidates, collect at most
64 sorted rejection records with at most 16 reasons each, sort compatible
candidates by `(priority, id)`, and return an error decision when none match.
Do not launch, probe, retry, or perform I/O.

- [ ] **Step 4: Run selector and runtime tests**

```bash
bash scripts/cargo-agent.sh test -p runtime --lib -- --nocapture
```

Expected: all runtime library tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/selector.rs crates/runtime/src/lib.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(runtime): select subagents by capabilities

Runtime selection must be deterministic, fail closed, and explain why
candidates were rejected without probing providers.

- match capability, interaction, workspace, input, and context constraints
- rank compatible manifests by configured priority and stable ID
- return bounded selection and rejection evidence
EOF
```

### Task 3: Add a provider-neutral Fabric spawn intent

**Files:**
- Modify: `crates/fabric/src/types/agent_control.rs`
- Modify: `crates/fabric/src/lib.rs`
- Test: `crates/fabric/tests/agent_control_contract.rs`

- [ ] **Step 1: Add failing intent validation tests**

Test an omitted override, a valid alias override, duplicate capabilities, and a
zero budget. The canonical selected request remains unchanged.

```rust
#[test]
fn generic_spawn_intent_accepts_automatic_selection() {
    let intent = generic_intent(None);
    intent.validate().unwrap();
    assert!(intent.runtime_override.is_none());
}

#[test]
fn generic_spawn_intent_rejects_duplicate_capabilities() {
    let mut intent = generic_intent(None);
    intent.required_capabilities = vec![
        AgentRuntimeCapability::CodeRead,
        AgentRuntimeCapability::CodeRead,
    ];
    assert_eq!(
        intent.validate().unwrap_err().kind,
        AgentControlErrorKind::InvalidRequest
    );
}
```

- [ ] **Step 2: Run and verify failure**

```bash
bash scripts/cargo-agent.sh test -p fabric --test agent_control_contract -- --nocapture
```

Expected: compilation fails because `AgentSpawnIntent` does not exist.

- [ ] **Step 3: Implement the generic intent and port method**

Add an intent mirroring caller-owned spawn fields while leaving
`AgentSpawnRequest.runtime_id` concrete:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpawnIntent {
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub parent_process_id: Option<ProcessId>,
    pub profile_id: AgentProfileId,
    pub runtime_override: Option<String>,
    pub required_capabilities: Vec<AgentRuntimeCapability>,
    #[serde(skip)]
    pub trusted_workspace: Option<crate::WorkspacePolicy>,
    pub task: String,
    pub context: AgentContextFork,
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
}
```

`validate` must apply the existing profile/task/context/tool/budget limits,
validate a nonblank override when present, sort a clone of capabilities, and
reject duplicates.

Add to `AgentControlPort`:

```rust
async fn spawn_intent(
    &self,
    _intent: AgentSpawnIntent,
) -> Result<AgentHandle, AgentControlError> {
    Err(AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: "generic subagent selection is unavailable".into(),
    })
}
```

The default keeps non-production test ports source-compatible; production
`AgentControlService` overrides it.

- [ ] **Step 4: Run Fabric tests**

```bash
bash scripts/cargo-agent.sh test -p fabric --test agent_control_contract -- --nocapture
bash scripts/cargo-agent.sh test -p fabric --test protocol_schema -- --nocapture
```

Expected: both test targets pass.

- [ ] **Step 5: Commit**

```bash
git add crates/fabric/src/types/agent_control.rs crates/fabric/src/lib.rs crates/fabric/tests/agent_control_contract.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(fabric): add generic subagent spawn intent

Callers need to express runtime requirements without weakening the canonical
durable request, which must retain a selected runtime ID.

- add validated provider-neutral spawn intent
- preserve the selected AgentSpawnRequest contract
- add a compatible control-port entry point
EOF
```

### Task 4: Implement the Executive broker and profile requirement catalog

**Files:**
- Modify: `crates/executive/src/application/agent_control/execution.rs`
- Modify: `crates/executive/src/application/agent_control/mod.rs`
- Modify: `crates/executive/src/composition/config/mod.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/services.rs`
- Test: `crates/executive/tests/agent_control_spawn.rs`
- Test: `crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs`

- [ ] **Step 1: Add failing broker tests**

Register two manifested fake launchers in reverse order. Configure the profile
with `CodeRead`, add caller `CodeSearch`, and assert the lower-priority
compatible runtime is persisted in the returned handle. Add separate tests
that an override lacking `CodeSearch` fails before either launcher is called,
and an unmanifested launcher is ignored by automatic selection.

- [ ] **Step 2: Run and verify failure**

```bash
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn -- --nocapture
```

Expected: tests fail because `AgentControlService` uses only exact
`runtime_id` resolution.

- [ ] **Step 3: Add configurable profile requirements**

Extend `ProfileOverride`:

```rust
#[serde(default)]
pub runtime_capabilities: Vec<fabric::AgentRuntimeCapability>,
#[serde(default)]
pub runtime_priority: Vec<String>,
```

At bootstrap, build a `HashMap<AgentProfileId, Vec<RuntimeCapability>>` from
the reviewed profile overrides and pass it through:

```rust
.with_runtime_profile_requirements(profile_runtime_requirements)
```

Empty configuration means no implicit runtime capability; it never invents
write or shell authority. Caller capabilities are unioned with configured
profile requirements.

- [ ] **Step 4: Implement broker resolution**

Add registry methods:

```rust
pub fn catalog(&self) -> Vec<runtime::RuntimeManifest>;

pub fn select(
    &self,
    request: &runtime::RuntimeSelectionRequest,
) -> Result<(RuntimeId, Arc<dyn AgentRuntimeLauncher>, runtime::RuntimeSelectionDecision), AgentControlError>;
```

`register_manifested` must call `manifest.validate()` before registering.

Override `spawn_intent` in `AgentControlService`. Validate the intent, merge
profile and caller capabilities, derive workspace intent from
`trusted_workspace.writable_roots().is_empty()`, select a natural-language
resident runtime, log a structured decision, construct the concrete
`AgentSpawnRequest`, and call `self.spawn(request).await`.

The selected runtime ID must be set before request hashing, admission,
persistence, and process creation. Selection failure must leave repository and
launcher counters unchanged.

- [ ] **Step 5: Run focused Executive tests**

```bash
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn -- --nocapture
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::runtime::runtime_tests --lib -- --nocapture
```

Expected: all focused broker and bootstrap tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/application/agent_control/execution.rs \
  crates/executive/src/application/agent_control/mod.rs \
  crates/executive/src/composition/config/mod.rs \
  crates/executive/src/host/daemon/bootstrap/services.rs \
  crates/executive/tests/agent_control_spawn.rs \
  crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(agent): broker generic subagent requests

The public spawn path currently bypasses manifested runtime selection. Resolve
profile and caller requirements before the existing durable lifecycle begins.

- add configurable profile runtime requirements
- select manifested launchers deterministically
- preserve canonical admission, persistence, and lifecycle ownership
EOF
```

### Task 5: Make the Corpus tool contract runtime-neutral

**Files:**
- Modify: `crates/corpus/src/tools/tools/agent_control.rs`
- Modify: `crates/corpus/tests/agent_control_tools.rs`

- [ ] **Step 1: Replace Pi-specific expectations with failing generic tests**

Assert:

```rust
let spawn = find(&tools, "agent_spawn");
let serialized = serde_json::to_string(&spawn.input_schema()).unwrap();
assert!(!spawn.description().contains("pi"));
assert!(!serialized.contains("pi-rpc"));
assert!(!serialized.contains("pi-coder"));
assert!(!serialized.contains("codex"));
assert!(!serialized.contains("claude"));
assert!(!spawn.input_schema()["required"]
    .as_array().unwrap()
    .contains(&serde_json::json!("runtime")));
```

Add one automatic request with `required_capabilities` and one legacy request
with `runtime`; assert both reach `spawn_intent` and preserve trusted identity
and workspace injection.

- [ ] **Step 2: Run and verify failure**

```bash
bash scripts/cargo-agent.sh test -p corpus --test agent_control_tools -- --nocapture
```

Expected: the schema still requires `runtime`, contains Pi text, and calls
`spawn` rather than `spawn_intent`.

- [ ] **Step 3: Implement generic schema and input**

Use:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnInput {
    profile: String,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    required_capabilities: Vec<fabric::AgentRuntimeCapability>,
    task: String,
    #[serde(default)]
    context: AgentContextFork,
    #[serde(default)]
    tools: Vec<String>,
    budget: AgentBudget,
}
```

Describe spawn as:

```text
Spawn a bounded child Agent. The runtime is selected automatically from the
profile and required capabilities; runtime is an optional constrained
diagnostic override.
```

Expose snake-case capability enum values, make `runtime` optional, and call
`control.spawn_intent(AgentSpawnIntent { ... })`. Keep trusted root, parent,
process, and workspace values host-injected.

- [ ] **Step 4: Run Corpus tests**

```bash
bash scripts/cargo-agent.sh test -p corpus --test agent_control_tools -- --nocapture
```

Expected: all five tool-contract tests pass with no provider name in the spawn
schema.

- [ ] **Step 5: Commit**

```bash
git add crates/corpus/src/tools/tools/agent_control.rs crates/corpus/tests/agent_control_tools.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(tools): expose generic subagent selection

Agent callers should describe required work rather than memorize Pi-specific
runtime IDs and task encodings.

- make runtime an optional constrained override
- expose provider-neutral capability requirements
- preserve host-injected identity and workspace authority
EOF
```

### Task 6: Manifest every shipped selectable runtime

**Files:**
- Modify: `crates/executive/src/adapters/runtime/native_cognit.rs`
- Modify: `crates/executive/src/adapters/runtime/pi.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs`
- Modify: `crates/executive/tests/pi_rpc_runtime.rs`
- Test: `crates/executive/tests/native_cognit_runtime.rs`

- [ ] **Step 1: Add failing manifest contract tests**

Assert native Cognit and Pi RPC accept natural-language resident read-only
analysis. Assert Pi coder declares structured JSON, code-edit/test/shell, and
isolated-worktree requirements. Assert no launcher registered only with
`register` is returned by the automatic catalog.

- [ ] **Step 2: Run and verify failure**

```bash
bash scripts/cargo-agent.sh test -p executive --test pi_rpc_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime -- --nocapture
```

Expected: native Cognit and Pi coder do not yet expose manifests.

- [ ] **Step 3: Add adapter-owned manifests and manifested registration**

Each adapter returns a `RuntimeManifest`; bootstrap passes that exact manifest
to `register_manifested`. The broker contains no runtime IDs. Goal compatibility
runtimes remain explicit-only until their owning adapters publish complete
manifests.

Use priorities from configuration, not provider-specific selection branches.
Default priorities are equal so stable runtime ID is the deterministic
tie-breaker.

- [ ] **Step 4: Run adapter and bootstrap tests**

```bash
bash scripts/cargo-agent.sh test -p executive --test pi_rpc_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::runtime::runtime_tests --lib -- --nocapture
```

Expected: all tests pass; catalog contains every manifested shipped subagent
runtime and excludes explicit-only compatibility routes.

- [ ] **Step 5: Commit**

```bash
git add crates/executive/src/adapters/runtime/native_cognit.rs \
  crates/executive/src/adapters/runtime/pi.rs \
  crates/executive/src/host/daemon/bootstrap/request.rs \
  crates/executive/tests/pi_rpc_runtime.rs \
  crates/executive/tests/native_cognit_runtime.rs
git diff --cached --check
git commit -F - <<'EOF'
feat(agent): publish shipped runtime manifests

Automatic selection must rely on adapter-owned facts rather than hard-coded
knowledge in the broker or tool schema.

- manifest native and Pi runtime capabilities and input contracts
- register selectable launchers with validated manifests
- leave incomplete compatibility routes explicit-only
EOF
```

### Task 7: Run focused regression validation

**Files:**
- Test: `crates/runtime/src/`
- Test: `crates/fabric/tests/agent_control_contract.rs`
- Test: `crates/corpus/tests/agent_control_tools.rs`
- Test: `crates/executive/tests/agent_control_spawn.rs`
- Test: `crates/executive/tests/agent_control_operations.rs`
- Test: `crates/executive/tests/agent_recovery.rs`

- [ ] **Step 1: Format-check**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: exit status 0.

- [ ] **Step 2: Run the bounded regression set serially**

```bash
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p fabric --test agent_control_contract
bash scripts/cargo-agent.sh test -p corpus --test agent_control_tools
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn
bash scripts/cargo-agent.sh test -p executive --test agent_control_operations
bash scripts/cargo-agent.sh test -p executive --test agent_recovery
```

Expected: every command exits 0. Do not run these commands concurrently.

- [ ] **Step 3: Inspect the complete diff**

```bash
git status --short
git diff --check
git diff --stat HEAD~6..HEAD
```

Expected: only files named in this plan are changed; no generated credentials,
state databases, user-local assets, or runtime logs are tracked.

### Task 8: Perform installed-runtime acceptance

**Files:**
- Add: `docs/testing/generic-subagent-runtime-acceptance.md`

- [ ] **Step 1: Deploy only the system installation**

```bash
sudo bash scripts/aletheon.sh deploy
```

Expected: deployment verification passes. Do not run the non-sudo deployment
and do not create `~/.local/bin/aletheon`.

- [ ] **Step 2: Verify provenance and service stability**

```bash
sha256sum target/release/aletheon /usr/bin/aletheon
systemctl show aletheon-core.service -p ExecMainPID -p NRestarts
systemctl --user show aletheon.service -p ExecMainPID -p NRestarts
readlink -f /proc/"$(systemctl show aletheon-core.service -p MainPID --value)"/exe
readlink -f /proc/"$(systemctl --user show aletheon.service -p MainPID --value)"/exe
```

Expected: target, installed binary, and both running executables have identical
SHA-256 digests; both restart counters remain unchanged across a seven-second
observation.

- [ ] **Step 3: Run three consecutive real-TUI generic requests**

Launch `/usr/bin/aletheon` against the official user socket. In each fresh TUI,
ask it to spawn a read-only repository-analysis child without specifying a
runtime, wait for completion, and summarize the result. Repeat three times.

Expected for every run:

- a manifested runtime is selected;
- the child completes without caller knowledge of a runtime ID;
- only the addressed parent receives child output;
- no rendered inference error appears;
- Ctrl+C remains responsive;
- session evidence identifies a distinct parent and child.

- [ ] **Step 4: Correlate logs, sessions, and accounting**

Record for each run:

```text
parent_session_id:
child_agent_id:
selected_runtime_id:
model_inference_rounds:
provider_retries:
tool_calls:
input_tokens:
output_tokens:
elapsed_ms:
rendered_error:
daemon_error:
```

Search daemon logs and durable session evidence for
`provider_unavailable`, `provider_rejected_request`, `rate_limit`, and
`inference provider failed`.

Expected: no error match; provider retries are not conflated with inference
rounds or tool calls; one spawn causes one launcher invocation.

- [ ] **Step 5: Write and commit acceptance evidence**

Write exact commands, hashes, PIDs, restart counters, session IDs, selected
runtimes, accounting, and log conclusions to
`docs/testing/generic-subagent-runtime-acceptance.md`.

```bash
git add docs/testing/generic-subagent-runtime-acceptance.md
git diff --cached --check
git commit -F - <<'EOF'
test(agent): record generic subagent acceptance

Generic runtime selection affects the installed tool, daemon bootstrap, and
real model behavior, so source-level tests alone are insufficient.

- record installed binary and daemon provenance
- capture three consecutive real-TUI generic subagent runs
- separate inference, retry, tool, token, and elapsed evidence
EOF
```
