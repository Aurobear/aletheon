# Phase 4 Implementation Plan: MCP + Session + Hooks + Skills

> **For agentic workers:** task-by-task with `- [ ]` steps. Structured for **multiple developer agents in parallel** — see Dependency Graph and Parallel Batches.

**Goal:** Connect the already-built-but-orphaned subsystems — MCP tools into the live registry, the remaining lifecycle hooks, session resume/new endpoints, and keyword-triggered skills — so the runtime gains external tools, full observability, and persistent/recoverable sessions.

**Architecture:** The survey found these subsystems are **implemented but not wired**: the MCP client/wrapper works but `connect_all()` is never called (MCP tools are dead code); 6 of 8 hook points are never fired; skills load and inject statically but keyword matching is unused; `SessionStore` persists and `resume` exists but there's no "load most recent"/"new" control. Phase 4 is almost entirely **wiring + small additions**, not new subsystems.

**Tech Stack:** Rust, tokio, rusqlite, serde_json, MCP over stdio.

**Design spec:** [2026-06-19-cli-agent-design.md](./2026-06-19-cli-agent-design.md) §8–§9.

**Verified current state (read-only survey, 2026-06-19):**
- **MCP (dead code):** `McpClient::connect_stdio/call_tool` (`body/impl/mcp/client.rs:31,104`), `McpConnectionManager::connect_all` (`client.rs:138`), `McpToolWrapper: Tool` (`mcp/wrapper.rs:12`) all work. **Zero callers of `connect_all`** anywhere; MCP tools never enter the `ToolRegistry` the ReAct loop sees (daemon registers only built-ins + skill tools, `handler.rs:133-230`).
- **Hooks (2/8 fired):** `HookPoint` has 8 variants (`abi/src/hook.rs:12`): OnSessionStart, OnSessionEnd, PreTurn, PostTurn, PreTool, PostTool, OnMemoryStore, OnMemoryRecall. Daemon fires only **PreTurn** (`handler.rs:482`) and **PostTurn** (`handler.rs:599`). `HookRegistry` execution engine works (`hooks/registry.rs:70`).
- **Session:** `SessionStore` (SQLite, `impl/session/store.rs:6`): `create_session`, `list_sessions` (DESC by last_active). `SessionManager` (`session_manager.rs`): in-memory messages, journaling, `recover()`, auto-compact. `resume` RPC exists (`handler.rs:979`). **Missing:** "load most recent" + "new session" RPC controls.
- **Skills:** loader (legacy `.md` + dir `SKILL.md`), manifest YAML frontmatter (`manifest.rs:60`), tool/hook registration (`plugin.rs:205`), static injection (`inject.rs:6`). Manifest has `keywords: Vec<String>` (`manifest.rs:18`) but **no code matches them**. Known bug: `~/.aletheon/skills/hello` lacks `---` frontmatter → warns each boot.

---

## File Structure & Owner Boundaries

| Agent | Owns (writes) | Responsibility |
|-------|---------------|----------------|
| **A — MCP wiring** | `crates/aletheon-body/src/impl/mcp/manager.rs` (NEW), `crates/aletheon-body/src/impl/mcp/mod.rs` | `McpManager` facade wrapping `McpConnectionManager`; expose `connect_all` + `tool_wrappers() -> Vec<Box<dyn Tool>>` for the daemon to register. |
| **B — Daemon integration** | `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Call MCP `connect_all` at boot + inject MCP tools into the registry; fire the 6 missing hook points; add `new_session`/`load_recent` RPC methods. |
| **C — Skills keyword matching** | `crates/aletheon-runtime/src/impl/skills/keyword_matcher.rs` (NEW), `crates/aletheon-runtime/src/impl/skills/mod.rs`, fix bundled `~/.aletheon/skills/hello/SKILL.md` | Match user message against skill keywords; expose activated skill bodies for injection. (Daemon calls it — B wires the call.) |
| **D — Session control** | `crates/aletheon-runtime/src/impl/session/store.rs`, `crates/aletheon-runtime/src/impl/session/mod.rs` | Add `most_recent()` query + a `new_session` helper to `SessionStore`. (B exposes them as RPC.) |

**Shared file:** `handler.rs` is owned by **B only**. A/C/D provide the building blocks; B
calls them. This keeps `handler.rs` single-writer (it's the highest-contention file).

---

## Dependency Graph

```
Batch 1 (parallel, independent building blocks):
  Agent A: A1 (McpManager facade)
  Agent C: C1 (keyword_matcher) + C2 (fix bundled skill frontmatter)
  Agent D: D1 (SessionStore most_recent + new_session)

Batch 2 (after A1 + C1 + D1 — all consumed by handler.rs):
  Agent B: B1 (MCP inject) → B2 (fire 6 hooks) → B3 (session RPC + keyword inject)

