# Installed Runtime Verification Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make deployment fail unless the release candidate, installed binary, running systemd daemons, official socket, and real LLM request are proven to be the same usable production runtime.

**Architecture:** Add a focused shell module for installed-runtime provenance, restart stability, fatal startup-log detection, and official-client smoke verification. Wire that module into the existing `verify` command, document the hard acceptance boundary, and validate it with hermetic command/process fixtures.

**Tech Stack:** Bash, systemd CLI contracts, `/proc` executable provenance, SHA-256, existing Aletheon CLI and JSON-RPC socket, shell integration tests.

---

### Task 1: Add failing runtime-gate integration tests

**Files:**
- Create: `tests/installed_runtime_gate_test.sh`
- Modify: `tests/operations_cli_static_test.sh`

- [ ] **Step 1: Create hermetic fixtures**

Create temporary candidate and installed executables, a fake `/proc` tree, and
fake `systemctl`, `journalctl`, and `timeout` commands. The fake systemctl must
return deterministic `MainPID` and `NRestarts` values for machine and user
units. The fake timeout command must execute the installed client fixture.

- [ ] **Step 2: Add the passing case**

Source `scripts/lib/aletheon/common.sh` and the new runtime gate module, set:

```bash
ALETHEON_RELEASE_BINARY="$tmp/candidate/aletheon"
ALETHEON_INSTALLED_BINARY="$tmp/installed/aletheon"
ALETHEON_PROC_ROOT="$tmp/proc"
ALETHEON_STABILITY_SECONDS=0
ALETHEON_SMOKE_TIMEOUT_SECONDS=2
ALETHEON_USER_SOCKET="$tmp/aletheon.sock"
```

Use byte-identical candidate, installed, and runtime executable fixtures. Assert
that `cmd_installed_runtime_gate` succeeds and prints:

```text
installed runtime provenance verified
runtime stability verified
official client real-request smoke test passed
```

- [ ] **Step 3: Add fail-closed cases**

Run isolated cases and assert non-zero status plus a precise diagnostic for:

```text
installed binary hash differs from release candidate
running executable hash differs from release candidate
service restart count increased during stability window
fatal startup validation error detected
official client real-request smoke test failed
official client real-request returned empty output
```

- [ ] **Step 4: Add static wiring expectations**

Require `scripts/aletheon.sh` to source `runtime_gate.sh` and require
`cmd_verify` to invoke `cmd_installed_runtime_gate`.

- [ ] **Step 5: Run tests and observe the expected failure**

Run:

```bash
bash tests/installed_runtime_gate_test.sh
bash tests/operations_cli_static_test.sh
```

Expected: failure because `scripts/lib/aletheon/runtime_gate.sh` and
`cmd_installed_runtime_gate` do not exist.

### Task 2: Implement binary provenance verification

**Files:**
- Create: `scripts/lib/aletheon/runtime_gate.sh`
- Modify: `scripts/lib/aletheon/common.sh`

- [ ] **Step 1: Add configurable production paths**

Add defaults:

```bash
ALETHEON_INSTALLED_BINARY=${ALETHEON_INSTALLED_BINARY:-/usr/bin/aletheon}
ALETHEON_PROC_ROOT=${ALETHEON_PROC_ROOT:-/proc}
ALETHEON_CORE_UNIT=${ALETHEON_CORE_UNIT:-aletheon-core.service}
ALETHEON_USER_UNIT=${ALETHEON_USER_UNIT:-aletheon.service}
ALETHEON_STABILITY_SECONDS=${ALETHEON_STABILITY_SECONDS:-7}
ALETHEON_SMOKE_TIMEOUT_SECONDS=${ALETHEON_SMOKE_TIMEOUT_SECONDS:-60}
ALETHEON_SMOKE_PROMPT=${ALETHEON_SMOKE_PROMPT:-Reply with exactly: ALETHEON_DEPLOYMENT_OK}
```

- [ ] **Step 2: Resolve running executables**

Implement helpers that call:

```bash
systemctl show "$ALETHEON_CORE_UNIT" --property MainPID --value
systemctl --user show "$ALETHEON_USER_UNIT" --property MainPID --value
readlink -f "$ALETHEON_PROC_ROOT/$pid/exe"
```

Reject empty/non-numeric/zero PIDs and non-executable paths.

- [ ] **Step 3: Compare provenance**

Compute SHA-256 for candidate, installed, core runtime, and user runtime.
Require every value to match the candidate and print paths/hashes only. Never
inspect or print process environments.

- [ ] **Step 4: Run the focused test**

Run:

```bash
bash tests/installed_runtime_gate_test.sh provenance
```

Expected: all provenance passing and mismatch cases pass.

### Task 3: Implement restart stability and fatal-log gates

**Files:**
- Modify: `scripts/lib/aletheon/runtime_gate.sh`
- Test: `tests/installed_runtime_gate_test.sh`

- [ ] **Step 1: Snapshot service state**

Read `ActiveState`, `MainPID`, and `NRestarts` through machine/user systemctl
commands. Require both services to be active before and after the stability
interval.

