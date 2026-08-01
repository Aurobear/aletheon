# Engineering Benchmark and Authoritative Acceptance Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the existing three-task coding harness into a strict ten-scenario black-box engineering benchmark with authoritative failure classification, replayable receipts, and deterministic suite reports.

**Architecture:** Keep production task authority in existing Fabric/Executive contracts. Add one additive `stop` field to the JSON `aletheon exec` diagnostic output, then implement strict Python benchmark contracts, per-task execution/receipt logic, and sequential aggregation under `tests/coding/`.

**Tech Stack:** Rust/Serde, Python 3.11 standard library, TOML, Git fixtures, existing `scripts/cargo-agent.sh` build lock.

**Design:** `docs/plans/2026-08-02-engineering-benchmark-design.md`

---

## Requirement anchors

- Real repeatable repository tasks and deterministic final-state evidence:
  `spec:78-103`.
- Host acceptance, receipt-first completion, and no prose-based success:
  `spec:560-571`.
- Separate provider retries, inference/tool counts, and active context metrics:
  repository `AGENTS.md` debugging evidence policy.
- Existing harness entry point and receipt logic:
  `tests/coding/harness/run.py:39-96`.
- Existing authoritative turn stop:
  `crates/fabric/src/types/turn.rs:28-71`.

### Task 1: Expose the authoritative Turn stop in JSON exec output

**Files:**
- Modify: `crates/executive/src/host/launcher.rs:170-189`
- Test: `crates/executive/src/host/launcher.rs`

- [x] **Step 1: Add a failing serialization test**

Extract JSON rendering to a private helper and add this focused test:

```rust
#[test]
fn exec_json_preserves_authoritative_stop_and_separate_metrics() {
    let result = fabric::TurnResult {
        output: "waiting for approval".into(),
        stop: fabric::TurnStop::Blocked,
        metrics: fabric::TurnMetrics {
            iterations: 2,
            tool_calls_made: 1,
            tool_errors: 0,
            provider_retries: 3,
            elapsed_ms: 40,
            completed_normally: false,
        },
    };
    let value = render_exec_json(fabric::OperationId::new(), &result);
    assert_eq!(value["stop"], "blocked");
    assert_eq!(value["provider_retries"], 3);
    assert_eq!(value["tool_calls_made"], 1);
    assert!(value.get("inference_rounds").is_none());
}
```