Batch 3:
  E1 (build + tests) → E2 (acceptance: MCP tool callable; hooks fire; resume works; skill triggers)
```

A, C, D are independent and run together in Batch 1. B integrates all three into
`handler.rs` in Batch 2 (single writer, sequential sub-tasks).

---

## Parallel Batches

- **Batch 1:** A (A1) ‖ C (C1, C2) ‖ D (D1).
- **Batch 2:** B (B1→B2→B3) — depends on all of Batch 1.
- **Batch 3:** integration agent.

Branch: `auro/feat/20260622-phase4-mcp-session-skills`. Commit per task.

---

## Agent A — MCP Manager

### Task A1: McpManager facade

**Files:** Create `crates/aletheon-body/src/impl/mcp/manager.rs`; modify `mcp/mod.rs`

- [ ] **Step 1: Write the failing test** — construct an `McpManager` from an empty config
  and assert `tool_wrappers()` is empty and `connect_all()` on empty config is Ok:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn empty_config_yields_no_tools() {
        let mut m = McpManager::new(Default::default());
        m.connect_all().await.unwrap();
        assert!(m.tool_wrappers().is_empty());
    }
}
```

- [ ] **Step 2: Implement** `manager.rs` — a thin facade over the existing
  `McpConnectionManager` (`client.rs:123`) and `McpToolWrapper` (`wrapper.rs`):

```rust
//! High-level MCP facade for the daemon: connect configured servers and expose
//! their tools as `Box<dyn Tool>` ready to register into the ToolRegistry.

use anyhow::Result;
use super::client::McpConnectionManager;
use super::config::McpConfig;
use aletheon_abi::tool::Tool;

pub struct McpManager {
    inner: McpConnectionManager,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self { inner: McpConnectionManager::new(config) }
    }

    /// Connect all configured servers (no-op for empty config). Errors from
    /// individual servers are logged inside connect_all and do not abort the rest.
    pub async fn connect_all(&mut self) -> Result<()> {
        self.inner.connect_all().await
    }

    /// All discovered MCP tools, wrapped as `Tool` trait objects for registration.
    pub fn tool_wrappers(&self) -> Vec<Box<dyn Tool>> {
        self.inner.get_all_tools()
    }
}
```

> Verify the real names: `McpConnectionManager::new(...)` constructor args, `connect_all`
> return type, and that `get_all_tools()` returns `Vec<Box<dyn Tool>>` (survey says
> `client.rs:177-208` produces wrappers). Adapt to the actual signatures — do not invent.

- [ ] **Step 3: Re-export** in `mcp/mod.rs`: `pub mod manager; pub use manager::McpManager;`
- [ ] **Step 4: Test** `cargo test -p aletheon-body mcp::manager -- --nocapture` → PASS.
- [ ] **Step 5: Commit** `git commit -am "feat(mcp): McpManager facade (connect_all + tool_wrappers)"`

---

## Agent C — Skills Keyword Matching

### Task C1: keyword_matcher

**Files:** Create `crates/aletheon-runtime/src/impl/skills/keyword_matcher.rs`; modify `skills/mod.rs`

- [ ] **Step 1: Write the failing test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_skill_by_keyword() {
        let skills = vec![
            SkillKeywords { name: "git".into(), keywords: vec!["commit".into(), "branch".into()], body: "GIT BODY".into() },
            SkillKeywords { name: "docker".into(), keywords: vec!["container".into()], body: "DOCKER BODY".into() },
        ];
        let hits = match_skills("please commit my branch", &skills);
        assert_eq!(hits, vec!["GIT BODY".to_string()]);
    }
    #[test]
    fn no_keyword_no_match() {
        let skills = vec![SkillKeywords { name: "git".into(), keywords: vec!["commit".into()], body: "B".into() }];
        assert!(match_skills("hello world", &skills).is_empty());
    }
}
```

- [ ] **Step 2: Implement** `keyword_matcher.rs`:

```rust
//! Keyword-triggered skill activation. Matches a user message (case-insensitive,
//! word-ish substring) against each skill's declared keywords and returns the
//! bodies of activated skills for injection into the turn.

/// A skill's matchable surface.
#[derive(Debug, Clone)]
pub struct SkillKeywords {
    pub name: String,
    pub keywords: Vec<String>,
    pub body: String,
}

