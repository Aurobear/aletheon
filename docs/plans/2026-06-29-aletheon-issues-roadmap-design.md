# Aletheon Issues Roadmap Design

**Date:** 2026-06-29
**Status:** Draft
**Scope:** Comprehensive issue remediation for the aletheon-* crate workspace

---

## 1. Background

Two audits were conducted on 2026-06-22 against the original crate set (base, cognit, corpus, dasein, memory, metacog, runtime), identifying 26 issues (3 P0, 7 P1, 7 P2, 9 P3).

On 2026-06-27, a major namespace refactoring created 8 new `aletheon-*` crates. The old crates are **no longer workspace members** — all binaries and examples depend exclusively on the new crates. The old crate directories remain on disk but are functionally dead.

This document re-evaluates all 26 issues against the active `aletheon-*` codebase and identifies new issues introduced by the refactoring.

---

## 2. Verification Summary

### 2.1 Resolved Issues (4 of 26)

| # | Issue | Resolution |
|---|-------|-----------|
| P0-1 | max_tool_calls=0 breaks tool use | Concept removed; `max_iterations` used instead |
| P0-2 | cognit tests fail to compile | Tests moved to inline `#[cfg(test)]` across 20 files |
| P2-15 | Hand-rolled TOML parser | Now uses `toml` crate |
| P3-20 | Controller dead code | File and struct removed entirely |

### 2.2 Downgraded Issues (2 of 26)

| # | Issue | New Status | Reason |
|---|-------|-----------|--------|
| P0-3 | 4x unimplemented!() in prod | Low (test-only) | Both remaining occurrences are in `#[cfg(test)]` mock code |
| P3-21 | Dasein perception dead code | Working implementations | All 4 sources (proc, inotify, journald, ebpf) have real functionality |

### 2.3 Changed Issues (3 of 26)

| # | Issue | Change |
|---|-------|--------|
| P1-4 | Exec mode missing subsystems | By design — exec is intentionally a lightweight CI/CD tool |
| P1-10 | ToolBudget vs reflection conflict | ToolBudget doesn't exist; reflection limit is a different concern |
| P2-11 | TUI 34-field god object | Reduced to 19 fields, all UI-related |

### 2.4 Still Present (17 of 26)

| Severity | # | Issue | Location in new crates |
|----------|---|-------|----------------------|
| **P0** | ★ | L1+ tools treated as bash commands | `aletheon-body/src/impl/security/runner.rs:130-163` |
| **P1** | 5 | Socket path requires root | `aletheon-runtime/src/core/config.rs:219`, `aletheon-brain/src/config/mod.rs:193`, `aletheon-abi/src/paths.rs:11` |
| **P1** | 6 | lock().unwrap() cascade (73 sites) | Across all crates, worst in aletheon-self (safety) and aletheon-memory (DB) |
| **P1** | 7 | Audit events silently dropped | `aletheon-body/src/impl/security/audit.rs:78`, `aletheon-self/.../audit.rs:78` |
| **P1** | 8 | API key silent empty string | `aletheon-brain/src/impl/llm/provider_factory.rs:105-113`, `provider_registry.rs:139-145` |
| **P1** | 9 | Anthropic URL misidentified | `aletheon-brain/src/impl/llm/provider_factory.rs:17-26`, `provider_registry.rs:20-27` |
| **P2** | 12 | Two parallel hook systems | `aletheon-runtime/src/impl/hooks/registry.rs` + `aletheon-self/src/impl/hook/dispatcher.rs` |
| **P2** | 13 | SandboxFirst verdict ignored | `aletheon-runtime/src/impl/daemon/handler.rs:293-319` |
| **P2** | 14 | Dual compaction | Engine AdvancedCompressor + SessionManager::compact_if_needed |
| **P2** | 16 | Hardcoded MCP session ID | `aletheon-runtime/src/impl/daemon/mcp_embedded.rs:162` |
| **P3** | 18 | Warning suppressions | 21x `#[allow(dead_code)]` across new crates |
| **P3** | 19 | LanceDB vector store stubbed | `aletheon-runtime/src/impl/memory/vector_store.rs:239-261` |

