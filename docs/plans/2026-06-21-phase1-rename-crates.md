# Phase 1: Rename Crates Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename all crates to remove the `aletheon-` prefix and apply new names (base, corpus, cognit, memory, dasein, runtime, metacog), plus rename binaries (daemon, cli, exec).

**Architecture:** Pure mechanical rename — directory renames, Cargo.toml `name` field updates, workspace member path updates, and `use` statement updates across all source files. No structural changes.

**Tech Stack:** Rust, Cargo workspace

---

## Naming Map

| Old Name | New Name | Directory |
|---|---|---|
| `aletheon-abi` | `base` | `crates/aletheon-abi` → `crates/base` |
| `aletheon-body` | `corpus` | `crates/aletheon-body` → `crates/corpus` |
| `aletheon-brain` | `cognit` | `crates/aletheon-brain` → `crates/cognit` |
| `aletheon-comm` | `comm` | `crates/aletheon-comm` → `crates/comm` |
| `aletheon-memory` | `memory` | `crates/aletheon-memory` → `crates/memory` |
| `aletheon-self` | `dasein` | `crates/aletheon-self` → `crates/dasein` |
| `aletheon-runtime` | `runtime` | `crates/aletheon-runtime` → `crates/runtime` |
| `aletheon-meta` | `metacog` | `crates/aletheon-meta` → `crates/metacog` |
| `aletheond` | `daemon` | `crates/binaries/aletheond` → `crates/binaries/daemon` |
| `aletheon-cli` | `cli` | `crates/binaries/aletheon-cli` → `crates/binaries/cli` |
| `aletheon-exec` | `exec` | `crates/binaries/aletheon-exec` → `crates/binaries/exec` |

---

## Task 1: Rename Library Crate Directories

**Files:**
- Modify: `crates/aletheon-abi/` → `crates/base/`
- Modify: `crates/aletheon-body/` → `crates/corpus/`
- Modify: `crates/aletheon-brain/` → `crates/cognit/`
- Modify: `crates/aletheon-comm/` → `crates/comm/`
- Modify: `crates/aletheon-memory/` → `crates/memory/`
- Modify: `crates/aletheon-self/` → `crates/dasein/`
- Modify: `crates/aletheon-runtime/` → `crates/runtime/`
- Modify: `crates/aletheon-meta/` → `crates/metacog/`

- [ ] **Step 1: Rename all library crate directories**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
mv crates/aletheon-abi crates/base
mv crates/aletheon-body crates/corpus
mv crates/aletheon-brain crates/cognit
mv crates/aletheon-comm crates/comm
mv crates/aletheon-memory crates/memory
mv crates/aletheon-self crates/dasein
mv crates/aletheon-runtime crates/runtime
mv crates/aletheon-meta crates/metacog
```

- [ ] **Step 2: Verify directories exist**

```bash
ls -la crates/
```

Expected: `base/ corpus/ cognit/ comm/ memory/ dasein/ runtime/ metacog/ binaries/`

---

## Task 2: Rename Binary Crate Directories

**Files:**
- Modify: `crates/binaries/aletheond/` → `crates/binaries/daemon/`
- Modify: `crates/binaries/aletheon-cli/` → `crates/binaries/cli/`
- Modify: `crates/binaries/aletheon-exec/` → `crates/binaries/exec/`

- [ ] **Step 1: Rename binary crate directories**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
mv crates/binaries/aletheond crates/binaries/daemon
mv crates/binaries/aletheon-cli crates/binaries/cli
mv crates/binaries/aletheon-exec crates/binaries/exec
```

- [ ] **Step 2: Verify directories exist**

```bash
ls -la crates/binaries/
```

Expected: `daemon/ cli/ exec/`

---

## Task 3: Update Workspace Cargo.toml

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Read current workspace Cargo.toml**

```bash
cat Cargo.toml
```

- [ ] **Step 2: Update workspace members**

Replace the `members` list in `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/binaries/daemon",
    "crates/binaries/exec",
    "crates/binaries/cli",
    "crates/base",
    "crates/comm",
    "crates/memory",
    "crates/dasein",
    "crates/cognit",
    "crates/corpus",
    "crates/runtime",
    "crates/metacog",
    "examples/basic-agent",
    "examples/self-evolution-loop",
]
```

- [ ] **Step 3: Verify workspace resolves**

```bash
cargo metadata --format-version 1 | head -5
```

Expected: JSON output with workspace members listed.

---

## Task 4: Update Library Crate Cargo.toml Names

**Files:**
- Modify: `crates/base/Cargo.toml`
- Modify: `crates/corpus/Cargo.toml`
- Modify: `crates/cognit/Cargo.toml`
- Modify: `crates/comm/Cargo.toml`
- Modify: `crates/memory/Cargo.toml`
- Modify: `crates/dasein/Cargo.toml`
- Modify: `crates/runtime/Cargo.toml`
- Modify: `crates/metacog/Cargo.toml`

