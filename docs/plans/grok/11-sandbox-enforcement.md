# OS-Level Sandbox Enforcement

## 1. Why This Is Separate From Folder Trust

Folder Trust (doc 02) decides whether repo-local *executable config* (hooks, MCP, plugins) is trusted enough to load. The sandbox is different: it constrains what the agent *process itself* can read, write, and connect to at the OS level, regardless of trust.

```text
Folder Trust                    Sandbox
---------------------------     ----------------------------------
Gates loading repo config       Limits process capability
Per-repo, per-principal         Per-process, applied once at startup
Decides "should I run this?"    Enforces "you cannot reach this"
```

Grok applies both. Aletheon has `WorkspacePolicy` for writable roots (`crates/fabric/src/types/local_authority.rs:70-117`) but no kernel-enforced sandbox at comparable granularity.

## 2. Grok's Sandbox Architecture

### 2.1 Five Built-in Profiles

| Profile | Read | Write | Network | Use Case |
|---|---|---|---|---|
| `workspace` | Full FS (default) | Essential paths only | Open | Default interactive use |
| `devbox` | Full FS (default) | Everything except `/data` | Open | Dev containers |
| `read-only` | Full FS (default) | Minimal essential paths | Blocked (child) | Audit / review |
| `strict` | Allowlist only (system + workspace) | Essential paths | Blocked (child) | Untrusted repos |
| `off` | — | — | — | No sandbox |

`essential_writable_paths` provides `~/.grok/`, workspace, temp dirs, and cache. `essential_writable_paths_minimal` (for read-only) further restricts to just `~/.grok/` and temp.

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-sandbox/src/profiles.rs:286-445`

### 2.2 Custom Profiles via sandbox.toml

Users and projects define custom profiles that extend a built-in base:

```toml
# ~/.grok/sandbox.toml
[profiles.enterprise]
extends = "workspace"
restrict_network = true
read_only = ["/data"]
deny = ["/home/user/.ssh", "/etc/ssl/private"]
```

Key rules (from `profiles.rs:111-127`):

1. **Global config** (`~/.grok/sandbox.toml`) loads first.
2. **Project config** (`.grok/sandbox.toml`) is *additive only* — new profile names only. It **cannot** redefine a name already in the global config. This prevents a malicious workspace from hollowing out an enterprise custom profile while keeping the trusted name (`profiles.rs:155-159`).
3. Custom profiles can extend built-in bases but not other custom profiles.
4. `extends = "off"` is rejected — no custom sandbox can start from zero.

### 2.3 Dual Enforcement Layer

Grok applies two OS mechanisms that compose:

**Layer 1: Landlock / Seatbelt (nono crate)**

Applied in-process via the `nono` crate. Converts the resolved `SandboxProfile` into a `CapabilitySet` with explicit read/write/deny paths, then applies it irreversibly to the current process.

- Linux: Landlock LSM (`profiles.rs:199-275`)
- macOS: Seatbelt sandbox
- Degrades gracefully on unsupported platforms — logs a warning and continues without enforcement
- Feature-gated: `enforce` feature pulls in `nono`; without it, only lightweight logging helpers compile

**Layer 2: bwrap mount namespace (Linux only)**

For deny semantics: Linux Landlock has no `deny_path` primitive — you can only *allow*, not *deny*. Grok compensates with a `bwrap` re-exec:

1. Before `nono`, Grok checks if deny paths are required.
2. If so, it re-execs itself inside `bwrap` with deny-write paths mounted read-only and deny-read paths bound over with zero-permission placeholders.
3. `BWRAP_ENV_VAR` prevents double-wrapping.

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-sandbox/src/lib.rs:249-487`

The composition: bwrap handles the deny set + `/data` write-deny; nono handles the allow set. Both apply before any agent logic runs.

### 2.4 Deny Globs

Deny entries can be glob patterns (e.g., `**/*.pem`, `**/.env*`):

- **macOS**: Seatbelt converts globs to anchored regexes — enforced at runtime, covers files created after sandbox application.
- **Linux**: bwrap can't enforce runtime globs. Globs are expanded once at launch with hard caps (`DENY_GLOB_MAX_DEPTH`, `DENY_GLOB_MAX_MATCHES`, `DENY_GLOB_MAX_ENTRIES`). Files matching a glob pattern created *after* launch are NOT covered on Linux (documented best-effort warning).
- If glob expansion hits a cap, the sandbox refuses to start (fail-closed).

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-sandbox/src/lib.rs:405-442`

### 2.5 Child Network Seccomp

When a profile restricts network (`read-only` or `strict`), Grok installs a seccomp filter that blocks network syscalls for child processes launched through known paths. The in-process agent still needs network (LLM API), but subprocesses spawned as tools cannot reach the network.

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-sandbox/src/lib.rs:71-76`, `child_net.rs`

### 2.6 Violation Monitoring

Sandbox violations are logged with structured events, atomic metrics, and immediate disk flush:

- `SandboxEventType`: `ProfileApplied`, `ApplyFailed`, `FsViolation`, `NetViolation`, `BypassGranted`, `BypassDenied`
- `SandboxMetrics`: atomic counters for fs/net violations, bypass grants/denies
- `SandboxLogger`: buffers events, supports immediate flush to disk

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-sandbox/src/types.rs:1-193`

## 3. Relationship to Aletheon's Current Security

Aletheon already has:

| Mechanism | What it does | Sandbox relationship |
|---|---|---|
| `WorkspacePolicy` | cwd, writable roots, protected paths | Sandbox adds deny and network restrictions **on top** |
| `ProtectedPathPolicy` | Credential path protection | Sandbox deny can add a second, OS-level layer |
| Kernel sandbox | Admission/lease model | Different layer — governs agent *admission*, not process capability |
| Seccomp per-tool | Not present | Grok's child_net seccomp is a candidate to add |

## 4. Suggested Adaptation for Aletheon

### 4.1 Conceptual Fit

The sandbox fits naturally between `WorkspacePolicy` and the tool execution layer:

```text
WorkspaceSelection -> WorkspacePolicy -> SandboxManager -> GovernedCapabilityInvoker
                                            |
                                     OS-level deny + network
                                     applied once before any tool runs
```

### 4.2 What to Adopt

| Element | Priority | Rationale |
|---|---|---|
| Profile system (workspace/strict/read-only/custom) | High | Aletheon already has policy; profiles make sandboxing usable |
| Hierarchical config (global > project, additive only) | High | Prevents repo-level sandbox hollowing |
| Dual enforcement (bwrap + Landlock/Seatbelt) | Medium | bwrap is Linux-only; Aletheon may prioritize Landlock-first |
| Deny globs with fail-closed caps | Medium | Important for `**/.env` and similar patterns |
| Child network seccomp | Medium | Depends on whether Aletheon tools spawn subprocesses |
| Violation metrics + events | High | Aletheon already has an event spine; sandbox violations should feed it |

### 4.3 What to Adapt for Aletheon's Multi-User Model

Grok's sandbox is per-process (applied once at startup). Aletheon is a daemon with multiple principals. Adaptation points:

1. **Per-principal, not per-process**: Sandbox profile resolution should incorporate principal identity. Alice's `strict` might differ from Bob's.
2. **Per-Agent, not global**: child Agents (via AgentControl) can inherit a *narrower* sandbox but never a wider one.
3. **Profile config source**: Aletheon's sandbox config should live in its own authority domain, not in untrusted repo `.grok/` directories. Trusted profiles come from the daemon's config store.
4. **bwrap re-exec is incompatible with a daemon**: Aletheon's daemon can't re-exec itself per-agent. Instead, sandbox enforcement should happen at Agent process fork (if Agents are separate processes) or via Landlock per-thread (if Agents are threads in the daemon).

### 4.4 Suggested Profile Mapping

| Grok Profile | Aletheon Equivalent |
|---|---|
| `workspace` | Default — read full FS, write within `WorkspacePolicy` writable roots |
| `devbox` | Not applicable (no `/data` convention in Aletheon) |
| `read-only` | Audit/observation mode — read FS, minimal writes, no child network |
| `strict` | Untrusted workspace — allowlist FS, deny protected paths, no network |
| `off` | Legacy / no sandbox (not recommended except for compatibility) |
| `Custom(name)` | Enterprise profiles from daemon config, not repo-local sandbox.toml |

## 5. Security-Specific Notes

- **TOCTOU**: Grok canonicalizes paths before comparison (`lib.rs:85-86` referencing Aletheon's existing canonicalization). Aletheon already does this in `WorkspacePolicy` resolution.
- **Placeholder PID-suffixing**: bwrap blocked-source placeholders are PID-suffixed to prevent races between concurrent grok processes (`lib.rs:316`). Aletheon should use per-Agent unique placeholders if using similar bind-over techniques.
- **Fail-closed is the default for glob overflow**: when deny glob expansion hits its cap, the sandbox refuses to start. No partial enforcement.
- **`requires_read_deny` is intrinsic**: it checks the raw profile config (not the resolved/expanded set, which could be empty-on-error and silently downgrade to fail-open). This principle should carry over to any Aletheon sandbox implementation.

## 6. Acceptance Direction

1. Agent launched under `strict` profile cannot read paths outside the allowlist.
2. Deny globs prevent access to matched paths even if they would otherwise be readable.
3. Custom profile from daemon config cannot be overridden by repo-local config.
4. Sandbox violation events appear in the canonical event spine with principal/agent attribution.
5. Child Agent inherits a sandbox at most as permissive as its parent.
6. Degraded enforcement (platform doesn't support Landlock) logs a warning and continues with restricted capabilities rather than silently dropping all restrictions.

## 7. Relationship to Other G-documents

- **G1 (Folder Trust)**: Trust gates config *loading*; sandbox gates process *capability*. Both are needed.
- **G2 (Streaming Tools)**: Sandbox violations during tool execution must be reported as progress/notification events without replacing the terminal result.
- **G6 (Subagent Settlement)**: Child Agent sandbox lease must be released at settlement, and violation metrics attributed to the correct principal.
- **G7 (Memory/Credential)**: Sandbox deny on credential paths provides a second, kernel-level layer beyond `ProtectedPathPolicy`.