- [x] **Step 2: Run the focused test and confirm it fails**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive host::launcher::tests::exec_json_preserves_authoritative_stop_and_separate_metrics -- --exact
```

Expected: FAIL because `render_exec_json` and the `stop` field do not exist.

- [x] **Step 3: Implement the additive JSON field**

Use the actual `TurnResult.stop` without deriving it from `success`:

```rust
fn render_exec_json(operation_id: OperationId, result: &fabric::TurnResult) -> serde_json::Value {
    serde_json::json!({
        "success": result.metrics.completed_normally,
        "operation_id": operation_id.0,
        "response": result.output,
        "stop": match result.stop {
            fabric::TurnStop::Completed => "completed",
            fabric::TurnStop::Blocked => "blocked",
            fabric::TurnStop::Cancelled => "cancelled",
            fabric::TurnStop::Failed => "failed",
        },
        "iterations": result.metrics.iterations,
        "tool_calls_made": result.metrics.tool_calls_made,
        "tool_errors": result.metrics.tool_errors,
        "provider_retries": result.metrics.provider_retries,
        "elapsed_ms": result.metrics.elapsed_ms,
    })
}
```

- [x] **Step 4: Run the focused test**

Expected: PASS.

- [x] **Step 5: Commit the client diagnostic stage**

Commit subject: `feat(exec): expose authoritative terminal stop`

### Task 2: Add strict benchmark task contracts

**Files:**
- Create: `tests/coding/harness/contracts.py`
- Create: `tests/coding/contracts_test.py`
- Modify: `tests/coding/tasks/rust_bugfix.toml`
- Modify: `tests/coding/tasks/rust_diagnosis.toml`
- Modify: `tests/coding/tasks/rust_multifile.toml`

- [x] **Step 1: Write failing contract tests**

Cover exact-key validation, unknown fields, traversal/absolute paths, duplicate IDs,
terminal/category enums, required/forbidden overlap, empty argv, and positive timeouts.
The valid fixture is:

```python
VALID = {
    "schema_version": 1,
    "id": "rust_bugfix",
    "category": "behavioral_bugfix",
    "fixture": "rust_bugfix",
    "prompt": "repair the defect",
    "timeout_secs": 300,
    "acceptance_commands": [["cargo", "test", "--quiet"]],
    "forbidden_paths": ["Cargo.toml"],
    "required_changed_paths": ["src/"],
    "expected_terminal": "verified",
    "setup": {},
    "resource_checks": ["no_descendant_processes"],
}
```

- [x] **Step 2: Run and confirm import failure**

```bash
python3 tests/coding/contracts_test.py
```

Expected: FAIL because `harness/contracts.py` does not exist.

- [x] **Step 3: Implement immutable `BenchmarkTask` and `load_catalog`**

Use `dataclasses.dataclass(frozen=True)`, `tomllib`, and `PurePosixPath`. Accept
only these top-level keys:

```python
TASK_KEYS = frozenset({
    "schema_version", "id", "category", "fixture", "prompt", "timeout_secs",
    "acceptance_commands", "forbidden_paths", "required_changed_paths",
    "expected_terminal", "setup", "resource_checks",
})
TERMINALS = frozenset({"verified", "blocked", "budget_exhausted", "cancelled"})
CATEGORIES = frozenset({
    "behavioral_bugfix", "diagnosis", "multifile_change", "regression_test",
    "config_schema", "lint", "documentation", "dirty_workspace",
    "budget", "approval",
})
```

`load_catalog(paths, root)` validates every task and rejects duplicate IDs before
returning tasks sorted by ID.

- [x] **Step 4: Migrate the existing task TOML files**

Add the missing schema/category/required/terminal/setup/resource fields without
changing prompts or acceptance semantics.

- [x] **Step 5: Run contract tests**

Expected: PASS.

- [x] **Step 6: Commit**

Commit subject: `test(coding): define strict benchmark task contracts`

### Task 3: Version and validate benchmark receipts

**Files:**
- Create: `tests/coding/harness/receipt.py`
- Modify: `tests/coding/harness/replay.py`
- Modify: `tests/coding/replay_test.py`

- [x] **Step 1: Add failing receipt tests**

Test canonical sealing, v1 compatibility, v2 required fields, mutually exclusive
failure classification, operation/evidence correlation, false-success rejection,
metric null preservation, and tampering.

Use this v2 terminal envelope:

```python
{
  "schema_version": 2,
  "task_schema_version": 1,
  "task_id": "rust_bugfix",
  "category": "behavioral_bugfix",
  "binary": {"path": "/tmp/aletheon", "sha256": "sha256:abc"},
  "operation_id": "op",
  "observed_stop": "completed",
  "expected_terminal": "verified",
  "failure": {"class": "none", "reasons": []},
  "verification": {"passed": True},
  "metrics": {
    "iterations": 1, "tool_calls": 2, "inference_rounds": None,
    "provider_retries": 0, "active_context_tokens": None, "elapsed_ms": 10
  }
}
```

- [x] **Step 2: Run and confirm failure**

```bash
python3 tests/coding/replay_test.py
```

Expected: FAIL until v2 replay exists.

- [x] **Step 3: Implement receipt helpers**

`seal`, `verify_integrity`, `classify_failure`, and `verify_receipt` own all
verdict precedence. Use this precedence:

```text
catalog/binary/core/transport → infrastructure_failure
timeout/cancel                → timeout_or_cancellation
forbidden/scope               → policy_scope_failure
runtime/tool                  → execution_failure
acceptance/evidence           → verification_failure
all gates                     → none
```

Retain a v1 verifier so the three checked-in historical receipts remain replayable.

- [x] **Step 4: Run replay tests**

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `test(coding): classify and replay benchmark receipts`

### Task 4: Refactor the per-task runner around the contracts

**Files:**
- Modify: `tests/coding/harness/run.py`
- Create: `tests/coding/runner_test.py`

- [x] **Step 1: Write fake-client runner tests**

Generate executable Python clients for completed, blocked, failed, malformed JSON,
missing operation, timeout, and false-success responses. Assert the runner writes
a v2 receipt for every case, preserves full-output digests, bounds inline output,
wraps Cargo, preserves dirty setup, rejects scope changes, and reaps the process
group.

- [x] **Step 2: Run and confirm failures**

```bash
python3 tests/coding/runner_test.py
```

- [x] **Step 3: Implement the phased runner**

Import `BenchmarkTask` and receipt helpers. Add:

```python
def observed_terminal(task, executive, execution):
    if execution["timed_out"]:
        return "cancelled"
    stop = str(executive.get("stop", "failed"))
    limit = task.setup.get("exec_max_turns")
    if (
        task.expected_terminal == "budget_exhausted"
        and stop == "blocked"
        and isinstance(limit, int)
        and executive.get("iterations") == limit
    ):
        return "budget_exhausted"
    return "verified" if stop == "completed" and executive.get("success") else stop