---

## 3. New Issues from Refactoring

### 3.1 Old crate directories still on disk

The 7 old crate directories (base, cognit, corpus, dasein, memory, metacog, runtime, interact) remain under `crates/`. They are not workspace members but could confuse contributors and search tools.

**Recommendation:** Delete or move to `crates/_archive/`.

### 3.2 Dasein phenomenological module removed

The `dasein/` subdirectory (11 files: bewandtnis.rs, sorge.rs, temporality.rs, negativity.rs, care_structure.rs, etc.) was dropped from `aletheon-self`. This is a significant philosophical simplification.

**Decision needed:** Was this intentional? If so, update architecture docs to reflect the change.

### 3.3 Kernel subsystem removed

`aletheon-runtime` does not include the kernel subsystem (`impl/kernel/`) that existed in the old runtime crate.

**Decision needed:** Was this intentional? The roadmap mentions eBPF/FUSE as completed Phase 5 work.

---

## 4. Remediation Plan

### Phase 0: Architectural Decisions (1 day)

**Goal:** Resolve open questions before any code changes.

| Decision | Options | Recommendation |
|----------|---------|---------------|
| Old crate directories | Delete / Archive / Keep | Archive to `crates/_archive/` |
| Dasein phenomenology | Restore / Document removal / Ignore | Document as intentional simplification |
| Kernel subsystem | Restore / Feature-gate / Remove | Feature-gate behind `kernel` feature flag |

**Deliverable:** Decision record in `docs/development/2026-06-29-arch-decisions.md`.

---

### Phase 1: Blocking Fixes (2-3 days)

**Goal:** Make the basic agent loop functional.

#### 1.1 Rewrite L1+ tool execution in security runner

**Problem:** `runner.rs:130-163` extracts `input["command"]` for ALL L1+ tools, breaking file_write, apply_patch, and any non-bash tool.

**Design:**

```
Tool execution dispatch:
  L0 tools          → tool.execute(input, ctx)
  L1 bash_exec      → sandbox.run(input["command"])
  L1 file_write     → tool.execute(input, ctx) + path boundary check
  L1 apply_patch    → tool.execute(input, ctx) + path boundary check
  L2+ tools         → approval gate → tool.execute(input, ctx) + sandbox profile
```

The runner should:
1. Check if the tool is `bash_exec` → route to sandbox with `input["command"]`
2. For all other L1+ tools → call `tool.execute(input, ctx)` with path policy enforcement
3. Path policy: canonicalize paths, check against workspace root and writable boundaries
4. The sandbox is an optional overlay for bash_exec, not a universal execution backend

**Files to modify:**
- `crates/aletheon-body/src/impl/security/runner.rs` — rewrite execute_tool dispatch
- `crates/aletheon-body/src/impl/tools/file_write.rs` — add path boundary validation in execute()
- `crates/aletheon-body/src/impl/tools/apply_patch.rs` — add path boundary validation

**Validation:** Add integration tests that exercise file_write and apply_patch through the security runner.

#### 1.2 Socket path: use XDG/user-space default

**Problem:** Default socket at `/run/aletheon/` requires root.

**Design:**
- Default to `$XDG_RUNTIME_DIR/aletheon/aletheon.sock` if set
- Fall back to `~/.aletheon/aletheon.sock`
- Keep `/run/aletheon/` only for systemd service unit (read from config)

**Files to modify:**
- `crates/aletheon-abi/src/paths.rs` — change SOCKET_DIR to user-space
- `crates/aletheon-runtime/src/core/config.rs:219` — change default_daemon_socket_path()
- `crates/aletheon-brain/src/config/mod.rs:193` — change default_daemon_socket_path()

#### 1.3 API key fail fast

**Problem:** Missing API key returns `""`, error surfaces only on first API call as cryptic 401.

**Design:**
- For providers that require a key (OpenAI, Anthropic): return `Result<String>`, fail at provider creation
- For local providers (Ollama): allow empty key
- Log a clear warning: `"API key not found for provider '{name}'. Set {NAME}_API_KEY or add api_key to config."`

