# Governed TUI Commands Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose only user-owned TUI commands and make reflection, evolution, evaluation, and hook orchestration reachable only through typed internal lifecycle paths.

**Architecture:** `CommandRegistry` remains the single source for TUI parsing, help, and completion. Internal governance RPC methods are removed from the public client binding and daemon dispatcher, while existing internal reflection/evolution coordinators and evidence stores remain unchanged. Typed evaluation metadata used by host tests remains available, but no slash command can trigger it.

**Tech Stack:** Rust, Clap, serde/schemars Fabric protocol, Tokio JSON-RPC, ratatui TUI, existing Cognit/Executive lifecycle tests.

**Approved design:** `docs/plans/2026-08-03-governed-commands-extension-runtime-design.md:58-138`

---

### Task 1: Freeze the exact public built-in command contract

**Files:**
- Modify: `crates/interact/src/tui/registry.rs:786-822`
- Test: `crates/interact/src/tui/registry.rs`

- [ ] **Step 1: Replace the permissive presence test with an exact-set test**

```rust
#[test]
fn public_builtin_command_set_is_exact() {
    let registry = CommandRegistry::new();
    let actual = registry
        .builtins()
        .iter()
        .map(|command| command.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            "agent", "agents", "clear", "compact", "context", "copy", "diff",
            "fork", "help", "input", "interrupt", "memory", "mention", "mode",
            "model", "new", "permissions", "profile", "quit", "resume", "sessions",
            "skills", "status",
        ]
    );
    for retired in [
        "reflect", "r", "reflect_now", "rn", "evolution", "evo", "genome",
        "gene", "hooks", "hk", "task", "evaluation", "eval", "approve", "a",
        "plan", "p", "computer",
    ] {
        assert!(!registry.is_builtin(retired), "retired /{retired} remains public");
    }
}
```

- [ ] **Step 2: Run the test and confirm it fails with the current extra commands**

Run:

```bash
bash scripts/cargo-agent.sh test -p interact --lib public_builtin_command_set_is_exact -- --nocapture
```

Expected: `FAILED`; `actual` contains at least `approve`, `computer`, `evaluation`, `evolution`, `genome`, `hooks`, `plan`, `reflect`, `reflect_now`, and `task`.

- [ ] **Step 3: Commit the failing contract test**

```bash
git add crates/interact/src/tui/registry.rs
git commit -F - <<'MSG'
test(tui): freeze public command surface

Internal governance commands are currently mixed into ordinary session
commands, so help and completion expose operations users should not trigger.

- assert the exact supported built-in command set
- reject internal command names and aliases as public built-ins
MSG
```

### Task 2: Remove internal commands from registry and parser types

**Files:**
- Modify: `crates/interact/src/tui/registry.rs:35-68,104-414,704-734`
- Modify: `crates/interact/src/tui/command.rs:4-53,126-275`
- Test: `crates/interact/src/tui/command.rs`

- [ ] **Step 1: Add parser rejection coverage**

```rust
#[test]
fn retired_governance_commands_parse_as_unknown() {
    for input in [
        "/reflect", "/r", "/reflect_now", "/rn", "/evolution", "/evo",
        "/genome", "/gene", "/hooks", "/hk", "/task coding", "/evaluation",
        "/eval", "/approve", "/a", "/plan", "/p", "/computer",
    ] {
        assert!(
            matches!(parse_command(input), Some(CommandType::Unknown { .. })),
            "{input} still resolves as a built-in"
        );
    }
}
```

- [ ] **Step 2: Run and confirm the new parser test fails**

Run:

```bash
bash scripts/cargo-agent.sh test -p interact --lib retired_governance_commands_parse_as_unknown -- --nocapture
```

Expected: `FAILED` because the first retired command resolves as `Builtin`.

- [ ] **Step 3: Delete the retired enum IDs and descriptors**

Remove these `BuiltinId` variants and their `CommandDescriptor::builtin(...)` entries:

```rust
Task,
Evaluation,
Reflect,
ReflectNow,
Evolution,
Genome,
Plan,
Approve,
Hooks,
Computer,
```

Remove the corresponding arms from `to_builtin`. Delete these variants from `BuiltinCommand`:

```rust
Reflect,
ReflectNow,
Evolution,
Genome,
Computer { args: String },
Plan,
Approve,
Hooks,
Task { kind: String },
Evaluation,
```

Delete the obsolete positive parser tests and retain the new unknown-command test.

- [ ] **Step 4: Run registry and parser tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p interact --lib tui::registry::tests -- --nocapture
bash scripts/cargo-agent.sh test -p interact --lib tui::command::tests -- --nocapture
```

Expected: both test groups pass.

- [ ] **Step 5: Commit registry/parser removal**

```bash
git add crates/interact/src/tui/registry.rs crates/interact/src/tui/command.rs
git commit -F - <<'MSG'
refactor(tui): retire internal governance commands

