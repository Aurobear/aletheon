# Tier 0 — Hygiene & Truth — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Make the repo's structure match its own description — delete the dead `binaries/` crate, correct the README crate names, make the shipped `config/default.toml` actually start a daemon, and reconcile the divergent socket paths.

**Architecture:** Pure deletion + docs + config. No behavior change to live code. Every claim below was re-verified against the repo on 2026-07-01 (anchors inline).

**Tech Stack:** Rust (Cargo workspace), TOML, Markdown.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "Tier 0 — Hygiene & Truth"

**Branch:** `auro/feat/20260701-aletheon-tier0-hygiene` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Claim | Evidence |
|---|---|
| `crates/binaries/` is NOT a workspace member | root `Cargo.toml` members = base, cognit, corpus, dasein, interact, memory, metacog, runtime, examples/* (no `binaries`) |
| `binaries/aletheond` depends on nonexistent `aletheon-runtime` | `crates/binaries/aletheond/Cargo.toml:12` `path = "../../aletheon-runtime"` |
| `binaries/aletheon-cli` depends on nonexistent `aletheon-body` | `crates/binaries/aletheon-cli/Cargo.toml` `aletheon-body = { path = "../../aletheon-body" }` |
| Real binaries live elsewhere | `crates/runtime/Cargo.toml:8-14` (`aletheond`, `aletheon-exec`), `crates/interact/Cargo.toml:8-10` (`aletheon`) |
| No script/CI references `binaries/` | grep for `binaries/` in `*.toml/*.sh/*.yml/*.yaml` (excluding the crate itself) = empty |
| README uses stale crate names | `README.md:185-211` uses `aletheon-abi/comm/self/brain/body/runtime/cli/meta` |
| `default.toml` cannot start a daemon | `config/default.toml` has `[agent] default_model` but no `[[providers]]` and no `default_provider`; error path `cognit/src/impl/provider_registry.rs:94` `"Default provider '{}' not found"` |
| `ProviderConfig` shape | `runtime/src/core/config/provider.rs:28-42` → `{ name, base_url, api_key?, transport?, models?, max_context_length? }`; canonical TOML at `mod.rs:218-228` |
| Socket paths diverge | `base/src/types/paths.rs:11` `SOCKET_DIR = "/var/run/aletheon"`; `config/default.toml` `[daemon] socket_path = "/run/aletheon/aletheon.sock"`; awareness scans `/var/run/aletheon/*.sock` (`corpus/.../awareness/discovery.rs:3`) |

---

## File map

| File | Change |
|---|---|
| `crates/binaries/**` | **delete** (dead crate, not a workspace member) |
| `config/default.toml` | add `[[providers]]` + `[agent] default_provider`; fix `[daemon] socket_path` |
| `crates/runtime/src/core/config/mod.rs` | add a test that parses the shipped `config/default.toml` |
| `README.md` | rewrite §5 crate list + dependency graph to real crate names; add concept-mapping table |

---

## Phase 1 — Delete the dead `binaries/` crate  ✅ DONE (2026-07-01)

> **Status:** `crates/binaries/` has already been removed by the owner. Verified
> `ls crates/binaries` → "No such file or directory", and the workspace
> `Cargo.toml` members list contains no `binaries` entry (only base, cognit,
> corpus, dasein, interact, memory, metacog, runtime, examples/*). The tasks
> below are retained as a record; **only Step 3 (build-verify) remains actionable**
> as a sanity check.

### Task 1: Remove `crates/binaries/` and prove the workspace still builds

**Files:**
- Delete: `crates/binaries/` (subdirs `aletheond/`, `aletheon-cli/`, `aletheon-exec/`) — **already deleted**

- [x] **Step 1: Pre-check — confirm nothing references it** (done implicitly: not a workspace member)

- [x] **Step 2: Delete the crate** — already removed by the owner.

- [ ] **Step 3: Verify the workspace still builds** (only remaining action)

Run: `cargo build --workspace`
Expected: builds; the real binaries (`aletheond`, `aletheon-exec`, `aletheon`) still resolve from `runtime`/`interact`. (`cargo build -p aletheond -p aletheon-exec -p aletheon` also succeeds.) If this fails, something still referenced `binaries/` — stop and investigate.

- [x] **Step 4: Commit** — deletion already committed by the owner; no action.

---

## Phase 2 — Make `config/default.toml` runnable + reconcile socket path

### Task 2: Add providers + default_provider; align socket path

**Files:**
- Modify: `config/default.toml`

The current file (verified) is:
```toml
[agent]
default_model = "claude-sonnet-4-20250514"
max_iterations = 50
...
[daemon]
socket_path = "/run/aletheon/aletheon.sock"
log_level = "info"
```
Problems: no `[[providers]]` and no `default_provider` (daemon exits with
`Default provider '' not found`), and `socket_path` uses `/run/aletheon` while the
code's canonical dir is `base::paths::SOCKET_DIR = /var/run/aletheon`.

- [ ] **Step 1: Rewrite `config/default.toml` to a minimal-but-runnable default**

Use the exact `ProviderConfig` field names from `provider.rs:28-42` and the
canonical socket dir from `base/src/types/paths.rs:11`. Keep a documented
placeholder key so a fresh checkout starts once the key is filled in:

```toml
[agent]
# Pick a model your provider serves; override in your local config.
default_model = "claude-sonnet-4-20250514"
default_provider = "anthropic"
max_iterations = 50

# At least one provider is required for the daemon to start.
# Fill in api_key (or set it via your local override config), then run the daemon.
[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
api_key = ""            # REQUIRED: set before first run (or override locally)
transport = "anthropic" # openai | anthropic | auto
models = ["claude-sonnet-4-20250514"]

[sandbox]
preference = "auto"
# bubblewrap_path = "/usr/bin/bwrap"

[plugins]
directories = []

[memory]
backend = "sqlite"
data_dir = "~/.aletheon/memory"

[daemon]
# Canonical socket dir is base::paths::SOCKET_DIR = /var/run/aletheon
socket_path = "/var/run/aletheon/aletheon.sock"
log_level = "info"
```

> Note: `api_key = ""` is intentionally blank (never commit a real key). The
> daemon fails clearly on a blank key at request time; `default_provider` being
> present means it no longer fails at *startup* with the empty-provider error.

- [ ] **Step 2: Add a test that the shipped default config parses and is startable-shaped**

Add to `crates/runtime/src/core/config/mod.rs` tests module:

```rust
#[test]
fn shipped_default_config_is_startable_shaped() {
    // repo-root config/default.toml relative to this crate (crates/runtime)
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/default.toml");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    let cfg: AppConfig = toml::from_str(&text).expect("default.toml must parse");
    assert!(!cfg.providers.is_empty(), "default.toml must define >=1 provider");
    let dp = cfg.agent.default_provider.as_deref()
        .expect("default.toml must set agent.default_provider");
    assert!(cfg.providers.iter().any(|p| p.name == dp),
        "default_provider '{dp}' must match a [[providers]] name");
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p runtime config::tests::shipped_default_config_is_startable_shaped`
Expected: PASS (fails before Step 1's edit, passes after).

- [ ] **Step 4: Commit**

```bash
git add config/default.toml crates/runtime/src/core/config/mod.rs
git commit -m "fix(config): make default.toml startable (providers + default_provider) and align socket path"
```

> **Scope guard:** This task aligns the *shipped default config* to the code's
> canonical socket dir. It does NOT change `base::paths::SOCKET_DIR` or the CLI
> client default. If the CLI client default (`interact`) still points elsewhere,
> reconciling it is a follow-up (verify `interact` DEFAULT_SOCKET before touching;
> out of scope here to keep the change low-risk).

---

## Phase 3 — Correct the README to real crate names

### Task 3: Rewrite README §5 (Crate Architecture) + add concept map

**Files:**
- Modify: `README.md` (§5 crate list ~`:185-211`; check §4 "Nous Architecture" `:124-175` for stale names too)

This is documentation — verification is visual + a grep that the stale names are gone from the architecture section.

- [ ] **Step 1: Replace the stale crate tree and dependency graph**

Replace the `aletheon-abi/comm/self/brain/body/runtime/cli/meta` names with the
real workspace crates. Real mapping (from root `Cargo.toml` members + concept
doc):

| Crate | Concept | Role |
|---|---|---|
| `base` | ABI | IPC, tool/message/sandbox/LLM types, `paths` |
| `dasein` | Self | identity, boundary, care, narrative |
| `cognit` | Brain | reasoning, planning, reflection, provider routing |
| `corpus` | Body | tools, sandbox, perception, MCP, drivers |
| `runtime` | Runtime | cognitive loop, orchestration, daemon (`aletheond`, `aletheon-exec` bins) |
| `interact` | Interface | CLI + TUI client (`aletheon` bin) |
| `memory` | Memory | cognitive memory backends (episodic/semantic/procedural/self) |
| `metacog` | Meta | self-evolution scaffolding |

Real binaries: `aletheond` + `aletheon-exec` (`crates/runtime/Cargo.toml:8-14`),
`aletheon` (`crates/interact/Cargo.toml:8-10`). Draw the dependency graph from the
actual `Cargo.toml` dep lists (note: `cognit → corpus/interact` inversion exists
today — Tier 2c will fix it; the README should describe *current* reality, not the
target, or footnote the inversion).

- [ ] **Step 2: Verify the stale names are gone from the architecture section**

Run:
```bash
cd /home/rj001/Bear-ws/work/aletheon
grep -n "aletheon-self\|aletheon-brain\|aletheon-body\|aletheon-comm\|aletheon-abi\|aletheon-meta" README.md \
  || echo "OK: no stale crate names remain"
```
Expected: `OK:` (or only intentional historical mentions clearly marked as old names).

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): correct crate architecture to real crate names + concept map"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** delete binaries (Task 1) ↔ spec problem 1; README (Task 3) ↔ problem 2; config (Task 2) ↔ problem 3; socket reconciliation (Task 2) ↔ verified drift. All Tier 0 spec items mapped.
- **Placeholder scan:** none — exact commands, exact TOML, exact test.
- **Type consistency:** `ProviderConfig` field names in the new `default.toml` match `provider.rs:28-42`; the parse test uses real `AppConfig`/`agent.default_provider`/`providers` accessors (`mod.rs:30`, `agent.rs:48`).

## Risks / notes for the implementer

- **Deletion is the highest-value, lowest-risk step** — do Task 1 first; if
  `cargo build --workspace` breaks, something *did* reference `binaries/` and the
  pre-check missed it (stop and investigate).
- **Never commit a real key** in `default.toml` (`api_key = ""`). Real keys live
  in local override config only (see repo memory: config with keys is not committed).
- **`default_model`** in `default.toml` is a config value, not code — leaving a
  concrete model string there is fine, but keep it a placeholder users override.
- **Socket path:** this plan aligns `default.toml` to `base::paths::SOCKET_DIR`
  only. A full audit (CLI client default, transport `/tmp/agent-ipc` in
  `unix_socket_transport.rs:26`) is deliberately out of scope — that envelope IPC
  path may be a separate subsystem; don't touch it without checking callers.
- **No product-logic change:** Tier 0 must not alter runtime behavior. If a task
  tempts you into logic changes, it belongs in Tier 2, not here.
