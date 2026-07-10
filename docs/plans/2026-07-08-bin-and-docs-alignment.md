# Binary Entry And Documentation Alignment Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Rename the executable assembly crate and align design documentation paths and content with the real workspace.

**Architecture:** Keep one installed `aletheon` executable in a dedicated `crates/bin` assembly package. Keep implementation in the eight domain crates and organize design documentation under those crate names.

**Tech Stack:** Rust Cargo workspace, Markdown, Python link validation.

---

### Task 1: Rename executable assembly package

**Files:**
- Move: `crates/bin/` -> `crates/bin/`
- Modify: `crates/bin/Cargo.toml`
- Modify: `Cargo.toml`

- [x] Move the package directory and change its package name to `aletheon-bin` while retaining `[[bin]] name = "aletheon"`.
- [x] Change the workspace member from `crates/aletheon` to `crates/bin`.
- [x] Run `cargo metadata --no-deps --format-version 1` and verify the package path and executable name.

### Task 2: Rename design directories

**Files:**
- Move: `docs/design/abi/` -> `docs/design/base/`
- Move: `docs/design/body/` -> `docs/design/corpus/`
- Move: `docs/design/brain/` -> `docs/design/cognit/`
- Move: `docs/design/self/` -> `docs/design/dasein/`
- Move: `docs/design/meta/` -> `docs/design/metacog/`
- Move: `docs/design/cli/` -> `docs/design/interact/`

- [x] Move each directory without altering unrelated document content.
- [x] Replace repository-relative references to the old directory paths.

### Task 3: Align entry documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/design/README.md`
- Modify: affected files under `docs/design/`

- [x] Replace deleted quick-start and architecture links with existing design entry points.
- [x] Document eight domain crates plus the non-domain `bin` assembly package.
- [x] Replace obsolete standalone binary and systemd service descriptions with the unified command and actual `config/aletheon*.service` paths.
- [x] Remove stale fixed test counts and references to nonexistent `Aletheon.md`.

### Task 4: Validate documentation and workspace

**Files:**
- Verify: `README.md`
- Verify: `docs/**/*.md`
- Verify: `Cargo.toml`

- [x] Run a relative Markdown link checker and require zero missing targets.
- [x] Search for obsolete crate and documentation paths and require zero active references outside historical plan text.
- [x] Run Cargo metadata and formatting checks; report any sandbox/toolchain limitation explicitly.
- [x] Confirm the original 14 deletions remain present in `git status`.