/// Return the bodies of all skills whose any keyword appears in `message`.
pub fn match_skills(message: &str, skills: &[SkillKeywords]) -> Vec<String> {
    let lower = message.to_lowercase();
    skills.iter()
        .filter(|s| s.keywords.iter().any(|k| !k.is_empty() && lower.contains(&k.to_lowercase())))
        .map(|s| s.body.clone())
        .collect()
}
```

- [ ] **Step 3: Re-export** in `skills/mod.rs`: `pub mod keyword_matcher; pub use keyword_matcher::{SkillKeywords, match_skills};`
- [ ] **Step 4: Test** `cargo test -p aletheon-runtime keyword_matcher -- --nocapture` → PASS.
- [ ] **Step 5: Commit** `git commit -am "feat(skills): keyword matcher for skill activation"`

### Task C2: Fix bundled skill frontmatter

**Files:** `~/.aletheon/skills/hello/SKILL.md` (user config, not repo) — OR the loader's tolerance.

- [ ] **Step 1:** Inspect `~/.aletheon/skills/hello/SKILL.md`. If it's a stray sample, either
  (a) add minimal valid frontmatter, or (b) make the loader skip dirs without frontmatter
  silently at `debug` level instead of `warn` (`skills/loader.rs` / `manifest.rs:parse_skill_md`).
  Prefer (b) — a malformed user skill shouldn't spam `warn` every boot.
- [ ] **Step 2:** If editing the loader: change the `warn!` to `debug!` for the
  "must start with '---' frontmatter" case and continue. Add a test that a frontmatter-less
  dir is skipped without error.
- [ ] **Step 3: Commit** `git commit -am "fix(skills): tolerate frontmatter-less skill dirs (debug, not warn)"`

---

## Agent D — Session Control

### Task D1: SessionStore most_recent + new_session

**Files:** Modify `crates/aletheon-runtime/src/impl/session/store.rs`; `session/mod.rs` if needed

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn most_recent_returns_latest() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    store.create_session("s1").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.create_session("s2").unwrap();
    assert_eq!(store.most_recent().unwrap(), Some("s2".to_string()));
}
```

> Use the same `SessionStore::new` signature the daemon uses (`handler.rs:114`
> `SessionStore::new(&data_dir)`). If `tempfile` isn't a dev-dep, add it under
> `[dev-dependencies]` of `aletheon-runtime`.

- [ ] **Step 2: Implement** on `SessionStore`:

```rust
    /// Most recently active session id, if any. Reuses the DESC-by-last_active
    /// ordering already used by list_sessions.
    pub fn most_recent(&self) -> anyhow::Result<Option<String>> {
        let mut stmt = self.db.prepare(
            "SELECT id FROM sessions ORDER BY last_active DESC LIMIT 1"
        )?;
        let mut rows = stmt.query([])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }
```

> Match the actual table/column names in `store.rs` (survey says a `sessions` table with
> creation/last_active; verify exact column names before coding). `create_session` already
> exists for "new session".

- [ ] **Step 3: Test** `cargo test -p aletheon-runtime most_recent -- --nocapture` → PASS.
- [ ] **Step 4: Commit** `git commit -am "feat(session): SessionStore::most_recent"`

---

## Agent B — Daemon Integration (Batch 2)

> Single writer of `handler.rs`. Three sequential sub-tasks.

### Task B1: Connect MCP + inject tools at boot

**Files:** Modify `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1:** In `RequestHandler::new`, after the built-in `ToolRegistry` is built
  (around `handler.rs:133-166`) and before it's shared, connect MCP and register its tools:

```rust
        // Connect configured MCP servers and register their tools alongside built-ins.
        {
            use aletheon_body::r#impl::mcp::manager::McpManager;
            let mcp_config = app_config.mcp_servers.clone(); // adapt to McpConfig shape
            let mut mcp = McpManager::new(mcp_config.into());
            if let Err(e) = mcp.connect_all().await {
                tracing::warn!(error = %e, "MCP connect_all failed; continuing without MCP tools");
            }
            for wrapper in mcp.tool_wrappers() {
                let name = wrapper.name().to_string();
                if let Err(e) = aletheon_abi::Registry::register(&mut tools, std::sync::Arc::from(wrapper)) {
                    tracing::warn!(tool = %name, error = %e, "skip MCP tool (name clash?)");
                }
            }
        }
```

> Adapt: the `tools` registry's exact `register` API (Phase 1 used
> `Registry::<Arc<dyn Tool>>::register`); `app_config.mcp_servers` → `McpConfig`
> conversion (check `config.rs`). Verify before coding.

- [ ] **Step 2: Test** — runtime build + a test (if feasible) that a registry built with an
  empty MCP config still contains the built-ins (no regression).

Run: `cargo build -p aletheon-runtime && cargo test -p aletheon-runtime`
Expected: clean.

- [ ] **Step 3: Commit** `git commit -am "feat(daemon): connect MCP servers + inject MCP tools into registry"`

### Task B2: Fire the 6 missing hook points

**Files:** Modify `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1:** Fire the missing points at their natural sites (the `HookRegistry::execute`
  pattern is already used for PreTurn/PostTurn at `handler.rs:482,599`):
  - `OnSessionStart` — after `session_store.create_session(...)` in `new` (or first chat).
  - `PreTool` / `PostTool` — inside the `execute_tool` closure (`handler.rs:544-569`),
    before and after `runner.run(...)`, with `tool_name`/`tool_input`/`tool_result` in the
    `HookContext`.
  - `OnMemoryStore` / `OnMemoryRecall` — around the recall-memory `store`/`search` calls.
  - `OnSessionEnd` — on `clear`/session switch (best-effort).
