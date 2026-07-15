# E01 Architecture Fitness Baseline Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CI reject new dependency and execution-path bypasses while preserving the explicitly recorded legacy baseline.

**Architecture:** A repository-local checker normalizes Cargo edges and source findings, then rejects only `actual - baseline`. Resolved findings may disappear, while new findings always fail; stale allowlist entries are reported for deletion. This is genuinely shrink-only rather than exact equality, which would incorrectly fail when code improves.

**Tech Stack:** Bash, ripgrep, Cargo metadata, GitHub Actions

**Prerequisites:** None; start from baseline `65f74981` or a descendant containing the same symbols.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:937-956`, `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1201-1210`.

---

## Current anchors and invariants

- CI currently runs workspace checks but no architecture job: `.github/workflows/ci.yml:12-86`.
- The governed path exists only as a Kernel contract: `crates/kernel/src/capability/mod.rs:27-50`.
- Both manual capability paths must remain visible until E03/S02 remove them: `crates/executive/src/service/exec_session.rs:218-316`, `crates/executive/src/service/turn_pipeline.rs:392-452`.
- The check must be deterministic, require only Bash/ripgrep/Cargo, and print an actionable diff.
- Non-goals: changing dependencies, moving code, or deleting legacy call sites.

```text
source tree -> rule scanners -> normalized findings -> allowlist comparison -> pass/fail
```

## File map

- Create: `scripts/architecture-check.sh` — rule runner and comparison logic.
- Create: `config/architecture-allowlist.txt` — maximum set of current findings; adding entries is forbidden.
- Create: `config/architecture-dependencies.txt` — maximum set of current workspace edges.
- Create: `config/architecture-path-inventory.txt` — current turn and capability production paths.
- Create: `tests/architecture_check.sh` — shrink/add regression fixture.
- Create: `tests/architecture_path_inventory.sh` — migration inventory for both turn paths and every capability path.
- Modify: `.github/workflows/ci.yml` — dedicated architecture job.
- Modify: `justfile` — local `architecture-check` recipe.

### Task 1: Add the comparison contract

**Files:** Create `tests/architecture_check.sh`; create `scripts/architecture-check.sh`.

- [ ] **Step 1: Write the failing shell test**

```bash
#!/usr/bin/env bash
set -euo pipefail
root=$(mktemp -d); trap 'rm -rf "$root"' EXIT
mkdir -p "$root/config" "$root/crates/corpus/src/legacy"
printf 'direct_tool|crates/corpus/src/legacy/mod.rs:1|tool.execute(x)\n' > "$root/config/architecture-allowlist.txt"
printf 'tool.execute(x)\n' > "$root/crates/corpus/src/legacy/mod.rs"
ARCH_ROOT="$root" bash scripts/architecture-check.sh
printf 'tool.execute(y)\n' >> "$root/crates/corpus/src/legacy/mod.rs"
if ARCH_ROOT="$root" bash scripts/architecture-check.sh; then exit 1; fi
rm "$root/crates/corpus/src/legacy/mod.rs"
ARCH_ROOT="$root" bash scripts/architecture-check.sh
```

- [ ] **Step 2: Verify the missing checker fails**

Run: `bash tests/architecture_check.sh`

Expected: FAIL with `scripts/architecture-check.sh: No such file or directory`.

- [ ] **Step 3: Implement exact-set comparison and the direct-tool scanner**

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT=${ARCH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
ALLOW="$ROOT/config/architecture-allowlist.txt"
actual=$(mktemp); expected=$(mktemp); trap 'rm -f "$actual" "$expected"' EXIT
cd "$ROOT"
rg -n --no-heading '(^|[^[:alnum:]_])([[:alnum:]_]+\.)?execute\(' crates/corpus crates/executive crates/bin \
  -g '*.rs' | grep -vE '^crates/corpus/src/(security/runner|tools/tools/executor)\.rs:' \
  | sed 's/^/direct_tool|/' >> "$actual" || true
sort -u "$ALLOW" > "$expected"; sort -u "$actual" -o "$actual"
comm -23 "$actual" "$expected" > "$actual.new"
comm -13 "$actual" "$expected" > "$actual.stale"
if [[ -s "$actual.new" ]]; then
  echo 'architecture-check: new forbidden findings:' >&2
  cat "$actual.new" >&2
  exit 1
fi
if [[ -s "$actual.stale" ]]; then
  echo 'architecture-check: resolved allowlist entries (delete in this change):'
  cat "$actual.stale"
fi
echo "architecture-check: $(wc -l < "$actual") findings, no additions"
```

