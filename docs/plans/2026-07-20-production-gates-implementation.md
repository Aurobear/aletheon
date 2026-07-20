# Production Gates Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make architecture, deterministic coding evidence, manual Leju evaluation, and Linux `openat2` confinement independently verifiable in CI.

**Architecture:** Ordinary CI remains deterministic and secret-free. A separate manually dispatched workflow owns paid Leju inference and emits artifacts, while the existing Platform contract suite owns Linux confinement attacks and the Linux backend keeps its syscall seam private.

**Tech Stack:** GitHub Actions, Bash, Python 3, Rust/Tokio, Linux `openat2`, existing `scripts/cargo-agent.sh`.

---

## Task 1: Add deterministic coding and Platform contract CI gates

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `tests/coding/workflow_static_test.py`

- [ ] **Step 1: Write the failing workflow contract test**

Create a standard-library Python test which loads `.github/workflows/ci.yml` as text and asserts:

- the job labels `Deterministic coding evidence` and `Linux Platform contract` exist;
- coding CI invokes `python3 tests/coding/replay_test.py` and `bash tests/coding/static_test.sh`;
- the Platform job invokes `bash scripts/cargo-agent.sh test -p platform --test contract_suite`;
- ordinary CI does not reference `LEJU_API_KEY`.

- [ ] **Step 2: Run the test and confirm it fails**

Run: `python3 tests/coding/workflow_static_test.py`

Expected: failure because the two jobs are absent.

- [ ] **Step 3: Add the two CI jobs**

Add independent jobs to `.github/workflows/ci.yml`:

- `coding-evidence`: checkout, then run replay and static tests without secrets;
- `platform-contract`: checkout, install the stable Rust toolchain and required system packages, then run the narrow Platform contract target through `scripts/cargo-agent.sh`.

Keep these jobs independent so GitHub reports deterministic coding evidence and Linux confinement separately. Do not modify architecture baseline policy.

- [ ] **Step 4: Verify the new gates locally**

Run:

```bash
python3 tests/coding/workflow_static_test.py
python3 tests/coding/replay_test.py
bash tests/coding/static_test.sh
bash scripts/cargo-agent.sh test -p platform --test contract_suite
```

Expected: all pass.

- [ ] **Step 5: Commit**

Stage only `.github/workflows/ci.yml` and `tests/coding/workflow_static_test.py`, inspect the staged diff, and commit as `ci(coding): enforce deterministic receipt gates` with a problem/solution body and concrete file bullets.

## Task 2: Add the manually dispatched Leju evaluation workflow

**Files:**
- Create: `.github/workflows/coding-e2e.yml`
- Modify: `tests/coding/workflow_static_test.py`
- Modify: `tests/coding/README.md`

- [ ] **Step 1: Extend the workflow contract test first**

Assert that the new workflow:

- is triggered only by `workflow_dispatch`;
- pins provider `leju` and model `deepseek/deepseek-v4-pro`;
- reads `${{ secrets.LEJU_API_KEY }}` and never prints the key;
- runs `rust_bugfix`, `rust_multifile`, and `rust_diagnosis` sequentially;
- uploads artifacts with `if: always()`.

- [ ] **Step 2: Run the test and confirm it fails**

Run: `python3 tests/coding/workflow_static_test.py`

Expected: failure because `.github/workflows/coding-e2e.yml` is absent.

- [ ] **Step 3: Implement the real evaluation workflow**

Create `.github/workflows/coding-e2e.yml` with these fixed environment values:

```yaml
ALETHEON_PROVIDER: leju
ALETHEON_MODEL: deepseek/deepseek-v4-pro
LEJU_API_KEY: ${{ secrets.LEJU_API_KEY }}
CARGO_BUILD_JOBS: "1"
```

The workflow must:

1. install `libxcb1-dev`, `ripgrep`, and `bubblewrap`;
2. build required binaries through `bash scripts/cargo-agent.sh`, never raw `cargo`;
3. reject an empty secret before core startup;
4. create a mode-`0600` temporary config using Python `json.dumps(os.environ["LEJU_API_KEY"])` rather than shell interpolation;
5. start `$ALETHEON_BIN core --config "$config" --socket "$ALETHEON_CORE_SOCKET"` once;
6. install a trap that stops the core and removes the credential-bearing config;
7. wait for the core socket with a bounded timeout;
8. run the three fixtures sequentially through `tests/coding/harness/run.py`, each with a distinct receipt path under `$RUNNER_TEMP/coding-artifacts`;
9. upload receipts and the core log with `actions/upload-artifact@v4` and `if: always()`.

Uploaded paths must exclude the temporary config and temporary HOME.

- [ ] **Step 4: Document operation and failure classes**

Update `tests/coding/README.md` with the manual workflow name, required `LEJU_API_KEY` repository secret, pinned provider/model, artifact contents, and the distinction between infrastructure failure and acceptance failure.

- [ ] **Step 5: Verify the workflow contract**

Run:

```bash
python3 tests/coding/workflow_static_test.py
git diff --check
```