**Files to modify:**
- `crates/aletheon-brain/src/impl/llm/provider_factory.rs:105-113`
- `crates/aletheon-brain/src/impl/provider_registry.rs:139-145`

#### 1.4 Anthropic URL detection

**Problem:** `ends_with("/anthropic")` doesn't match `https://api.anthropic.com`.

**Design:**
```rust
fn detect_provider_kind(base_url: &str) -> &str {
    let normalized = base_url.trim().to_lowercase();
    if normalized.contains("anthropic.com") || normalized.ends_with("/anthropic") {
        "anthropic"
    } else if normalized.contains("localhost:11434") || normalized.contains("127.0.0.1:11434") {
        "ollama"
    } else {
        "openai"
    }
}
```

**Files to modify:**
- `crates/aletheon-brain/src/impl/llm/provider_factory.rs:17-26`
- `crates/aletheon-brain/src/impl/provider_registry.rs:20-27`

---

### Phase 2: Safety Hardening (3-5 days)

**Goal:** Eliminate cascade-panic risks and close security gaps.

#### 2.1 Poison-safe mutex handling

**Problem:** 73 `.lock().unwrap()` calls. A single panic in one subsystem cascades through the runtime.

**Design:** Three-tier strategy:
1. **Safety-critical paths** (aletheon-self: killswitch, integrity_monitor, resource_governor): Use `.lock().unwrap_or_else(|e| { warn!("mutex poisoned: {}", e); e.into_inner() })` with recovery logging
2. **Data paths** (aletheon-memory: DB connections): Same pattern, plus metrics counter
3. **Non-critical paths** (aletheon-meta, aletheon-body): `.lock().unwrap_or_else(|e| e.into_inner())` silently

Priority order: aletheon-self (16 sites) → aletheon-memory (12) → aletheon-meta (20) → aletheon-brain (3) → aletheon-abi (3) → aletheon-body (15+3)

**Files:** All crates, 73 sites total.

#### 2.2 Audit event channel: log drops

**Problem:** `try_send` silently drops audit records when channel is full.

**Design:**
```rust
fn log_sync(&self, record: AuditRecord) {
    if self.tx.try_send(record).is_err() {
        // Use a separate counter to avoid recursion
        self.dropped_count.fetch_add(1, Ordering::Relaxed);
        eprintln!("[audit] channel full, record dropped (total dropped: {})",
                  self.dropped_count.load(Ordering::Relaxed));
    }
}
```

**Files:**
- `crates/aletheon-body/src/impl/security/audit.rs:78`
- `crates/aletheon-self/src/impl/security/audit.rs:78`

#### 2.3 Unify hook systems

**Problem:** Two separate hook execution paths (HookRegistry in runtime, HookDispatcher in aletheon-self) with different configs, contexts, and semantics.

**Design:** Consolidate into a single `HookEngine` in aletheon-runtime:
1. Keep HookDispatcher's TOML config loading (it's cleaner)
2. Keep HookRegistry's JSON stdin/stdout protocol
3. Merge into one pipeline: config load → priority sort → sequential execute with block/inject semantics
4. Both daemon handler and cognitive loop call the same engine

**Files:**
- `crates/aletheon-runtime/src/impl/hooks/` — new unified HookEngine
- `crates/aletheon-self/src/impl/hook/` — move dispatcher logic to runtime
- `crates/aletheon-runtime/src/impl/daemon/handler.rs` — update hook calls
- `crates/aletheon-runtime/src/core/cognitive_loop.rs` — update hook calls

**Note:** This is the largest change in Phase 2. May warrant its own sub-design.

#### 2.4 SandboxFirst: enforce or remove

**Problem:** SandboxFirst verdict is detected and logged but not enforced — injected as a text note only.

**Design (option A — enforce):**
- When SelfField returns SandboxFirst, route the tool call through the sandbox regardless of tool type
- Requires the Phase 1.1 tool execution rewrite to be complete