def apply_setup(task, workspace):
    dirty_path = task.setup.get("dirty_path")
    if dirty_path is None:
        return None
    target = workspace / dirty_path
    target.write_text(task.setup["dirty_content"], encoding="utf-8")
    return digest(git(workspace, "diff", "--binary", "HEAD", "--", dirty_path))

def changed_paths(workspace):
    lines = git(workspace, "status", "--porcelain=v1", "-z").split(b"\0")
    return sorted({line[3:].decode() for line in lines if len(line) >= 4})

def path_matches(path, declared):
    return path == declared or (declared.endswith("/") and path.startswith(declared))

def verify_dirty_patch(task, workspace, before_digest):
    dirty_path = task.setup.get("dirty_path")
    if dirty_path is None:
        return True
    current = digest(git(workspace, "diff", "--binary", "HEAD", "--", dirty_path))
    return current == before_digest

def check_resources(task, execution):
    if "no_descendant_processes" not in task.resource_checks:
        return {"passed": True, "checks": []}
    passed = bool(execution["process_group_reaped"])
    return {"passed": passed, "checks": ["no_descendant_processes"]}

def build_receipt(task, binary, execution, executive, workspace_evidence):
    observed = observed_terminal(task, executive, execution)
    receipt = {
        "schema_version": 2,
        "task_schema_version": task.schema_version,
        "task_id": task.id,
        "category": task.category,
        "binary": {"path": str(binary), "sha256": digest(binary.read_bytes())},
        "operation_id": str(executive.get("operation_id", "")),
        "observed_stop": str(executive.get("stop", "failed")),
        "observed_terminal": observed,
        "expected_terminal": task.expected_terminal,
        "execution": execution,
        "workspace": workspace_evidence,
        "metrics": {
            "iterations": executive.get("iterations"),
            "tool_calls": executive.get("tool_calls_made"),
            "inference_rounds": executive.get("inference_rounds"),
            "provider_retries": executive.get("provider_retries"),
            "active_context_tokens": executive.get("active_context_tokens"),
            "elapsed_ms": executive.get("elapsed_ms", execution["elapsed_ms"]),
        },
    }
    receipt["failure"] = classify_failure(receipt)
    receipt["verification"] = {
        "passed": observed == task.expected_terminal and receipt["failure"]["class"] == "none"
    }
    return seal(receipt)