- [ ] **Step 2: Observe the bounded window**

Sleep for `ALETHEON_STABILITY_SECONDS`, then require stable PIDs and unchanged
restart counters. A zero-second interval is allowed only through the explicit
environment override used by tests.

- [ ] **Step 3: Detect fatal startup evidence for current PIDs**

Query journal entries scoped to the current service PID and fail on:

```text
references unknown tool
Agent profile
panic
Main process exited
Failed with result
```

Do not scan unbounded historical logs, because repaired historical failures
must not poison a healthy deployment.

- [ ] **Step 4: Run stability cases**

Run:

```bash
bash tests/installed_runtime_gate_test.sh stability
```

Expected: stable case passes; restart-count and fatal-log cases fail closed.

### Task 4: Implement official-client real-request smoke gate

**Files:**
- Modify: `scripts/lib/aletheon/runtime_gate.sh`
- Test: `tests/installed_runtime_gate_test.sh`

- [ ] **Step 1: Execute only the installed client**

Run:

```bash
timeout "$ALETHEON_SMOKE_TIMEOUT_SECONDS" \
  "$ALETHEON_INSTALLED_BINARY" \
  --socket "$ALETHEON_USER_SOCKET" \
  -m "$ALETHEON_SMOKE_PROMPT"
```

Capture output in a mode-0600 temporary file and clean it on return.

- [ ] **Step 2: Validate completion**

Require exit status zero and at least one non-whitespace output character.
Do not log the full model response because it may contain user/provider data.

- [ ] **Step 3: Recheck runtime state**

After the request, repeat provenance and stability snapshots so a daemon crash
during inference fails deployment.

- [ ] **Step 4: Run smoke cases**

Run:

```bash
bash tests/installed_runtime_gate_test.sh smoke
```

Expected: successful response passes; timeout/failure and empty response fail.

### Task 5: Wire the strict gate into deploy and verify

**Files:**
- Modify: `scripts/aletheon.sh`
- Modify: `scripts/lib/aletheon/verify.sh`
- Test: `tests/operations_cli_static_test.sh`
- Test: `tests/operations_cli_test.sh`

- [ ] **Step 1: Source the runtime module**

Add:

```bash
source "$SCRIPT_DIR/lib/aletheon/runtime_gate.sh"
```

- [ ] **Step 2: Extend deployed verification**

Call `cmd_installed_runtime_gate` after existing configuration, service, health,
closure-asset, and Pi-runtime checks. Preserve fail-fast behavior.

- [ ] **Step 3: Verify deploy ordering**

Keep the existing order:

```text
build -> install -> closure -> restart -> verify
```

Because `verify` now owns the strict gate, both `deploy` and standalone
`verify` use identical post-install acceptance.

- [ ] **Step 4: Run operations tests**

Run:

```bash
bash tests/operations_cli_static_test.sh
bash tests/operations_cli_test.sh
bash tests/installed_runtime_gate_test.sh
```

Expected: all tests pass.

### Task 6: Add repository and operator constraints

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/deployment/README.md`
- Modify: `docs/deployment/operations-checklist.md`

- [ ] **Step 1: Add the hard repository rule**

Document that isolated/debug/direct-provider evidence is diagnostic only, that
test-only profiles must remain isolated, and that affected runtime changes
require `bash scripts/aletheon.sh deploy` before completion can be claimed.

- [ ] **Step 2: Document canonical commands**

Document:

```bash
bash scripts/aletheon.sh build
bash scripts/aletheon.sh deploy
bash scripts/aletheon.sh verify
```

Explain candidate/installed/runtime hash evidence, stability interval, and the
real LLM smoke request.

- [ ] **Step 3: Update the operator checklist**

Add explicit checkboxes for installed provenance, no restart-count increase,
current-PID fatal-log scan, and real request completion.

- [ ] **Step 4: Run documentation/static checks**

Run:

```bash
bash tests/operations_cli_static_test.sh
git diff --check
```

Expected: pass with no whitespace errors.

### Task 7: Review and installed-host validation

**Files:**
- Review: all files changed by Tasks 1-6

- [ ] **Step 1: Run deterministic validation**

Run:

```bash
bash tests/operations_cli_static_test.sh
bash tests/operations_cli_test.sh
bash tests/installed_runtime_gate_test.sh
bash -n scripts/aletheon.sh scripts/lib/aletheon/*.sh
git diff --check
```

Expected: all commands pass.

- [ ] **Step 2: Inspect the staged diff**

Confirm that no credentials, provider responses, temporary paths, or unrelated
working-tree changes are staged.

- [ ] **Step 3: Commit implementation stages**

Create coherent commits for:

1. runtime gate and tests;
2. repository/deployment documentation.

Each commit must include problem context, solution context, and per-file bullets.

- [ ] **Step 4: Run the real deployment gate**

On the authorized host, run:

```bash
bash scripts/aletheon.sh deploy
```

Expected: release build, native install, official service restart, provenance,
stability, real LLM request, and final health all pass. If privilege or provider
access is unavailable, report the gate as blocked rather than complete.