- [ ] **Step 4: Run the fixture**

Run: `bash tests/architecture_check.sh`

Expected: first run prints `no drift`; second run produces a unified diff; script exits 0.

### Task 2: Encode all required scanners and the current baseline

**Files:** Modify `scripts/architecture-check.sh`; create `config/architecture-allowlist.txt`.

- [ ] **Step 1: Add a fixture case for every rule**

Extend `tests/architecture_check.sh` to create one line for `Envelope`, deprecated `Event`, `SystemClock::new()`, `.runtime.`, and `executive::impl::kernel`, then assert that an empty allowlist fails:

```bash
: > "$root/config/architecture-allowlist.txt"
if output=$(ARCH_ROOT="$root" bash scripts/architecture-check.sh 2>&1); then exit 1; fi
for rule in legacy_event concrete_clock core_systems_field duplicate_kernel; do
  grep -q "$rule|" <<<"$output"
done
```

- [ ] **Step 2: Add named scanners**

In `scripts/architecture-check.sh`, append normalized `rg` results for:

```bash
scan() { local rule=$1 pattern=$2; shift 2; rg -n --no-heading "$pattern" "$@" -g '*.rs' | sed "s/^/$rule|/" >> "$actual" || true; }
scan legacy_event '\b(Envelope|Event)\b' crates
scan concrete_clock 'SystemClock::new\(' crates/dasein crates/agora crates/cognit crates/mnemosyne crates/metacog crates/interact
scan core_systems_field '\.(runtime|domain|infra|orchestration|memory)\.' crates/executive/src crates/bin/src
scan duplicate_kernel 'executive::impl::kernel|crate::impl::kernel' crates
```

Exclude approved Corpus runtime modules and tests directly in the scanner command. Do not introduce a second path-approval mechanism: every remaining legacy production match belongs in `config/architecture-allowlist.txt`.

- [ ] **Step 3: Generate and inspect the baseline**

Run: `ARCH_UPDATE=1 bash scripts/architecture-check.sh && sort config/architecture-allowlist.txt | less`

Add this guarded branch before comparison:

```bash
if [[ ${ARCH_UPDATE:-0} == 1 ]]; then sort -u "$actual" > "$ALLOW"; exit 0; fi
```

Expected: allowlist entries use `rule|path|source` (diagnostics retain current line numbers); every entry is a verified legacy site.

- [ ] **Step 4: Prove shrink-only behavior**

Run: `bash scripts/architecture-check.sh && cp config/architecture-allowlist.txt /tmp/a && sed -i '1d' config/architecture-allowlist.txt; ! bash scripts/architecture-check.sh; mv /tmp/a config/architecture-allowlist.txt`

Expected: baseline passes; deleting an entry while code remains fails as a new finding; deleting the corresponding code first passes and reports a stale entry.

### Task 3: Gate workspace dependency edges

**Files:** Modify `scripts/architecture-check.sh`; create `config/architecture-dependencies.txt`.

- [ ] **Step 1: Add a failing unexpected-edge fixture**

Append a fixture workspace with `fabric -> kernel` and assert output contains `dependency|fabric|aletheon-kernel`.

- [ ] **Step 2: Extract local edges with Cargo metadata**

