# Runtime Context And Socket Access Fix Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent transient prompt data from growing persisted sessions and make secure socket group activation explicit after installation.

**Architecture:** Assemble each request from one bounded system prefix, raw persisted history, and ephemeral bounded injections. Preserve socket mode `0660` while detecting and explaining stale login group credentials.

**Tech Stack:** Rust, Tokio, SQLite-backed session journal, Bash installer, Cargo tests.

---

### Task 1: Bound the system prefix and transient recall

**Files:**
- Modify: `crates/runtime/src/impl/daemon/prefix_builder.rs`
- Modify: `crates/runtime/src/impl/daemon/handler/chat.rs`

- [x] Add tests proving prefixes contain skill descriptions but not full bodies.
- [x] Add UTF-8-safe per-item and total character budgeting for activated skills and recalled facts.
- [ ] Run focused runtime tests.

### Task 2: Separate raw history from ephemeral prompt context

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/chat.rs`
- Modify: `crates/runtime/src/impl/daemon/session_manager.rs`

- [ ] Add tests proving persisted user history contains raw input only.
- [x] Build the ReAct seed from one system prefix, recent raw history, and one current enriched user message.
- [x] Do not persist failed provider responses or feed them into AutoMemory.
- [x] Add recovery coverage ensuring transient injection cannot return after restart.

### Task 3: Explain inactive socket group membership

**Files:**
- Modify: `setup.sh`
- Modify: `README.md`
- Modify: `docs/design/runtime/daemon.md`

- [x] Keep socket mode `0660` and add an installer group-activation check.
- [x] Print re-login, `newgrp`, and temporary `sg` instructions when required.
- [x] Document verification with `id -nG`.

### Task 4: Verify

**Files:**
- Verify: all modified Rust, shell, and Markdown files

- [ ] Run focused tests for prefix, session persistence, and socket authorization.
- [ ] Run formatting and workspace checks where the local toolchain permits.
- [x] Run Markdown link and shell syntax checks.
- [x] Record any environment-only limitation separately from code failures.