- [ ] **Step 1: Update base (was aletheon-abi)**

In `crates/base/Cargo.toml`, change:
```toml
[package]
name = "aletheon-abi"
```
to:
```toml
[package]
name = "base"
```

- [ ] **Step 2: Update corpus (was aletheon-body)**

In `crates/corpus/Cargo.toml`, change:
```toml
[package]
name = "aletheon-body"
```
to:
```toml
[package]
name = "corpus"
```

- [ ] **Step 3: Update cognit (was aletheon-brain)**

In `crates/cognit/Cargo.toml`, change:
```toml
[package]
name = "aletheon-brain"
```
to:
```toml
[package]
name = "cognit"
```

- [ ] **Step 4: Update comm (was aletheon-comm)**

In `crates/comm/Cargo.toml`, change:
```toml
[package]
name = "aletheon-comm"
```
to:
```toml
[package]
name = "comm"
```

- [ ] **Step 5: Update memory (was aletheon-memory)**

In `crates/memory/Cargo.toml`, change:
```toml
[package]
name = "aletheon-memory"
```
to:
```toml
[package]
name = "memory"
```

- [ ] **Step 6: Update dasein (was aletheon-self)**

In `crates/dasein/Cargo.toml`, change:
```toml
[package]
name = "aletheon-self"
```
to:
```toml
[package]
name = "dasein"
```

- [ ] **Step 7: Update runtime (was aletheon-runtime)**

In `crates/runtime/Cargo.toml`, change:
```toml
[package]
name = "aletheon-runtime"
```
to:
```toml
[package]
name = "runtime"
```

- [ ] **Step 8: Update metacog (was aletheon-meta)**

In `crates/metacog/Cargo.toml`, change:
```toml
[package]
name = "aletheon-meta"
```
to:
```toml
[package]
name = "metacog"
```

---

## Task 5: Update Binary Crate Cargo.toml Names

**Files:**
- Modify: `crates/binaries/daemon/Cargo.toml`
- Modify: `crates/binaries/cli/Cargo.toml`
- Modify: `crates/binaries/exec/Cargo.toml`

- [ ] **Step 1: Update daemon (was aletheond)**

In `crates/binaries/daemon/Cargo.toml`, change:
```toml
[package]
name = "aletheond"
```
to:
```toml
[package]
name = "daemon"
```

- [ ] **Step 2: Update cli (was aletheon-cli)**

In `crates/binaries/cli/Cargo.toml`, change:
```toml
[package]
name = "aletheon-cli"
```
to:
```toml
[package]
name = "cli"
```

- [ ] **Step 3: Update exec (was aletheon-exec)**

In `crates/binaries/exec/Cargo.toml`, change:
```toml
[package]
name = "aletheon-exec"
```
to:
```toml
[package]
name = "exec"
```

---

## Task 6: Update Inter-Crate Dependencies in Cargo.toml Files

**Files:**
- Modify: All `Cargo.toml` files that reference `aletheon-*` dependencies

- [ ] **Step 1: Find all Cargo.toml files with aletheon dependencies**

```bash
grep -r "aletheon-" crates/*/Cargo.toml crates/binaries/*/Cargo.toml examples/*/Cargo.toml
```

- [ ] **Step 2: Update base dependencies**

In each `Cargo.toml` that has:
```toml
aletheon-abi = { path = "..." }
```
Replace with:
```toml
base = { path = "..." }
```

Similarly for all other `aletheon-*` dependencies:
- `aletheon-body` → `corpus`
- `aletheon-brain` → `cognit`
- `aletheon-comm` → `comm`
- `aletheon-memory` → `memory`
- `aletheon-self` → `dasein`
- `aletheon-runtime` → `runtime`
- `aletheon-meta` → `metacog`

- [ ] **Step 3: Verify no remaining aletheon references in Cargo.toml**

```bash
grep -r "aletheon-" crates/*/Cargo.toml crates/binaries/*/Cargo.toml examples/*/Cargo.toml
```

Expected: No output (all references replaced).

---

## Task 7: Update `use` Statements in Source Files

**Files:**
- Modify: All `.rs` files with `use aletheon_*` imports

- [ ] **Step 1: Find all use statements with aletheon prefix**

```bash
grep -rn "use aletheon_" crates/ examples/ --include="*.rs"
```

- [ ] **Step 2: Replace use statements**

For each file, replace:
- `use aletheon_abi::` → `use base::`
- `use aletheon_body::` → `use corpus::`
- `use aletheon_brain::` → `use cognit::`
- `use aletheon_comm::` → `use comm::`
- `use aletheon_memory::` → `use memory::`
- `use aletheon_self::` → `use dasein::`
- `use aletheon_runtime::` → `use runtime::`
- `use aletheon_meta::` → `use metacog::`