```bash
cargo metadata --no-deps --format-version 1 | python3 -c '
import json,sys
d=json.load(sys.stdin); names={p["name"] for p in d["packages"]}
for p in d["packages"]:
  for dep in p["dependencies"]:
    if dep["name"] in names: print("dependency|{}|{}".format(p["name"], dep["name"]))
' | sort -u > "$ROOT/target/architecture-dependencies.actual"
diff -u "$ROOT/config/architecture-dependencies.txt" "$ROOT/target/architecture-dependencies.actual"
```

- [ ] **Step 3: Record and verify the current graph**

Run: `ARCH_UPDATE=1 bash scripts/architecture-check.sh && bash scripts/architecture-check.sh`

Expected: dependency baseline and bypass baseline both match.

### Task 4: Wire local and CI gates

**Files:** Modify `justfile`; modify `.github/workflows/ci.yml`.

- [ ] **Step 1: Add the migration inventory test**

Create `tests/architecture_path_inventory.sh` with exact `rg` assertions for `TurnService`, `TurnPipeline`, `ExecTurnServices::invoke`, daemon closure admission, `DefaultCapabilityInvoker`, and every `impl CapabilityInvoker`. It writes sorted `kind|path|symbol` output and compares it with `config/architecture-path-inventory.txt`. A new path fails; E02/E03/S02 delete resolved entries as they converge.

- [ ] **Step 2: Reject allowlist additions relative to the target branch**

Add to the checker when `ARCH_BASE_REF` is set:

```bash
for file in config/architecture-allowlist.txt config/architecture-dependencies.txt config/architecture-path-inventory.txt; do
  if git diff --unified=0 "$ARCH_BASE_REF" -- "$file" | grep -q '^+[^+]'; then
    echo "architecture-check: $file may only lose entries" >&2
    exit 1
  fi
done
```

CI fetches the pull-request base and runs with `ARCH_BASE_REF=origin/${{ github.base_ref }}`. `ARCH_UPDATE=1` is a bootstrap/local regeneration aid only; it is never accepted as evidence for adding entries.

- [ ] **Step 3: Add the local recipe**

```make
architecture-check:
    bash tests/architecture_check.sh
    bash tests/architecture_path_inventory.sh
    bash scripts/architecture-check.sh
```

- [ ] **Step 4: Add the CI job**

```yaml
  architecture:
    name: architecture fitness
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: sudo apt-get update && sudo apt-get install -y ripgrep
      - run: bash tests/architecture_check.sh
      - run: bash tests/architecture_path_inventory.sh
      - run: bash scripts/architecture-check.sh
```

- [ ] **Step 5: Correct stale architecture claims**

Run `rg -n 'all inter-subsystem|single turn path|all capabilities|fabric only' docs README.md crates/*/README.md`. For each statement contradicted by the generated dependency/path inventories, update it to describe verified current behavior and link the migration plan. Re-run the same search and inspect every remaining match.

- [ ] **Step 6: Run all gates**

Run: `just architecture-check && cargo fmt --all -- --check && cargo check --workspace --all-targets`

Expected: all commands exit 0.

### Task 5: Commit the vertical baseline

- [ ] Inspect: `git diff --check && git diff -- scripts/architecture-check.sh tests/architecture_check.sh config/architecture-allowlist.txt config/architecture-dependencies.txt justfile .github/workflows/ci.yml`
- [ ] Commit with:

```text
test(architecture): freeze dependency and bypass drift

Architecture migrations had no executable boundary, allowing new legacy calls
to appear while old paths were being removed. Add deterministic dependency and
source scanners with an exact, shrink-only legacy baseline.

- gate workspace dependency changes
- inventory capability, clock, event, and kernel bypasses
- run the architecture fitness suite locally and in CI
```

## Compatibility deletion gate and completion evidence

Delete an allowlist entry in the same commit that removes its source finding. Delete the checker only after an equivalent typed compiler boundary covers every rule and CI proves it.

- [ ] `bash tests/architecture_check.sh` passes.
- [ ] `bash scripts/architecture-check.sh` passes twice with byte-identical output.
- [ ] Adding one forbidden fixture line fails with its rule and locator.
- [ ] Workspace format/check gates pass.
