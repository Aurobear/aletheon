# Phase 3: Split corpus Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the corpus crate (formerly aletheon-body) into 5 focused crates: corpus (core), drivers, tools, security, interact.

**Architecture:** Based on cross-module dependency analysis, sandbox moves with security (15 imports), interact depends on drivers (17 imports from acix→driver). Each new crate depends only on base.

**Tech Stack:** Rust, Cargo workspace

---

## Split Map

| New Crate | Source Modules | Files | Dependencies |
|---|---|---|---|
| `corpus` | core/, bridge/, testing/ | ~5 | base |
| `drivers` | impl/driver/, impl/platform/ | ~29 | base |
| `tools` | impl/tools/, impl/hooks/, impl/skills/, impl/mcp/ | ~48 | base |
| `security` | impl/security/, impl/sandbox/ | ~24 | base |
| `interact` | impl/ui/, impl/cli/, impl/acix/ | ~27 | base, drivers |

---

## Task 1: Create new crate directories

**Files:**
- Create: `crates/drivers/`
- Create: `crates/tools/`
- Create: `crates/security/`
- Create: `crates/interact/`

- [ ] **Step 1: Create crate directories**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
mkdir -p crates/drivers/src
mkdir -p crates/tools/src
mkdir -p crates/security/src
mkdir -p crates/interact/src
```

- [ ] **Step 2: Create Cargo.toml for each new crate**

Create `crates/drivers/Cargo.toml`:
```toml
[package]
name = "drivers"
version = "0.1.0"
edition = "2021"