**Design (option B — remove):**
- Remove SandboxFirst from the Verdict enum
- Remove the text injection code
- If sandbox enforcement is needed, implement it at the tool runner level, not the verdict level

**Recommendation:** Option B — remove SandboxFirst verdict. Sandboxing should be determined by tool permission level, not a runtime verdict.

**Files:**
- `crates/aletheon-self/src/core/mod.rs` — remove SandboxFirst from Verdict
- `crates/aletheon-runtime/src/impl/daemon/handler.rs:293-319` — remove injection code

#### 2.5 MCP session ID: generate UUID

**Problem:** All MCP tool calls share `"mcp-session"`.

**Design:** Generate a UUID per MCP connection:
```rust
session_id: uuid::Uuid::new_v4().to_string(),
```

**Files:**
- `crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs:162`

---

### Phase 3: Quality Cleanup (2-3 days)

**Goal:** Reduce technical debt and improve maintainability.

#### 3.1 Clean up old crate directories

Move `crates/{base,cognit,corpus,dasein,interact,memory,metacog,runtime}` to `crates/_archive/` or delete entirely.

#### 3.2 Remove dead code suppressions

Audit 21 `#[allow(dead_code)]` annotations:
- If the field is genuinely unused → remove it
- If it's needed for future use → add a TODO with issue reference
- If it's a public API surface → keep but document

#### 3.3 LanceDB: implement or remove from Auto

Options:
- Implement LanceDB backend (significant effort, ~1 day)
- Remove LanceDB from Auto selection, keep Qdrant as default
- Remove LanceDB entirely if not planned

**Recommendation:** Remove from Auto selection. Keep the stub with a clear "not implemented" message for users who explicitly configure it.

#### 3.4 Merge dual compaction

Consolidate AdvancedCompressor (Engine) and SessionManager::compact_if_needed into a single compaction point. The Engine's compressor should be the authoritative one; SessionManager should delegate to it.

#### 3.5 Release workflow fix

Update `.github/workflows/release.yml` to use correct Cargo package names.

---

## 5. Dependency Graph

```
Phase 0 (decisions)
  │
  ├──→ Phase 1.1 (L1+ tool execution) ──→ Phase 2.4 (SandboxFirst)
  │
  ├──→ Phase 1.2 (socket path)
  ├──→ Phase 1.3 (API key fail fast)
  ├──→ Phase 1.4 (Anthropic URL)
  │
  └──→ Phase 2.1 (mutex safety) ──→ independent
       Phase 2.2 (audit drops) ──→ independent
       Phase 2.3 (hook unification) ──→ independent
       Phase 2.5 (MCP session) ──→ independent
       │
       └──→ Phase 3 (cleanup) ──→ all Phase 2 done
```

Phase 1 items are independent of each other and can be parallelized.
Phase 2 items are independent of each other (except 2.4 depends on 1.1).
Phase 3 depends on Phase 2 completion.

---

## 6. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| L1+ tool rewrite breaks bash_exec | Medium | High | Keep bash_exec sandbox path unchanged; only change non-bash dispatch |
| Hook unification breaks existing hooks | Medium | Medium | Integration tests with sample hook scripts |
| Mutex poison recovery masks real bugs | Low | Medium | Log all poison events; add metrics |
| Old crate deletion loses reference code | Low | Low | Archive first, delete after Phase 3 verified |

---

## 7. Success Criteria

- [ ] `cargo test --workspace` passes (currently blocked by P0-★)
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `file_write` and `apply_patch` work through the security runner
- [ ] Daemon starts without root privileges
- [ ] Missing API key produces clear error at startup, not cryptic 401
- [ ] Single thread panic does not cascade to other subsystems
- [ ] All audit events are logged (none silently dropped)
- [ ] Release workflow builds successfully

---

## 8. Open Questions

1. **Dasein phenomenology removal:** Intentional simplification or oversight?
2. **Kernel subsystem removal:** Planned for later phases or abandoned?
3. **Hook unification scope:** Should this be a separate design effort?
4. **LanceDB investment:** Worth implementing or should we commit to Qdrant-only?
5. **Old crate cleanup timing:** Before or after Phase 1?
