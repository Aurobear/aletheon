# Rust Toolchain Compatibility Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish Rust 1.85 as Aletheon's reproducible MSRV while continuously verifying compatibility with current stable Rust.

**Architecture:** Repository metadata declares one MSRV and a root toolchain file makes local builds deterministic. CI separates the compatibility contract (1.85) from current-stable quality checks so Arch Linux users can use a rolling compiler without silently raising the project minimum.

**Tech Stack:** Cargo workspace metadata, rustup toolchain files, GitHub Actions, Markdown documentation

---

### Task 1: Declare and pin the MSRV

**Files:**
- Create: `rust-toolchain.toml`
- Modify: `Cargo.toml:16-20`
- Modify: `crates/base/Cargo.toml:1-5`
- Modify: `crates/cognit/Cargo.toml:1-5`
- Modify: `crates/corpus/Cargo.toml:1-5`
- Modify: `crates/dasein/Cargo.toml:1-5`
- Modify: `crates/interact/Cargo.toml:1-5`
- Modify: `crates/memory/Cargo.toml:1-5`
- Modify: `crates/metacog/Cargo.toml:1-5`
- Modify: `crates/runtime/Cargo.toml:1-5`
- Modify: `examples/basic-agent/Cargo.toml:1-5`
- Modify: `examples/self-evolution-loop/Cargo.toml:1-5`

- [ ] **Step 1: Add the pinned toolchain**

```toml
[toolchain]
channel = "1.85.0"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

- [ ] **Step 2: Declare workspace MSRV inheritance**

Add this under `[workspace.package]`:

```toml
rust-version = "1.85"
```

Add this to every workspace member's `[package]` table, replacing explicit edition-only metadata where needed:

```toml
rust-version.workspace = true
```

- [ ] **Step 3: Verify Cargo metadata**

Run: `cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; d=json.load(sys.stdin); assert all(p["rust_version"] == "1.85" for p in d["packages"]); print("MSRV metadata: PASS")'`

Expected: `MSRV metadata: PASS`

### Task 2: Add dual-toolchain CI coverage

**Files:**
- Modify: `.github/workflows/ci.yml:12-66`
- Modify: `.github/workflows/release.yml:24-35`

- [ ] **Step 1: Add an MSRV job to CI**

Add a job that installs `1.85.0` and runs:

```yaml
  msrv:
    name: Rust 1.85 compatibility
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.85.0
      - uses: Swatinem/rust-cache@v2
        with:
          key: msrv-1.85
      - run: cargo +1.85.0 check --workspace --all-targets
      - run: cargo +1.85.0 test --workspace --all-targets
```

- [ ] **Step 2: Make current-stable jobs explicit**

Keep check, test, clippy, fmt, docs, and build on `dtolnay/rust-toolchain@stable`, but run Cargo as `cargo +stable ...`. Update release builds likewise so the root pin cannot accidentally turn stable verification into MSRV verification.

- [ ] **Step 3: Validate workflow syntax structurally**

Run: `python3 -c 'from pathlib import Path; s=Path(".github/workflows/ci.yml").read_text(); assert "toolchain: 1.85.0" in s; assert "cargo +1.85.0 check --workspace --all-targets" in s; assert "cargo +stable clippy --workspace -- -D warnings" in s; print("CI toolchains: PASS")'`

Expected: `CI toolchains: PASS`

### Task 3: Publish the compatibility contract

**Files:**
- Modify: `docs/guide/getting-started.md:8-12`
- Modify: `README.md:398-413`

- [ ] **Step 1: Replace the unverified version statement**

Use this requirement in the getting-started guide:

```markdown
- Rust 1.85 or newer. The repository pins Rust 1.85 for reproducible local builds; newer stable toolchains, including Arch Linux's rolling Rust package, are verified separately in CI.
```

- [ ] **Step 2: Add the same concise MSRV note to the README technology/tooling section**

Document `rustup show`, `cargo +1.85.0 check --workspace`, and `cargo +stable check --workspace` as the three diagnostic commands.

- [ ] **Step 3: Verify no active setup guide advertises Rust 1.75**

Run: `! rg -n 'Rust toolchain 1\.75|Rust 1\.75' README.md docs/guide`

Expected: exit code 0 and no output.

### Task 4: Run compatibility validation

**Files:**
- Modify only if required by an actual Rust 1.85 compiler error: the smallest affected manifest or source file

- [ ] **Step 1: Confirm both toolchains are installed**

Run: `rustup toolchain list`

Expected: entries for `1.85.0` and `stable`. If 1.85 is unavailable locally, report the network/install constraint rather than changing the MSRV.

- [ ] **Step 2: Validate MSRV**

Run: `cargo +1.85.0 check --workspace --all-targets`

Expected: exit code 0.

Run: `cargo +1.85.0 test --workspace --all-targets`

Expected: exit code 0 outside restricted sandboxes; Unix-socket permission failures must be reported separately.

- [ ] **Step 3: Validate current stable**

Run: `cargo +stable check --workspace --all-targets`

Expected: exit code 0.

Run: `cargo +stable test --workspace --all-targets`

Expected: exit code 0 outside restricted sandboxes; Unix-socket permission failures must be reported separately.

- [ ] **Step 4: Validate formatting and repository diff**

Run: `cargo +stable fmt --all -- --check && git diff --check`

Expected: exit code 0.
