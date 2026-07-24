# Operations Entrypoint Consolidation Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate repository operations behind `setup.sh`, `scripts/aletheon.sh`, and `scripts/cargo-agent.sh` while moving internal scripts and shell tests into explicit implementation suites.

**Architecture:** Preserve low-level scripts as focused executables beneath `scripts/libexec/aletheon`, and add small sourced dispatcher modules for maintenance, security, acceptance, verification, and tests. Move top-level shell tests into categorized suites, update every live caller atomically, and add an inventory gate that prevents removed public paths from returning.

**Tech Stack:** Bash, git path moves, systemd asset installation, GitHub Actions, existing shell/Rust integration tests.

---

### Task 1: Add the failing public-surface inventory test

**Files:**
- Create: `tests/suites/operations/script_surface_test.sh`
- Modify: `tests/operations_cli_static_test.sh` before its move

- [ ] **Step 1: Define the approved root scripts**

The test enumerates repository public shell entrypoints and requires exactly:

```text
setup.sh
scripts/aletheon.sh
scripts/cargo-agent.sh
```

It permits `.sh` implementations only below `scripts/lib/` and
`scripts/libexec/`.

- [ ] **Step 2: Define removed-path rejection**

Add an exact array containing every former `scripts/*.sh` path except
`aletheon.sh` and `cargo-agent.sh`. Use `git grep` over live code, CI, current
docs, tests, `justfile`, and `architecture-status.toml`; exclude historical
documents under `docs/plans/` and `docs/arch/` whose paths are preserved as
historical evidence.

- [ ] **Step 3: Assert unified help groups**

Require `scripts/aletheon.sh help` to contain:

```text
backup
restore
upgrade
cleanup
secrets
database
verify
acceptance
test
```

- [ ] **Step 4: Run and confirm failure**

Run:

```bash
bash tests/suites/operations/script_surface_test.sh
```

Expected: failure listing the current top-level internal scripts.

### Task 2: Move standalone internal scripts

**Files:**
- Move: `scripts/aletheon-healthcheck.sh` → `scripts/libexec/aletheon/healthcheck.sh`
- Move: `scripts/aletheon-pi-scheduled-task.sh` → `scripts/libexec/aletheon/pi-scheduled-task.sh`
- Move: `scripts/aletheon-secret-audit.sh` → `scripts/libexec/aletheon/secret-audit.sh`
- Move: `scripts/aletheon-secret-init.sh` → `scripts/libexec/aletheon/secret-init.sh`
- Move: `scripts/aletheon-sqlite-check.sh` → `scripts/libexec/aletheon/sqlite-check.sh`
- Move: `scripts/architecture-check.sh` → `scripts/libexec/aletheon/architecture-check.sh`
- Move: `scripts/backup-aletheon.sh` → `scripts/libexec/aletheon/backup.sh`
- Move: `scripts/cleanup-aletheon.sh` → `scripts/libexec/aletheon/cleanup.sh`
- Move: `scripts/cleanup-cargo-target.sh` → `scripts/libexec/aletheon/cleanup-cargo-target.sh`
- Move: `scripts/install-systemd.sh` → `scripts/libexec/aletheon/install-systemd.sh`
- Move: `scripts/release-acceptance.sh` → `scripts/libexec/aletheon/release-acceptance.sh`
- Move: `scripts/restore-aletheon.sh` → `scripts/libexec/aletheon/restore.sh`
- Move: `scripts/upgrade-aletheon.sh` → `scripts/libexec/aletheon/upgrade.sh`
- Move: `scripts/verify-compose.sh` → `scripts/libexec/aletheon/verify/compose.sh`
- Move: `scripts/verify-migration-matrix.sh` → `scripts/libexec/aletheon/verify/migration-matrix.sh`
- Move: `scripts/verify-multi-user-runtime.sh` → `scripts/libexec/aletheon/verify/multi-user-runtime.sh`
- Move: `scripts/verify-network-exposure.sh` → `scripts/libexec/aletheon/verify/network-exposure.sh`
- Move: `scripts/verify-systemd.sh` → `scripts/libexec/aletheon/verify/systemd.sh`

- [ ] **Step 1: Create destination directories and move with Git**

Run exact `mkdir -p` and `git mv` commands for the paths above so history is
preserved.

- [ ] **Step 2: Normalize repository-root discovery**

Scripts formerly using one parent from `scripts/` must resolve three parents
from `scripts/libexec/aletheon/`; verifier scripts resolve four parents from the
`verify/` subdirectory. Keep production installed-path behavior independent of
the source checkout.