Expected: pass without invoking an external model.

- [ ] **Step 6: Commit**

Inspect the staged diff and commit as `ci(coding): add manual Leju evaluation` with a full message body.

## Task 3: Test scoped-root replacement

**Files:**
- Modify: `crates/platform/tests/contract_suite.rs`

- [ ] **Step 1: Add the adversarial contract test**

On Unix, create an admitted root, a moved-root destination, and an outside directory containing a sentinel. Construct the scoped filesystem host first, rename the admitted root, and replace its former pathname with a symlink to the outside directory.

Attempt atomic write, read, and remove operations through the already-created host. Each operation may either continue through the pinned root descriptor or return its documented typed error, but it must never affect the outside sentinel or create a temporary file outside.

- [ ] **Step 2: Run the narrow test**

Run: `bash scripts/cargo-agent.sh test -p platform --test contract_suite root_replacement -- --nocapture`

Expected: pass on Linux; non-Linux compilation remains protected by `cfg` attributes.

- [ ] **Step 3: Run the full contract target**

Run: `bash scripts/cargo-agent.sh test -p platform --test contract_suite`

Expected: pass.

- [ ] **Step 4: Commit**

Inspect the staged diff and commit as `test(platform): cover scoped-root replacement` with a full message body.

## Task 4: Test parent-directory swap pressure

**Files:**
- Modify: `crates/platform/tests/contract_suite.rs`

- [ ] **Step 1: Add the pressure test**

Create a worker thread that repeatedly swaps a workspace parent entry between an admitted directory and a symlink to an outside directory. Concurrently perform a bounded series of atomic writes through the scoped host.

Typed operation failures are acceptable during the race. After joining the worker, assert that the outside sentinel is unchanged and that no escaped `.tmp` file exists.

- [ ] **Step 2: Run the test repeatedly but sequentially**

Run:

```bash
for attempt in 1 2 3 4 5; do
  bash scripts/cargo-agent.sh test -p platform --test contract_suite parent_swap_pressure -- --nocapture || exit 1
done
```

Expected: five passes without launching concurrent Cargo builds.

- [ ] **Step 3: Run the full contract target**

Run: `bash scripts/cargo-agent.sh test -p platform --test contract_suite`

Expected: pass.

- [ ] **Step 4: Commit**

Inspect the staged diff and commit as `test(platform): exercise parent swap pressure` with a full message body.

## Task 5: Prove `openat2` unavailability fails closed

**Files:**
- Modify: `crates/platform/src/backend/linux/filesystem_host.rs`

- [ ] **Step 1: Add a private test-only syscall seam**

Add a `#[cfg(test)]` `AtomicBool` named for forced `openat2` unavailability. At the start of the private `open_beneath` path, return an error derived from `ENOSYS` whose detail states that `openat2` is unavailable and the scoped filesystem fails closed.

The production branch must continue to call `libc::SYS_openat2` directly. Do not expose the seam through the Platform trait or public API and do not add a canonicalization fallback.

- [ ] **Step 2: Add a serialized module test**

Within the same module, protect the global injection flag with a static mutex and reset it through an RAII guard. Construct the scoped host before enabling the flag, attempt a read, and assert:

- the error kind is `HostErrorKind::Io`;
- the detail contains `fails closed`;
- the source file is unchanged.

- [ ] **Step 3: Run focused and package tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p platform openat2_unavailable -- --nocapture
bash scripts/cargo-agent.sh test -p platform
```

Expected: pass.

- [ ] **Step 4: Commit**

Inspect the staged diff and commit as `test(platform): verify openat2 fail-closed behavior` with a full message body.

## Task 6: Run final verification and obtain remote proof

**Files:**
- Modify: `docs/plans/2026-07-20-production-gates-implementation.md`

- [ ] **Step 1: Run deterministic local validation sequentially**

```bash
bash scripts/cargo-agent.sh test -p platform
python3 tests/coding/workflow_static_test.py
python3 tests/coding/replay_test.py
bash tests/coding/static_test.sh
bash scripts/architecture-check.sh
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

Expected: every command passes. Do not overlap the Platform and architecture builds.

- [ ] **Step 2: Record completion in this plan**

Mark completed checkboxes and add a short verification-results section containing commands and outcomes. Commit this documentation-only update with a full conventional commit message.

- [ ] **Step 3: Publish the feature branch and open a PR to `dev`**

Use the repository commit workflow. Push `auro/ci/20260720-production-gates`, open a PR targeting `dev`, and monitor the architecture, deterministic coding, and Linux Platform contract checks. The remote architecture check is the closure evidence for the historical PR #106 failure.

- [ ] **Step 4: Run the paid evaluation when credentials permit**

Check only whether the `LEJU_API_KEY` Actions secret exists; never display its value. If present, manually dispatch `coding-e2e.yml` and verify the artifact contains three independent receipts plus the core log. If absent, report the missing repository secret as the sole external blocker; do not substitute local credentials or claim the paid gate passed.
