# Unified Operations and Documentation Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide one repeatable Aletheon operations command and one canonical deployment guide while archiving superseded SER8 process notes.

**Architecture:** A small Bash dispatcher sources focused modules under `scripts/lib/aletheon/` and delegates reviewed low-level work to existing build, install, health, and systemd helpers. Active deployment documentation points to the dispatcher; dated process documents move to a clearly marked archive.

**Tech Stack:** Bash 5, Python 3 TOML parsing, systemd user/system units, existing Cargo wrapper, shell static/integration tests.

---

### Task 1: Lock the command contract with shell tests

**Files:**
- Create: `tests/operations_cli_test.sh`
- Create: `tests/operations_cli_static_test.sh`

- [ ] **Step 1: Write failing dispatch and validation tests**

Create a temporary fake `systemctl`, `journalctl`, `curl`, `sudo`, and Cargo wrapper on `PATH`. Assert that `help` lists every public command, unknown commands return 2, `build` invokes the bounded wrapper with repository `CARGO_TARGET_DIR`, invalid GBrain URLs fail, read-only commands never invoke `sudo`, deploy stops after a failed phase, and closure staging is byte-identical.

- [ ] **Step 2: Run tests and observe missing-entrypoint failures**

Run: `bash tests/operations_cli_static_test.sh && bash tests/operations_cli_test.sh`
Expected: FAIL because `scripts/aletheon.sh` does not exist.

- [ ] **Step 3: Commit tests with the first implementation stage**

The tests and minimal dispatcher are committed together after Task 3 so the commit remains green.

### Task 2: Add shared operations modules

**Files:**
- Create: `scripts/lib/aletheon/common.sh`
- Create: `scripts/lib/aletheon/build.sh`
- Create: `scripts/lib/aletheon/install.sh`
- Create: `scripts/lib/aletheon/service.sh`
- Create: `scripts/lib/aletheon/verify.sh`

- [ ] **Step 1: Implement common helpers**

Provide timestamped `info`, `ok`, `warn`, and `die` functions; `require_command`; canonical repository paths; `validate_http_endpoint` using Python `urllib.parse`; and service/socket paths derived from `XDG_RUNTIME_DIR`.

- [ ] **Step 2: Implement the build boundary**

`cmd_build` exports `CARGO_TARGET_DIR=$ALETHEON_ROOT/target` and executes:

```bash
bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
```

- [ ] **Step 3: Implement install and closure staging**

`cmd_install` requires the release binary and invokes the existing root installer through `sudo env ALETHEON_BINARY=... ALETHEON_CONFIG=...`. `cmd_closure_install` installs the tracked wrapper into `~/.local/bin`, both tracked units into `~/.config/systemd/user`, verifies each with `cmp`, runs `systemd-analyze --user verify`, reloads the user manager, and enables the timer.

- [ ] **Step 4: Implement service operations**

Add non-mutating `status`, scoped `logs`, and `closure status`; mutating `restart` and `closure run`; configuration inspection that prints endpoint locations but never environment values.

- [ ] **Step 5: Implement health and verification**

Use the existing healthcheck for the private user socket, validate the core socket, inspect active service/timer states, compare installed closure assets, check journal evidence for both Pi runtimes, and probe the configured GBrain health origin. Report GBrain failure as degraded while returning failure from the full `verify` gate.

### Task 3: Add the unified dispatcher

**Files:**
- Create: `scripts/aletheon.sh`

- [ ] **Step 1: Source modules in dependency order**

Resolve `SCRIPT_DIR` and `ALETHEON_ROOT`, export them, source common/build/install/service/verify, and register an ERR trap that does not expose command environment values.

- [ ] **Step 2: Implement stable dispatch**

Support `build`, `install`, `configure show|check`, `deploy`, `status`, `health`, `restart`, `logs`, `verify`, `closure install|run|status`, and `help`. Unknown commands print help and return 2.

- [ ] **Step 3: Implement ordered deploy**

Parse only `--no-build`, `--no-restart`, and `--no-enable`; reject all other options. Run build unless disabled, install, closure install, restart unless disabled, then verify. Any failure terminates the pipeline.

- [ ] **Step 4: Run tests**