```

Pass `--max-turns` from `setup.exec_max_turns` when present and `--config` from a
setup-provided isolated config. Never place secret values in the receipt.

- [x] **Step 4: Run runner and replay tests**

```bash
python3 tests/coding/runner_test.py
python3 tests/coding/replay_test.py
```

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `test(coding): make task execution fail closed`

### Task 5: Add deterministic suite aggregation

**Files:**
- Create: `tests/coding/harness/suite.py`
- Create: `tests/coding/suite_test.py`

- [x] **Step 1: Write failing aggregation tests**

Create sealed receipt fixtures proving deterministic task ordering, per-category
and failure counts, false-success counting, null metric availability, percentile
calculation, integrity propagation, and nonzero exits for infrastructure versus
verification failures.

- [x] **Step 2: Run and confirm import failure**

```bash
python3 tests/coding/suite_test.py
```

- [x] **Step 3: Implement sequential suite execution and report sealing**

The CLI is:

```bash
python3 tests/coding/harness/suite.py \
  --catalog tests/coding/tasks \
  --receipts tests/coding/receipts/current \
  --report tests/coding/receipts/current/suite.json
```

Exit `2` for any infrastructure failure, `1` for other unmet expected outcomes,
and `0` only when every scenario matches its declared expected terminal and all
receipt integrity checks pass.

- [x] **Step 4: Run suite tests**

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `test(coding): aggregate authoritative suite results`

### Task 6: Add the four positive engineering fixtures

**Files:**
- Create: `tests/coding/fixtures/rust_regression_test/**`
- Create: `tests/coding/fixtures/config_schema_sync/**`
- Create: `tests/coding/fixtures/clippy_cleanup/**`
- Create: `tests/coding/fixtures/rustdoc_contract/**`
- Create: `tests/coding/acceptance/rust_regression_test/**`
- Create: `tests/coding/acceptance/config_schema_sync/**`
- Create: `tests/coding/acceptance/clippy_cleanup/**`
- Create: `tests/coding/acceptance/rustdoc_contract/**`
- Create: four matching `tests/coding/tasks/*.toml`

- [x] **Step 1: Create failing minimal repositories**

Each repository is a standalone Rust workspace fixture with no external crates:

- regression: an escaped-delimiter parser defect; hidden acceptance requires a
  new visible regression test and correct behavior;
- config/schema: a new typed `request_timeout_ms` field whose checked-in JSON
  schema must match the Rust default and reject zero;
- clippy: behavior-preserving cleanup with `#![deny(clippy::all)]` and a hidden
  edge-case test;
- rustdoc: a documented public error contract enforced by `#![deny(rustdoc::broken_intra_doc_links)]`
  plus hidden behavior acceptance.

- [x] **Step 2: Prove every initial fixture fails its acceptance**

For each fixture run its declared command through:

```bash
bash scripts/cargo-agent.sh test --manifest-path tests/coding/fixtures/<id>/Cargo.toml --quiet
```

Expected: at least one declared pre-repair check fails for each fixture.

- [x] **Step 3: Add strict task documents and hidden acceptance overlays**

Every task uses schema version 1, expected terminal `verified`, required `src/`
or schema paths, and protects its manifest/irrelevant files.

- [x] **Step 4: Run catalog/static tests**

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `test(coding): add positive engineering scenarios`

### Task 7: Add dirty-workspace and negative-terminal scenarios

**Files:**
- Create: `tests/coding/fixtures/dirty_workspace_preservation/**`
- Create: `tests/coding/fixtures/budget_exhaustion/**`
- Create: `tests/coding/fixtures/approval_blocked_patch/**`
- Create: three matching task files and hidden acceptance overlays
- Modify: `tests/coding/runner_test.py`

- [x] **Step 1: Add dirty setup and terminal tests**

The dirty task setup writes a user-owned change after the fixture base commit and
requires its exact binary patch digest to survive. The budget task sets
`setup.exec_max_turns = 1` and expects `budget_exhausted`. The approval task uses
safe policy with an intentionally guarded mutation and expects `blocked` without
the target diff appearing.

- [x] **Step 2: Confirm old runner cannot represent the scenarios**

Run runner tests and expect failures for setup and expected-terminal mapping.

- [x] **Step 3: Complete scenario support without prompt-specific verdicts**

Verdicts derive only from task typed state, authoritative `stop`, process exit,
Git evidence, and acceptance results. No code may inspect prompt text, fixture ID,
language, or expected answer.

- [x] **Step 4: Run contract, runner, and static tests**

Expected: PASS with exactly ten catalog entries.

- [x] **Step 5: Commit**

Commit subject: `test(coding): cover dirty and bounded terminal behavior`

### Task 8: Wire static CI and the manual real-model suite

**Files:**
- Modify: `tests/coding/static_test.sh`
- Modify: `tests/coding/workflow_static_test.py`
- Modify: `.github/workflows/coding-e2e.yml`
- Modify: `tests/coding/README.md`

- [x] **Step 1: Update failing workflow/static expectations**

Require all ten tasks in the catalog, suite aggregation instead of a shell loop,
always-uploaded per-task receipts/report/core log, and explicit infrastructure
versus benchmark failure exit codes.

- [x] **Step 2: Run static tests and confirm failure before workflow edits**

```bash
python3 tests/coding/workflow_static_test.py
bash tests/coding/static_test.sh
```

- [x] **Step 3: Replace the manual workflow task loop with `suite.py`**

Keep provider credentials in the existing owner-only core config. Upload the
sealed suite report and receipts, never the generated config or temporary HOME.

- [x] **Step 4: Document diagnostic versus installed acceptance**

Document deterministic checks, suite invocation, failure exit codes, receipt
locations, and the rule that target/debug plus isolated homes are diagnostic only.

- [x] **Step 5: Run static tests**

Expected: PASS.

- [x] **Step 6: Commit**

Commit subject: `ci(coding): run the versioned engineering suite`

### Task 9: Focused validation and installed-runtime gate

**Files:**
- Verify all files above

- [x] **Step 1: Run deterministic Python validation**

```bash
python3 tests/coding/contracts_test.py
python3 tests/coding/replay_test.py
python3 tests/coding/runner_test.py
python3 tests/coding/suite_test.py
python3 tests/coding/workflow_static_test.py
bash tests/coding/static_test.sh
```

Expected: PASS.

- [x] **Step 2: Run the focused Rust test and formatting**

```bash
bash scripts/cargo-agent.sh test -p executive host::launcher::tests::exec_json_preserves_authoritative_stop_and_separate_metrics -- --exact
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: PASS.

- [x] **Step 3: Run repository diff validation**

```bash
git diff --check
git status --short
```

Expected: no whitespace errors; only intended files changed.

- [x] **Step 4: Run the real-model suite using the actual client**

Build through the shared lock, start the configured core, then run `suite.py`.
Record the real outcome without converting provider/infrastructure/verification
failures into success.

- [x] **Step 5: Perform system-installed acceptance**

Because Task 1 changes client JSON behavior, run:

```bash
sudo bash scripts/aletheon.sh deploy
```

The gate must prove equal SHA-256 digests for `target/release/aletheon`,
`/usr/bin/aletheon`, and running machine/user daemons, stable restart counters,
and a real request through `/usr/bin/aletheon` plus the official user socket.

- [x] **Step 6: Commit validation evidence changes if any**

Commit subject: `test(coding): validate engineering benchmark acceptance`

## Validation evidence — 2026-08-02

- Deterministic contract, replay, runner, suite, workflow, and static checks:
  PASS. The three historical v1 receipts also replay as `verified` through the
  explicit compatibility path.
- Focused Executive serialization test and `fmt --all -- --check`: PASS through
  `scripts/cargo-agent.sh`.
- Real-client diagnostic suite: completed all 10 scenarios and exited 1 with
  the actual result (1 passed, 9 failed, 0 infrastructure failures, 0 leaked
  resources, and 2 false-success detections). Sealed report SHA-256:
  `905153458f13381018adfe423bf0cfba3ec3f4edb5827c01bea99f92b7c6ca07`.
- `sudo bash scripts/aletheon.sh deploy`: PASS. Release, installed, machine
  daemon, and user daemon executables all had SHA-256
  `d0ada4d1a3850c63b94db445c6eaacd994f460dd21f8d9f21fab36f1a5471a12`;
  both services remained active with unchanged PID and `NRestarts=0`, and the
  deployment gate completed a real request through the installed client and
  official user socket.