**Note:** This can be done with sed:
```bash
find crates/ examples/ -name "*.rs" -exec sed -i \
  -e 's/use aletheon_abi/use base/g' \
  -e 's/use aletheon_body/use corpus/g' \
  -e 's/use aletheon_brain/use cognit/g' \
  -e 's/use aletheon_comm/use comm/g' \
  -e 's/use aletheon_memory/use memory/g' \
  -e 's/use aletheon_self/use dasein/g' \
  -e 's/use aletheon_runtime/use runtime/g' \
  -e 's/use aletheon_meta/use metacog/g' \
  {} +
```

- [ ] **Step 3: Verify no remaining aletheon use statements**

```bash
grep -rn "use aletheon_" crates/ examples/ --include="*.rs"
```

Expected: No output.

---

## Task 8: Update `extern crate` Statements

**Files:**
- Modify: Any `.rs` files with `extern crate aletheon_*`

- [ ] **Step 1: Find extern crate statements**

```bash
grep -rn "extern crate aletheon_" crates/ examples/ --include="*.rs"
```

- [ ] **Step 2: Replace extern crate statements**

For each file, replace:
- `extern crate aletheon_abi` → `extern crate base`
- `extern crate aletheon_body` → `extern crate corpus`
- etc.

- [ ] **Step 3: Verify no remaining aletheon extern crate**

```bash
grep -rn "extern crate aletheon_" crates/ examples/ --include="*.rs"
```

Expected: No output.

---

## Task 9: Update Documentation References

**Files:**
- Modify: `docs/` directory (all .md files)
- Modify: `README.md` (if exists)
- Modify: `CLAUDE.md` (if exists)

- [ ] **Step 1: Find documentation references**

```bash
grep -rn "aletheon-" docs/ README.md CLAUDE.md --include="*.md"
```

- [ ] **Step 2: Update documentation**

Replace all references to old crate names with new names in documentation files.

- [ ] **Step 3: Verify no remaining old references in docs**

```bash
grep -rn "aletheon-abi\|aletheon-body\|aletheon-brain\|aletheon-comm\|aletheon-memory\|aletheon-self\|aletheon-runtime\|aletheon-meta\|aletheond\|aletheon-cli\|aletheon-exec" docs/ README.md CLAUDE.md --include="*.md"
```

Expected: No output (except possibly in historical/archival documents).

---

## Task 10: Update Example Crates

**Files:**
- Modify: `examples/basic-agent/Cargo.toml`
- Modify: `examples/basic-agent/src/*.rs`
- Modify: `examples/self-evolution-loop/Cargo.toml`
- Modify: `examples/self-evolution-loop/src/*.rs`

- [ ] **Step 1: Update example Cargo.toml dependencies**

```bash
grep -r "aletheon-" examples/*/Cargo.toml
```

Replace all `aletheon-*` dependencies with new names.

- [ ] **Step 2: Update example source files**

```bash
grep -rn "use aletheon_" examples/ --include="*.rs"
```

Replace all `use aletheon_*` with new names.

- [ ] **Step 3: Verify examples compile**

```bash
cargo check -p basic-agent-example
cargo check -p self-evolution-loop-example
```

Expected: Both compile successfully.

---

## Task 11: Verify Full Workspace Compilation

- [ ] **Step 1: Run cargo check on entire workspace**

```bash
cargo check --workspace
```

Expected: All crates compile without errors.

- [ ] **Step 2: Run cargo test on entire workspace**

```bash
cargo test --workspace
```

Expected: All tests pass.

- [ ] **Step 3: Verify no remaining aletheon references**

```bash
grep -rn "aletheon-" crates/ examples/ --include="*.rs" --include="*.toml"
```

Expected: No output (all references replaced).

---

## Task 12: Commit Changes

- [ ] **Step 1: Stage all changes**

```bash
git add -A
```

- [ ] **Step 2: Commit**

```bash
git commit -m "refactor: rename all crates — remove aletheon- prefix

- aletheon-abi → base
- aletheon-body → corpus
- aletheon-brain → cognit
- aletheon-comm → comm
- aletheon-memory → memory
- aletheon-self → dasein
- aletheon-runtime → runtime
- aletheon-meta → metacog
- aletheond → daemon
- aletheon-cli → cli
- aletheon-exec → exec

Pure mechanical rename, no structural changes."
```

---

## Self-Review Checklist

1. **Spec coverage:** This plan covers Phase 1 of the architectural redesign spec (§8.1)
2. **Placeholder scan:** No TBD/TODO — all steps are concrete
3. **Type consistency:** Naming map is consistent throughout
4. **Verification:** Each task has explicit verification steps
5. **Risk mitigation:** Incremental verification at each step

---

## Execution Options

Plan complete and saved to `docs/plans/2026-06-21-phase1-rename-crates.md`.

Execution options:
1. **workflow-feature** — Multi-agent pipeline with approval gates
2. **Inline execution** — Execute tasks in this session with checkpoints

Which approach?