Run: `bash -n scripts/aletheon.sh scripts/lib/aletheon/*.sh tests/operations_cli*.sh`
Expected: exit 0.

Run: `bash tests/operations_cli_static_test.sh && bash tests/operations_cli_test.sh`
Expected: all operations CLI tests pass.

- [ ] **Step 5: Commit the operations interface**

Commit subject: `feat(operations): add unified deployment command`

Body records the dispatcher, modules, closure installation, endpoint validation, and test coverage.

### Task 4: Create the canonical deployment guide

**Files:**
- Create: `docs/deployment/README.md`
- Modify: `README.md:13-14`
- Modify: `docs/deployment/systemd.md:21-34`
- Modify: `docs/deployment/ser8-acceptance-2026-07-21.md:34-95`

- [ ] **Step 1: Write the operator guide**

Document prerequisites, external secret files, provider configuration, loopback and Tailscale GBrain examples, first installation, repeat deployment, status/health/verify, closure operation, logs, upgrade/rollback, and failure diagnosis. All executable examples use `bash scripts/aletheon.sh ...`.

- [ ] **Step 2: Add prominent navigation**

Add an Operations link near the root README design links. Make `systemd.md` defer the happy path to `docs/deployment/README.md` while retaining boundary details.

- [ ] **Step 3: Correct acceptance topology**

Replace the local-only GBrain assertion with endpoint-configurable wording and distinguish the historical acceptance endpoint from the current SER8 Tailscale deployment. Do not rewrite point-in-time evidence IDs.

### Task 5: Archive superseded process documents

**Files:**
- Create: `docs/archive/README.md`
- Create: `docs/archive/plans/2026-07-21/README.md`
- Move: `docs/plans/2026-07-21-aletheon-ser8-deployment.md`
- Move: `docs/plans/2026-07-21-ser8-pi-memory-closure-design.md`
- Move: `docs/plans/2026-07-21-ser8-pi-memory-closure-implementation.md`
- Modify: repository references found by `rg`

- [ ] **Step 1: Move with Git history**

Use `mkdir -p` and `git mv` into `docs/archive/plans/2026-07-21/`. Do not permanently delete any historical document.

- [ ] **Step 2: Mark the archive non-operational**

State that archived documents preserve requirements and adjudication but commands and status claims may be stale. Link operators to `docs/deployment/README.md`.

- [ ] **Step 3: Repair references**

Run `rg -n 'docs/plans/2026-07-21-(aletheon-ser8-deployment|ser8-pi-memory-closure)' . --glob '!target/**'` and update every active reference to the archive path. Keep line anchors accurate after moves.

- [ ] **Step 4: Validate documentation**

Run a local Python Markdown-link checker over all repository-relative links in changed Markdown files.
Expected: no missing repository-relative targets.

- [ ] **Step 5: Commit documentation convergence**

Commit subject: `docs(deployment): establish one operations guide`

Body records the canonical guide, corrected endpoint topology, and non-destructive archive move.

### Task 6: Validate on SER8 and publish

**Files:**
- Modify only if validation exposes a scoped defect.

- [ ] **Step 1: Run deterministic validation**

```bash
bash tests/operations_cli_static_test.sh
bash tests/operations_cli_test.sh
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p executive --test gbrain_bootstrap
bash scripts/cargo-agent.sh test -p executive --test pi_runtime --test pi_rpc_runtime
```

Expected: all pass.

- [ ] **Step 2: Run read-only live operations**

```bash
bash scripts/aletheon.sh status
bash scripts/aletheon.sh health
bash scripts/aletheon.sh verify
```

Expected: core/user services active, user RPC ready/degraded only for a documented optional component, Pi runtimes registered, closure assets identical, timer active, current GBrain endpoint healthy.

- [ ] **Step 3: Inspect staged diffs and commit scoped fixes**

Use full conventional commit messages with problem/solution context and concrete bullets. Do not mix unrelated changes.

- [ ] **Step 4: Push and open PR to `dev`**

Push the feature branch, create a concise PR, wait for all CI checks, fix scoped failures, and merge only after green CI.

- [ ] **Step 5: Deploy merged `dev`**

Update local `dev`, run `bash scripts/aletheon.sh deploy`, and re-run `status`, `health`, and `verify`. Record the merge commit and installed binary SHA-256 in the final report.
