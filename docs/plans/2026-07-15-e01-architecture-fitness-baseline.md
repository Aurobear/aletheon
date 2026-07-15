# E01 Architecture Fitness Baseline Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CI reject new dependency and execution-path bypasses while preserving the explicitly recorded legacy baseline.

**Architecture:** A repository-local Rust-free checker reads Cargo manifests and ripgrep output, normalizes each finding as `rule|path:line|text`, and compares it with a checked-in allowlist. Exact equality makes the allowlist shrink-only: deleting a known finding passes, adding one fails. The checker observes architecture; it does not change runtime behavior.

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
- Create: `config/architecture-allowlist.txt` — exact current findings.
- Create: `tests/architecture_check.sh` — shrink/add regression fixture.
- Modify: `.github/workflows/ci.yml` — dedicated architecture job.
- Modify: `justfile` — local `architecture-check` recipe.

### Task 1: Add the comparison contract

**Files:** Create `tests/architecture_check.sh`; create `scripts/architecture-check.sh`.

- [ ] **Step 1: Write the failing shell test**

```bash
#!/usr/bin/env bash
set -euo pipefail
root=$(mktemp -d); trap 'rm -rf "$root"' EXIT
mkdir -p "$root/config" "$root/crates/demo/src"
printf 'direct_tool|crates/demo/src/lib.rs:1|tool.execute(x)\n' > "$root/config/architecture-allowlist.txt"
printf 'tool.execute(x)\n' > "$root/crates/demo/src/lib.rs"
ARCH_ROOT="$root" bash scripts/architecture-check.sh
printf 'tool.execute(y)\n' >> "$root/crates/demo/src/lib.rs"
if ARCH_ROOT="$root" bash scripts/architecture-check.sh; then exit 1; fi
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
if ! diff -u "$expected" "$actual"; then
  echo 'architecture-check: baseline changed; remove resolved entries or reject new bypasses' >&2
  exit 1
fi
echo "architecture-check: $(wc -l < "$actual") known findings, no drift"
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

Filter each rule with an adjacent `grep -vFf config/architecture-approved-paths/<rule>.txt` only for modules explicitly approved by the source requirement; do not use directory-wide exclusions.

- [ ] **Step 3: Generate and inspect the baseline**

Run: `ARCH_UPDATE=1 bash scripts/architecture-check.sh && sort config/architecture-allowlist.txt | less`

Add this guarded branch before comparison:

```bash
if [[ ${ARCH_UPDATE:-0} == 1 ]]; then sort -u "$actual" > "$ALLOW"; exit 0; fi
```

Expected: allowlist entries use `rule|path:line|source`; every entry is a verified legacy site.

- [ ] **Step 4: Prove shrink-only behavior**

Run: `bash scripts/architecture-check.sh && cp config/architecture-allowlist.txt /tmp/a && sed -i '1d' config/architecture-allowlist.txt; ! bash scripts/architecture-check.sh; mv /tmp/a config/architecture-allowlist.txt`

Expected: baseline passes; deleting an entry while code remains fails; restored baseline passes.

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
    if dep["name"] in names: print(f"dependency|{p[chr(110)+chr(97)+chr(109)+chr(101)]}|{dep[chr(110)+chr(97)+chr(109)+chr(101)]}")
' | sort -u > "$ROOT/target/architecture-dependencies.actual"
diff -u "$ROOT/config/architecture-dependencies.txt" "$ROOT/target/architecture-dependencies.actual"
```

- [ ] **Step 3: Record and verify the current graph**

Run: `ARCH_UPDATE=1 bash scripts/architecture-check.sh && bash scripts/architecture-check.sh`

Expected: dependency baseline and bypass baseline both match.

### Task 4: Wire local and CI gates

**Files:** Modify `justfile`; modify `.github/workflows/ci.yml`.

- [ ] **Step 1: Add the local recipe**

```make
architecture-check:
    bash tests/architecture_check.sh
    bash scripts/architecture-check.sh
```

- [ ] **Step 2: Add the CI job**

```yaml
  architecture:
    name: architecture fitness
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: sudo apt-get update && sudo apt-get install -y ripgrep
      - run: bash tests/architecture_check.sh
      - run: bash scripts/architecture-check.sh
```

- [ ] **Step 3: Run all gates**

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