[dependencies]
base = { path = "../base" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
libc = { workspace = true }
nix = { workspace = true, optional = true }
x11rb = { version = "0.13", optional = true }
atspi = { version = "0.5", optional = true }
tesseract = { version = "0.14", optional = true }
zbus = { version = "3", optional = true }

[features]
default = []
input = ["nix"]
display = ["nix", "x11rb"]
a11y = ["atspi"]
ocr = []
ocr-tesseract = ["ocr", "tesseract"]
dbus = ["zbus"]
```

Create `crates/tools/Cargo.toml`:
```toml
[package]
name = "tools"
version = "0.1.0"
edition = "2021"

[dependencies]
base = { path = "../base" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
reqwest = { version = "0.11", features = ["json"] }
tree-sitter = "0.20"
tempfile = "3"
which = "4"
walkdir = "2"
toml = "0.8"
```

Create `crates/security/Cargo.toml`:
```toml
[package]
name = "security"
version = "0.1.0"
edition = "2021"

[dependencies]
base = { path = "../base" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
nix = { workspace = true, optional = true }

[features]
default = []
sandbox-primitives = ["nix"]
```

Create `crates/interact/Cargo.toml`:
```toml
[package]
name = "interact"
version = "0.1.0"
edition = "2021"

[dependencies]
base = { path = "../base" }
drivers = { path = "../drivers" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
ratatui = "0.24"
crossterm = "0.27"
pulldown-cmark = "0.9"
syntect = "5"
clap = { version = "4", features = ["derive"] }
atty = "0.2"
```

---

## Task 2: Move driver and platform to drivers crate

**Files:**
- Move: `crates/corpus/src/impl/driver/` → `crates/drivers/src/driver/`
- Move: `crates/corpus/src/impl/platform/` → `crates/drivers/src/platform/`

- [ ] **Step 1: Copy driver directory**

```bash
cp -r crates/corpus/src/impl/driver crates/drivers/src/
cp -r crates/corpus/src/impl/platform crates/drivers/src/
```

- [ ] **Step 2: Create drivers/lib.rs**

Create `crates/drivers/src/lib.rs`:
```rust
//! Hardware drivers and platform adapters.

pub mod driver;
pub mod platform;
```

- [ ] **Step 3: Fix internal imports**

Replace `crate::impl::driver` with `crate::driver` in all moved files:
```bash
find crates/drivers/src/ -name "*.rs" -exec sed -i 's/crate::impl::driver/crate::driver/g' {} +
find crates/drivers/src/ -name "*.rs" -exec sed -i 's/crate::impl::platform/crate::platform/g' {} +
```

---

## Task 3: Move tools, hooks, skills, mcp to tools crate

**Files:**
- Move: `crates/corpus/src/impl/tools/` → `crates/tools/src/tools/`
- Move: `crates/corpus/src/impl/hooks/` → `crates/tools/src/hooks/`
- Move: `crates/corpus/src/impl/skills/` → `crates/tools/src/skills/`
- Move: `crates/corpus/src/impl/mcp/` → `crates/tools/src/mcp/`

- [ ] **Step 1: Copy directories**

```bash
cp -r crates/corpus/src/impl/tools crates/tools/src/
cp -r crates/corpus/src/impl/hooks crates/tools/src/
cp -r crates/corpus/src/impl/skills crates/tools/src/
cp -r crates/corpus/src/impl/mcp crates/tools/src/
```

- [ ] **Step 2: Create tools/lib.rs**

Create `crates/tools/src/lib.rs`:
```rust
//! Tool implementations, hooks, skills, and MCP client.

pub mod tools;
pub mod hooks;
pub mod skills;
pub mod mcp;
```

- [ ] **Step 3: Fix internal imports**

Replace `crate::impl::` with `crate::` in all moved files:
```bash
find crates/tools/src/ -name "*.rs" -exec sed -i 's/crate::impl::tools/crate::tools/g' {} +
find crates/tools/src/ -name "*.rs" -exec sed -i 's/crate::impl::hooks/crate::hooks/g' {} +
find crates/tools/src/ -name "*.rs" -exec sed -i 's/crate::impl::skills/crate::skills/g' {} +
find crates/tools/src/ -name "*.rs" -exec sed -i 's/crate::impl::mcp/crate::mcp/g' {} +
```

---

## Task 4: Move security and sandbox to security crate

**Files:**
- Move: `crates/corpus/src/impl/security/` → `crates/security/src/security/`
- Move: `crates/corpus/src/impl/sandbox/` → `crates/security/src/sandbox/`

- [ ] **Step 1: Copy directories**

```bash
cp -r crates/corpus/src/impl/security crates/security/src/
cp -r crates/corpus/src/impl/sandbox crates/security/src/
```

- [ ] **Step 2: Create security/lib.rs**

Create `crates/security/src/lib.rs`:
```rust
//! Security pipeline and sandbox execution.

pub mod security;
pub mod sandbox;
```

- [ ] **Step 3: Fix internal imports**

Replace `crate::impl::` with `crate::` in all moved files:
```bash
find crates/security/src/ -name "*.rs" -exec sed -i 's/crate::impl::security/crate::security/g' {} +
find crates/security/src/ -name "*.rs" -exec sed -i 's/crate::impl::sandbox/crate::sandbox/g' {} +
```

---

## Task 5: Move ui, cli, acix to interact crate

**Files:**
- Move: `crates/corpus/src/impl/ui/` → `crates/interact/src/ui/`
- Move: `crates/corpus/src/impl/cli/` → `crates/interact/src/cli/`
- Move: `crates/corpus/src/impl/acix/` → `crates/interact/src/acix/`

- [ ] **Step 1: Copy directories**

```bash
cp -r crates/corpus/src/impl/ui crates/interact/src/
cp -r crates/corpus/src/impl/cli crates/interact/src/
cp -r crates/corpus/src/impl/acix crates/interact/src/
```

- [ ] **Step 2: Create interact/lib.rs**

Create `crates/interact/src/lib.rs`:
```rust
//! User interaction: TUI, CLI, and ACIX.

pub mod ui;
pub mod cli;
pub mod acix;
```

- [ ] **Step 3: Fix internal imports**

Replace `crate::impl::` with `crate::` in all moved files:
```bash
find crates/interact/src/ -name "*.rs" -exec sed -i 's/crate::impl::ui/crate::ui/g' {} +
find crates/interact/src/ -name "*.rs" -exec sed -i 's/crate::impl::cli/crate::cli/g' {} +
find crates/interact/src/ -name "*.rs" -exec sed -i 's/crate::impl::acix/crate::acix/g' {} +
```

---

## Task 6: Slim down corpus to core only

**Files:**
- Modify: `crates/corpus/src/lib.rs`
- Delete: `crates/corpus/src/impl/` (after moving)

- [ ] **Step 1: Update corpus/lib.rs**

Replace `crates/corpus/src/lib.rs` with:
```rust
//! Core execution body — the minimal runtime for tool execution.

pub mod core;
pub mod bridge;
pub mod testing;

// Re-export main types
pub use core::AletheonBodyRuntime;
```

- [ ] **Step 2: Remove impl/ directory from corpus**

```bash
rm -rf crates/corpus/src/impl
rm -rf crates/corpus/src/bin
```

- [ ] **Step 3: Update corpus/Cargo.toml**

Remove all feature flags and heavy dependencies. Keep only:
```toml
[package]
name = "corpus"
version = "0.1.0"
edition = "2021"

[dependencies]
base = { path = "../base" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
```

---

## Task 7: Update workspace Cargo.toml

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add new crates to workspace members**

Add to workspace members:
```toml
"crates/drivers",
"crates/tools",
"crates/security",
"crates/interact",
```

- [ ] **Step 2: Verify workspace resolves**

```bash
cargo metadata --format-version 1 | head -5
```

---

## Task 8: Update dependent crates

**Files:**
- Modify: `crates/cognit/Cargo.toml`
- Modify: `crates/dasein/Cargo.toml`
- Modify: `crates/runtime/Cargo.toml`
- Modify: `crates/binaries/cli/Cargo.toml`
- Modify: `crates/binaries/exec/Cargo.toml`

- [ ] **Step 1: Update cognit dependencies**

In `crates/cognit/Cargo.toml`, change:
```toml
corpus = { path = "../corpus", features = ["input", "display", "a11y", "ocr"] }
```
to:
```toml
corpus = { path = "../corpus" }
drivers = { path = "../drivers", features = ["input", "display", "a11y", "ocr"] }
tools = { path = "../tools" }
security = { path = "../security" }
```

- [ ] **Step 2: Update dasein dependencies**

In `crates/dasein/Cargo.toml`, change:
```toml
corpus = { path = "../corpus" }
```
to:
```toml
corpus = { path = "../corpus" }
security = { path = "../security" }
```

- [ ] **Step 3: Update runtime dependencies**

In `crates/runtime/Cargo.toml`, change:
```toml
corpus = { path = "../corpus" }
```
to:
```toml
corpus = { path = "../corpus" }
tools = { path = "../tools" }
security = { path = "../security" }
```

- [ ] **Step 4: Update cli binary dependencies**

In `crates/binaries/cli/Cargo.toml`, change:
```toml
corpus = { path = "../../corpus", features = ["cli", "input", "display", "a11y", "ocr"] }
```
to:
```toml
corpus = { path = "../../corpus" }
drivers = { path = "../../drivers", features = ["input", "display", "a11y", "ocr"] }
interact = { path = "../../interact" }
```

- [ ] **Step 5: Update exec binary dependencies**

In `crates/binaries/exec/Cargo.toml`, change:
```toml
corpus = { path = "../../corpus" }
```
to:
```toml
corpus = { path = "../../corpus" }
tools = { path = "../../tools" }
security = { path = "../../security" }
```

---

## Task 9: Update use statements in dependent crates

**Files:**
- Modify: All files that import from corpus::impl::

- [ ] **Step 1: Find all corpus::impl:: references**

```bash
grep -rn "use corpus::impl::" crates/ examples/ --include="*.rs"
```

- [ ] **Step 2: Update imports**

Replace:
- `use corpus::impl::tools::` → `use tools::tools::`
- `use corpus::impl::driver::` → `use drivers::driver::`
- `use corpus::impl::security::` → `use security::security::`
- `use corpus::impl::sandbox::` → `use security::sandbox::`
- `use corpus::impl::ui::` → `use interact::ui::`
- `use corpus::impl::mcp::` → `use tools::mcp::`
- `use corpus::impl::hooks::` → `use tools::hooks::`
- `use corpus::impl::skills::` → `use tools::skills::`

- [ ] **Step 3: Verify no remaining corpus::impl:: references**

```bash
grep -rn "use corpus::impl::" crates/ examples/ --include="*.rs"
```

Expected: No output.

---

## Task 10: Verify compilation and tests

- [ ] **Step 1: Run cargo check**

```bash
cargo check --workspace
```

Expected: All crates compile without errors.

- [ ] **Step 2: Run cargo test**

```bash
cargo test --workspace
```

Expected: All tests pass.

- [ ] **Step 3: Verify crate structure**

```bash
ls -la crates/
```

Expected: `base/ corpus/ drivers/ tools/ security/ interact/ memory/ dasein/ runtime/ metacog/ binaries/`

---

## Task 11: Commit changes

- [ ] **Step 1: Stage all changes**

```bash
git add -A
```

- [ ] **Step 2: Commit**

```bash
git commit -m "refactor: split corpus into 5 focused crates

- corpus: core execution body (slimmed down)
- drivers: hardware drivers and platform adapters
- tools: tool implementations, hooks, skills, MCP
- security: security pipeline and sandbox execution
- interact: TUI, CLI, and ACIX

Each crate has single responsibility and clear boundaries."
```

---

## Self-Review Checklist

1. **Spec coverage:** This plan covers Phase 3 of the architectural redesign spec (§8.3)
2. **Placeholder scan:** No TBD/TODO — all steps are concrete
3. **Type consistency:** Import rewrites are consistent throughout
4. **Verification:** Each task has explicit verification steps
5. **Risk mitigation:** sandbox stays with security to avoid coupling issues

---

## Execution Options

Plan complete and saved to `docs/plans/2026-06-21-phase3-split-corpus.md`.

Execution options:
1. **workflow-feature** — Multi-agent pipeline with approval gates
2. **Inline execution** — Execute tasks in this session with checkpoints

Which approach?