- [ ] **Step 2: Test** — register a counting test hook on `PreTool` and assert it fires
  when a tool runs (use the existing hook registration test pattern; if hard to test
  through the full daemon, add a focused unit test on the closure's hook-firing helper).

Run: `cargo test -p aletheon-runtime` → PASS.

- [ ] **Step 3: Commit** `git commit -am "feat(daemon): fire PreTool/PostTool/Session/Memory hooks"`

### Task B3: Session RPC + keyword skill injection

**Files:** Modify `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add RPC methods** to the `handle()` match:
  - `"new_session"` → create a fresh `SessionStore` entry + reset the `SessionManager`,
    return the new id.
  - `"load_recent"` → `SessionStore::most_recent()` → if Some, recover it (reuse the
    existing `resume` path), return the id; else create new.
- [ ] **Step 2: Keyword skill injection** — in the `chat` arm, before building
  `effective_message`, gather the loaded skills' `(name, keywords, body)` from the
  `skill_loader`, run `match_skills(message, &skills)`, and append activated bodies to
  `effective_message` (the same place SandboxFirst notes / memory updates are injected,
  ~`handler.rs:452-480`), so keyword-triggered skills enter the turn dynamically (the
  cached system prefix stays stable for cache hits).
- [ ] **Step 3: Test** — runtime build; a focused test that `load_recent` on an empty store
  creates a session and on a populated store returns the latest.

Run: `cargo build -p aletheon-runtime && cargo test -p aletheon-runtime` → clean/PASS.

- [ ] **Step 4: Commit** `git commit -am "feat(daemon): new_session/load_recent RPC + keyword skill injection"`

---

## Batch 3 — Integration & Acceptance

### Task E1: Build + tests
- [ ] `cargo fmt --all && cargo build --workspace` → clean.
- [ ] `cargo test --workspace` → no failures; count up.
- [ ] `cargo clippy --workspace -- -D warnings` → clean. Commit fixups.

### Task E2: Defining acceptance test
- [ ] **Step 1: MCP tool callable.** Add a trivial stdio MCP server to
  `~/.aletheon/config.toml [[mcp_servers]]` (e.g. `@modelcontextprotocol/server-filesystem`),
  start the daemon, and confirm via logs that `connect_all` connected and the server's tools
  appear in the registry (and the agent can call one).
- [ ] **Step 2: Hooks fire.** Register a script hook on `PreTool`; run a task that uses a
  tool; confirm the hook ran (e.g. it appended to a log file).
- [ ] **Step 3: Session resume.** Have a chat, restart the daemon, `aletheon` →
  `load_recent` restores the prior conversation (history present).
- [ ] **Step 4: Skill triggers by keyword.** Put a skill with `keywords = ["commit"]`; send
  a message containing "commit"; confirm the skill body influenced the response (and a
  message without the keyword did not inject it).
- [ ] **Step 5: No more `skills/hello` warn at boot.** Confirm the boot log no longer warns.
- [ ] **Step 6:** Record outputs in the PR description.

---

## Self-Review (spec coverage)

- MCP discovery/invoke + inject into registry → **A1** (facade) + **B1** (wire). ✓
- Session persistence + recovery (resume exists; +most_recent/new) → **D1** + **B3**. ✓
- Hooks: 6 missing points fired → **B2** (8/8 total). ✓
- Skills keyword-matched injection → **C1** (matcher) + **B3** (inject); frontmatter bug → **C2**. ✓
- Acceptance (MCP callable, hooks fire, resume works, skill triggers, no boot warn) → **E2**.
- Multi-agent: A=mcp facade, C=skills matcher, D=session store, B=single-writer handler integration; B-after-(A,C,D). ✓

## Notes for implementing agents
- **This phase is mostly wiring** — verify the real signatures of `McpConnectionManager`,
  `get_all_tools`, `SessionStore`/columns, `HookRegistry::execute`, and the registry
  `register` API before writing. Do not invent APIs.
- `handler.rs` is single-writer (Agent B). A/C/D must land their building blocks first.
- Keep the cache-stable system prefix stable — inject keyword skills into the *user turn*,
  not the system prefix (B3), to preserve provider cache hits.
- Commit per task.