Reflection, evolution, evaluation, and hook orchestration belong to typed host
lifecycle policy rather than the user command registry.

- remove internal operations from parsing, help, and completion
- delete aliases that could bypass the reduced command surface
- preserve unknown-command diagnostics for retired names
MSG
```

### Task 3: Remove TUI and fallback dispatch paths

**Files:**
- Modify: `crates/interact/src/tui/app/submit.rs:130-175,300-365,425-470`
- Modify: `crates/interact/src/tui/app/lifecycle.rs:248-285`
- Modify: `crates/interact/src/tui/response.rs:430-455,790-925,990-1050`
- Modify: `crates/interact/src/tui/mod.rs:430-470`
- Test: `crates/interact/src/tui/app/submit.rs`

- [ ] **Step 1: Add a registry-driven dispatch contract**

Add a pure helper beside `submit_message`:

```rust
#[cfg(test)]
fn retired_command_has_no_builtin_dispatch(input: &str) -> bool {
    matches!(
        super::super::CommandRegistry::new().parse(input),
        Some(CommandType::Unknown { .. })
    )
}

#[test]
fn internal_governance_text_has_no_tui_dispatch() {
    for command in ["/reflect", "/reflect_now", "/evolution", "/genome", "/hooks"] {
        assert!(retired_command_has_no_builtin_dispatch(command));
    }
}
```

- [ ] **Step 2: Remove unreachable submit branches and pending response formatting**

Delete `BuiltinCommand` match arms for all retired variants. Delete response-only formatters that no remaining public RPC or internal event uses; keep event rendering for autonomous reflection/evolution events. Remove retired names from the line-mode fallback mapping in `app/lifecycle.rs` so they flow to ordinary unknown-command handling.

- [ ] **Step 3: Run all Interact library tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p interact --lib -- --nocapture
```

Expected: all tests pass and there are no non-exhaustive match failures.

- [ ] **Step 4: Commit dispatch cleanup**

```bash
git add crates/interact/src/tui/app/submit.rs crates/interact/src/tui/app/lifecycle.rs crates/interact/src/tui/response.rs crates/interact/src/tui/mod.rs
git commit -F - <<'MSG'
refactor(tui): remove retired command dispatch

Registry removal must also eliminate compatibility paths that can still send
internal governance RPCs from alternate TUI entry points.

- delete retired submission and fallback mappings
- remove command-only pending response handling
- retain autonomous governance event rendering
MSG
```

### Task 4: Remove public CLI subcommands and client request variants

**Files:**
- Modify: `crates/interact/src/tui/cli.rs:109-135,309-323,540-560`
- Modify: `crates/fabric/src/protocol/client.rs:22-55,680-720`
- Rename: `crates/fabric/tests/client_rpc_reflect.rs` to `crates/fabric/tests/client_rpc_public_surface.rs`
- Test: `crates/interact/src/tui/cli.rs`

- [ ] **Step 1: Add serialization rejection/absence assertions**

Replace the old positive reflection serialization test with a source-level
public-surface guard (all semantic request behavior remains covered by Fabric's
unit tests):

```rust
#[test]
fn client_protocol_does_not_publish_manual_governance_methods() {
    let source = include_str!("../src/protocol/client.rs");
    for method in ["reflect", "reflect_now", "evolution", "genome", "hooks_list"] {
        assert!(!source.contains(&format!("\"{method}\"")), "published retired method {method}");
    }
}
```

- [ ] **Step 2: Confirm the protocol test fails**

Run:

```bash
bash scripts/cargo-agent.sh test -p fabric --test client_rpc_public_surface client_protocol_does_not_publish_manual_governance_methods -- --nocapture
```

Expected: `FAILED` while the current schema publishes at least `reflect`.

- [ ] **Step 3: Remove public CLI and request bindings**

Delete `Command::{Reflect, ReflectNow, Evolution, Genome}`, their Clap aliases,
`handle_command` arms, and line-mode request mappings. Delete
`ClientRpcRequest::{Reflect, ReflectNow, ReflectNowFor, Evolution, Genome,
HooksList}` and their `to_json_rpc` arms. Keep `EvaluationGet`,
`EvaluationLatest`, and `EvaluationList` as typed host/test evidence APIs, but do
not map them from slash commands.