- [ ] **Step 3: Update internal script references**

In `install-systemd.sh` and `release-acceptance.sh`, replace old repository
sources with exact `scripts/libexec/aletheon/...` paths while keeping installed
destinations such as `/usr/libexec/aletheon/verify-systemd.sh` unchanged.

- [ ] **Step 4: Run syntax validation**

Run:

```bash
bash -n scripts/libexec/aletheon/*.sh scripts/libexec/aletheon/verify/*.sh
```

Expected: pass.

### Task 3: Add grouped dispatcher modules

**Files:**
- Create: `scripts/lib/aletheon/maintenance.sh`
- Create: `scripts/lib/aletheon/security.sh`
- Create: `scripts/lib/aletheon/acceptance.sh`
- Create: `scripts/lib/aletheon/test.sh`
- Modify: `scripts/lib/aletheon/install.sh`
- Modify: `scripts/lib/aletheon/verify.sh`
- Modify: `scripts/aletheon.sh`

- [ ] **Step 1: Add a shared internal executor**

In `common.sh`, define:

```bash
ALETHEON_LIBEXEC=${ALETHEON_LIBEXEC:-$ALETHEON_ROOT/scripts/libexec/aletheon}

run_internal() {
  local relative=$1
  shift
  local command=$ALETHEON_LIBEXEC/$relative
  [[ -x "$command" ]] || {
    aletheon_die "internal command is unavailable: $relative"
    return
  }
  "$command" "$@"
}
```

- [ ] **Step 2: Implement maintenance commands**

Dispatch without rewriting low-level behavior:

```text
backup             -> backup.sh
restore            -> restore.sh
upgrade            -> upgrade.sh
cleanup runtime    -> cleanup.sh
cleanup cargo      -> cleanup-cargo-target.sh
```

All remaining arguments pass through unchanged.

- [ ] **Step 3: Implement security and database commands**

Dispatch:

```text
secrets init       -> secret-init.sh
secrets audit      -> secret-audit.sh
database check     -> sqlite-check.sh
```

Reject missing/unknown subcommands with exit status 2.

- [ ] **Step 4: Implement verification and acceptance groups**

Preserve bare `verify` as `cmd_verify`. Dispatch specialized forms:

```text
verify systemd     -> verify/systemd.sh
verify network     -> verify/network-exposure.sh
verify compose     -> verify/compose.sh
verify migration   -> verify/migration-matrix.sh
verify multi-user  -> verify/multi-user-runtime.sh
acceptance architecture -> architecture-check.sh
acceptance release      -> release-acceptance.sh
```

- [ ] **Step 5: Update existing installer/health paths**

Use `$ALETHEON_LIBEXEC` for healthcheck, scheduled-task source, and
install-systemd source. Installed filenames and systemd unit commands remain
unchanged.

- [ ] **Step 6: Update help and top-level parsing**

Source the four new modules and add exact case branches for every public group.
Pass all arguments after the top-level command to the owning module.

### Task 4: Categorize top-level shell tests

**Files:**
- Move: `tests/architecture_check.sh` → `tests/suites/architecture/architecture_check.sh`
- Move: `tests/architecture_path_inventory.sh` → `tests/suites/architecture/path_inventory.sh`
- Move: `tests/installed_runtime_gate_test.sh` → `tests/suites/operations/installed_runtime_gate_test.sh`
- Move: `tests/operations_cli_static_test.sh` → `tests/suites/operations/cli_static_test.sh`
- Move: `tests/operations_cli_test.sh` → `tests/suites/operations/cli_test.sh`
- Move: `tests/systemd_runtime_boundary.sh` → `tests/suites/deployment/systemd_runtime_boundary.sh`
- Move: `tests/upgrade_multi_user_test.sh` → `tests/suites/deployment/upgrade_multi_user_test.sh`

- [ ] **Step 1: Move tests with Git**

Create suite directories and use `git mv` for all seven paths.

- [ ] **Step 2: Fix repository-root discovery**

Each moved script resolves the root with:

```bash
root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
```

Preserve uppercase `ROOT` where existing fixtures rely on that variable name.

- [ ] **Step 3: Implement test dispatch**

Map:

```text
test operations
  -> script_surface_test.sh
  -> cli_static_test.sh
  -> cli_test.sh
  -> installed_runtime_gate_test.sh

test architecture
  -> architecture_check.sh
  -> path_inventory.sh
  -> internal architecture-check.sh

test deployment
  -> systemd_runtime_boundary.sh
  -> upgrade_multi_user_test.sh
  -> tests/production/installed_host_static_test.sh
  -> tests/production/failure_matrix_static_test.sh
  -> tests/production/release_aggregate_receipt_test.sh

test unit
  -> bash scripts/cargo-agent.sh test --workspace

test all
  -> operations, architecture, deployment, unit in fail-fast order
```

The test dispatcher executes every suite through `bash` and stops on the first
failure.

### Task 5: Migrate live callers and installed sources

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `justfile`
- Modify: `setup.sh` only where it references moved internals
- Modify: `scripts/libexec/aletheon/install-systemd.sh`
- Modify: `scripts/libexec/aletheon/release-acceptance.sh`
- Modify: `crates/executive/tests/production_health.rs`
- Modify: `tests/production/*.sh`
- Modify: `tests/production/lib/installed_host.sh`
- Modify: moved test files under `tests/suites/`
- Modify: `architecture-status.toml`

- [ ] **Step 1: Update CI and just recipes**

Use public commands:

```text
scripts/aletheon.sh test architecture
scripts/aletheon.sh cleanup cargo
scripts/aletheon.sh acceptance release
```

Keep narrow Cargo jobs on `scripts/cargo-agent.sh`.

- [ ] **Step 2: Update code-embedded script source**

Change Rust `include_str!` to:

```rust
include_str!("../../../scripts/libexec/aletheon/healthcheck.sh")
```

- [ ] **Step 3: Update low-level production tests**

Tests that validate low-level behavior may reference
`scripts/libexec/aletheon/...` directly. Operator-facing tests must invoke
`scripts/aletheon.sh`.

- [ ] **Step 4: Preserve production install names**

Installer source paths change, but installed helpers retain reviewed names:

```text
/usr/libexec/aletheon/verify-systemd.sh
/usr/libexec/aletheon/backup-aletheon.sh
/usr/libexec/aletheon/upgrade-aletheon.sh
```

### Task 6: Update current documentation

**Files:**
- Modify: `docs/deployment/README.md`
- Modify: `docs/deployment/systemd.md`
- Modify: `docs/deployment/secrets.md`
- Modify: `docs/deployment/upgrade-rollback.md`
- Modify: `docs/deployment/tailscale.md`
- Modify: `docs/testing/production-scenarios.md`
- Modify: `deploy/README.md`
- Modify: `deploy/gbrain/README.md`
- Modify: `docs/design/executive/daemon.md`
- Modify: `docs/arch/PUBLIC_API_CONTRACTION_INVENTORY.md`
- Modify: `tests/fixtures/architecture/README.md`

- [ ] **Step 1: Replace operator examples**

Use public grouped commands rather than repository internal paths.

- [ ] **Step 2: Explain internal versus installed paths**

Document that source implementations live in `scripts/libexec/aletheon`, while
installed service helpers retain `/usr/libexec/aletheon` names.

- [ ] **Step 3: Add an old-to-new mapping**

List every removed top-level script and its public `aletheon.sh` replacement.
Historical plans remain unchanged and are excluded from the live-reference
inventory.

### Task 7: Validate, review, and commit

**Files:**
- Review: all files changed by Tasks 1-6

- [ ] **Step 1: Run shell syntax and surface tests**

Run:

```bash
bash -n setup.sh scripts/aletheon.sh scripts/cargo-agent.sh \
  scripts/lib/aletheon/*.sh scripts/libexec/aletheon/*.sh \
  scripts/libexec/aletheon/verify/*.sh tests/suites/*/*.sh
bash scripts/aletheon.sh test operations
bash scripts/aletheon.sh test architecture
bash scripts/aletheon.sh test deployment
```

Expected: pass.

- [ ] **Step 2: Run focused Rust validation**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive --test production_health
```

Expected: pass.

- [ ] **Step 3: Run static reference and diff checks**

Run:

```bash
bash tests/suites/operations/script_surface_test.sh
git diff --check
git status --short
```

Expected: no forbidden live references and no unrelated files staged.

- [ ] **Step 4: Commit coherent stages**

Create separate commits for:

1. internal script moves and dispatcher commands;
2. test-suite moves and CI migration;
3. documentation and inventory constraints.

Every commit includes problem context, solution context, and per-file change
bullets.

- [ ] **Step 5: Re-run strict installed deployment acceptance**

After review and authorized installation:

```bash
bash scripts/aletheon.sh deploy
```

Expected: installed provenance, service stability, real LLM request, and final
health pass through the consolidated public entrypoint.