- [ ] **Step 4: Run Fabric and Interact tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p fabric --test client_rpc_public_surface -- --nocapture
bash scripts/cargo-agent.sh test -p interact --lib -- --nocapture
```

Expected: both pass.

- [ ] **Step 5: Commit protocol contraction**

```bash
git add crates/interact/src/tui/cli.rs crates/fabric/src/protocol/client.rs crates/fabric/tests/client_rpc_reflect.rs crates/fabric/tests/client_rpc_public_surface.rs
git commit -F - <<'MSG'
refactor(protocol): make governance triggers host-internal

Manual reflection and evolution methods allow clients to bypass the lifecycle
policies that own evidence, budgeting, and settlement.

- remove governance subcommands from the public CLI
- remove manual trigger variants from the client protocol
- retain typed evaluation evidence APIs without slash exposure
MSG
```

### Task 5: Remove daemon routes while preserving automatic governance

**Files:**
- Modify: `crates/executive/src/host/daemon/handler/rpc.rs:111-136`
- Modify: `crates/executive/src/host/daemon/handler/rpc/rpc_reflection.rs`
- Modify: `crates/executive/src/host/daemon/handler/rpc/mod.rs`
- Test: `crates/executive/tests/evolution_integration.rs`
- Test: `crates/executive/tests/self_evolution_loop_test.rs`
- Create: `crates/executive/tests/retired_governance_rpc.rs`

- [ ] **Step 1: Add a daemon-level unknown-method test**

Use the existing authenticated daemon test harness and assert each request returns JSON-RPC `-32601`:

```rust
for method in ["reflect", "reflect_now", "evolution", "genome", "hooks_list"] {
    let response = harness.rpc(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": method
    })).await;
    assert_eq!(response["error"]["code"], -32601, "{method}");
}
```

- [ ] **Step 2: Remove daemon dispatcher arms and command-only handlers**

Delete the five dispatcher arms from `handler/rpc.rs`. Remove `rpc_reflection.rs`
only after moving any formatter or helper still used by internal lifecycle code
to its owning application module. Do not delete `ReflectionUseCases`, the
reflector, evolution coordinator, event observers, or evidence stores.

- [ ] **Step 3: Prove automatic reflection/evolution remains operational**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive --test evolution_integration -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test self_evolution_loop_test -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test retired_governance_rpc retired_governance_methods_are_unknown -- --nocapture
```

Expected: automatic governance tests pass; retired RPC methods return `-32601`.

- [ ] **Step 4: Commit daemon route removal**

```bash
git add crates/executive/src/host/daemon/handler/rpc.rs crates/executive/src/host/daemon/handler/rpc/rpc_reflection.rs crates/executive/src/host/daemon/handler/rpc/mod.rs crates/executive/tests/retired_governance_rpc.rs
git commit -F - <<'MSG'
refactor(executive): close manual governance RPCs

The host already owns reflection and evolution through typed lifecycle
components, so public JSON-RPC triggers are unnecessary bypasses.

- return unknown-method for retired governance RPC names
- preserve automatic reflection and evolution execution
- keep durable evidence available to typed diagnostics
MSG
```

### Task 6: Validate the installed command surface

**Files:**
- Modify: `tests/suites/operations/completion_test.sh`
- Create: `tests/suites/operations/tui_command_surface_test.sh`

- [ ] **Step 1: Add installed help/completion assertions**

```bash
expected='agent agents clear compact context copy diff fork help input interrupt memory mention mode model new permissions profile quit resume sessions skills status'
actual=$(
  source scripts/completions/aletheon.bash
  # TUI slash commands are validated by the Rust exact-set test; shell CLI must
  # not expose reflect/evolution top-level commands either.
  COMP_WORDS=(aletheon '') COMP_CWORD=1 _aletheon_cli_completion
  printf '%s\n' "${COMPREPLY[@]}" | sort | tr '\n' ' '
)
! grep -Eq '(^| )(reflect|reflect_now|evolution|genome)( |$)' <<<"$actual"
```

The TUI surface script must run the exact-set Rust test and grep recorded `/help`
frames to ensure no retired command is rendered.

- [ ] **Step 2: Run focused validation**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p interact --lib -- --nocapture
bash scripts/cargo-agent.sh test -p fabric --test client_rpc_public_surface -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test evolution_integration -- --nocapture
bash tests/suites/operations/completion_test.sh
bash tests/suites/operations/tui_command_surface_test.sh
```

Expected: all commands exit zero.

- [ ] **Step 3: Commit acceptance coverage**

```bash
git add tests/suites/operations/completion_test.sh tests/suites/operations/tui_command_surface_test.sh
git commit -F - <<'MSG'
test(tui): verify governed command surface

Unit contracts must agree with installed help and completion so removed internal
commands cannot reappear through a secondary entry point.

- check the exact TUI built-in set
- reject retired commands in help and completion
- retain proof that automatic governance still executes
MSG
```
