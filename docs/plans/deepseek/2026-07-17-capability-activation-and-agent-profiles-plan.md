# Capability Activation and Agent Profile System Plan

> **Status:** Proposed（前提已校正 2026-07-17）
> **Target branch:** `dev`
> **Baseline:** Aletheon `65f74981`
> **Execution rule:** Phase 1 is config-only (change TOML/MD files, no Rust). Each subsequent phase adds one Rust module at a time, keeping the daemon operational.

> **⚠ 前提校正 (code-level re-check 2026-07-17):** 本计划假设 `.md` profile 携带 YAML frontmatter、`.toml` 为 legacy。**实际相反**：`agents/code-agent.md` / `fs-agent.md` **没有 frontmatter**（纯 markdown），而 AgentLoader（`agent_loader/mod.rs:60-66`）扫描 `*.md` 时要求 frontmatter——因此当前 `.md` profile **不授予任何工具**。真实授权源是 `agents/code-agent.toml:6 = ["file_read","file_write","bash_exec"]`（只 3 个）。执行 Phase 1 前必须先决定：(A) 给 `.md` 补 frontmatter 并让 loader 生效，或 (B) 直接扩展 `.toml` 授权表。另注：`max_iterations` quick-win 已无需做——默认已是 50（`config/agent.rs:42`）。

## 1. Executive conclusion

Aletheon has **20+ fully implemented, tested tools** registered in `ToolRegistry::default()`. The default agent profile (`code-agent`) only grants **3** of them. This plan activates the already-built capability surface through four actions: audit every tool, define tiered agent profiles, build an `AgentProfile` system that maps profiles to tool grants, and stage activation in order of risk.

The end state is not "add new tools." The end state is "unlock tools that already exist."

```text
current:  ToolRegistry::default() has 20+ tools
          -> code-agent.toml grants {file_read, file_write, bash_exec}
          -> ExtensionGrant per-session carries all capabilities
          -> no profile-level tool gating, no profile switching at runtime

target:   ToolRegistry::default() has 20+ tools (unchanged)
          -> AgentProfile struct defines default_tools, model, prompt, limits
          -> profile maps to ExtensionGrant via config
          -> admin-agent has all tools, safe-agent has read-only only
          -> runtime /perf command can switch profiles
          -> new sessions inherit profile defaults from config
```

## 2. Method and confidence

This plan is based on static inspection of:

- `crates/corpus/src/tools/tools/registry.rs` -- ToolRegistry::default() and all 20+ tool registrations
- `crates/corpus/src/tools/tools/{bash_exec,file_read,file_write,system_status,process_list,ebpf_compile,module_build,module_load,kernel_build,code_graph,file_search,apply_patch,glob,grep,web_fetch,web_search,task_tools}.rs` -- individual tool implementations
- `crates/fabric/src/types/tool.rs` -- PermissionLevel enum, Tool trait, ToolContext
- `crates/fabric/src/types/agent_control.rs` -- AgentProfile struct (id, system_prompt, model, allowed_tools, limits)
- `agents/code-agent.toml`, `agents/fs-agent.toml`, `agents/net-agent.toml` -- current agent profile definitions
- `agents/code-agent.md`, `agents/fs-agent.md`, `agents/net-agent.md` -- system prompts with YAML frontmatter
- `crates/executive/src/impl/agent_loader/mod.rs` -- AgentLoader, AgentRole struct, YAML frontmatter parsing
- `crates/executive/src/impl/daemon/bootstrap/runtime.rs` -- load_agent_profiles(), register_agent_tools()
- `crates/executive/src/impl/daemon/bootstrap/request.rs` -- RequestHandler::new(), full daemon bootstrap including tool registration and profile loading (lines 293-528)
- `crates/executive/src/core/config/agent.rs` -- ExecutiveConfig, AgentLoopConfig
- `crates/executive/src/core/config/mod.rs` -- AppConfig, ConfigLayer, layered loading
- `crates/corpus/src/tools/capability_executor.rs` -- discover_tool_extensions(), tool_risk_levels()
- `crates/corpus/src/service.rs` -- ExtensionGrant, ActivationRequest, CorpusService

All line counts, permission levels, and struct fields are verified against the source. Tool implementations exist and pass unit tests (references to assert lines in tool implementations).

## 3. Complete tool audit

### 3.1 Tool inventory

Every tool listed below exists in `crates/corpus/src/tools/tools/`, is registered in `ToolRegistry::default()` at `crates/corpus/src/tools/tools/registry.rs:109-184`, and has a concrete `impl Tool` with defined `permission_level()` and `execute()`.

| # | Tool | LOC | File | Permission | Risk | Mutable | Sandbox Needed | Quality Assessment |
|---|------|-----|------|-----------|------|---------|----------------|-------------------|
| 1 | bash_exec | 116 | bash_exec.rs | L1 | Sandboxed | Yes | Yes | Solid; tokio::Command, timeout support, output capture. Test report noted 25% error rate on complex commands -- output bounding and error classification should be hardened before full activation (see section 6.3). |
| 2 | file_read | 94 | file_read.rs | L0 | ReadOnly | No | No | Solid; offset/limit, binary detection, truncation metadata. |
| 3 | file_write | 97 | file_write.rs | L1 | Sandboxed | Yes | Yes | Solid; creates parent directories, content+path params. |
| 4 | system_status | 71 | system_status.rs | L0 | ReadOnly | No | No | Basic; returns OS, arch, cwd, env vars as JSON. Needs output bounding (env vars can be large). |
| 5 | process_list | 70 | process_list.rs | L0 | ReadOnly | No | No | Basic; runs `ps aux` via Command. Needs output bounding. |
| 6 | ebpf_compile | 248 | ebpf_compile.rs | L2 | SystemModify | No* | No* | Solid implementation; clang/bpf compilation pipeline, output artifact path. *Compiles BPF bytecode, does not load it. |
| 7 | module_build | 230 | module_build.rs | L2 | SystemModify | No* | No* | Solid; builds kernel modules via make. *Builds .ko files, does not load them. |
| 8 | module_load | 292 | module_load.rs | L3 | Destructive | Yes | Yes | Solid; insmod/rmmod wrappers, dependency resolution. Requires root. Destroys kernel stability if misused. |
| 9 | kernel_build | 541 | kernel_build.rs | L3 | Destructive | Yes | Yes | Most complex tool; full kernel build + install pipeline, config management, bootloader update. Requires root. Irreversible. |
| 10 | code_graph | 437 | code_graph.rs | L0 | ReadOnly | No | No | Production-quality; tree-sitter AST queries, symbol extraction, call graph generation. Supports multiple languages. |
| 11 | file_search | 427 | file_search.rs | L0 | ReadOnly | No | No | Production-quality; ripgrep-backed content search, filename search, regex support. ConcurrencyClass::ReadOnly. |
| 12 | apply_patch | 771 | apply_patch.rs | L1 | Sandboxed | Yes | Yes | Large implementation; unified diff parsing, hunk application, conflict detection, dry-run mode. Needs structured patch format hardening (see code-editing plan). |
| 13 | glob | 383 | glob.rs | L0 | ReadOnly | No | No | Production-quality; glob pattern matching, .gitignore respect, ConcurrencyClass::ReadOnly. |
| 14 | grep | 330 | grep.rs | L0 | ReadOnly | No | No | Production-quality; regex search, file-type filtering, context lines, ConcurrencyClass::ReadOnly. |
| 15 | web_fetch | 245 | web_fetch.rs | L1 | Sandboxed | No | No | Solid; HTTP client fetch, User-Agent, timeout, content-type detection. Network-capable. |
| 16 | web_search | 224 | web_search.rs | L1 | Sandboxed | No | No | Solid; search engine query, configurable provider, result formatting. Network-capable. |
| 17 | task_create | 631* | task_tools.rs | L0 | ReadOnly | No | No | Shared TaskStore; creates structured task with title, description, priority. |
| 18 | task_update | 631* | task_tools.rs | L0 | ReadOnly | No | No | Updates task status, description, priority fields. |
| 19 | task_list | 631* | task_tools.rs | L0 | ReadOnly | No | No | Lists tasks with optional status filter. |
| 20 | task_get | 631* | task_tools.rs | L0 | ReadOnly | No | No | Retrieves single task by ID. |

\* Task tools share one file (task_tools.rs, 631 lines total, 4 structs).

**Additional tools registered at daemon bootstrap** (not in ToolRegistry::default(), added during `RequestHandler::new()` at `crates/executive/src/impl/daemon/bootstrap/request.rs:293-528`):

| # | Tool | Source | Permission | Notes |
|---|------|--------|-----------|-------|
| 21 | core_memory_append | mnemosyne::CoreMemoryAppendTool | Internal | Memory store write |
| 22 | core_memory_replace | mnemosyne::CoreMemoryReplaceTool | Internal | Memory store replace |
| 23 | memory_search | mnemosyne::MemorySearchTool | Internal | Cross-store memory search |
| 24 | google_gmail_search | Google read integration | L0 | Gmail search, conditional on Google config |
| 25 | google_gmail_read | Google read integration | L0 | Gmail read, conditional on Google config |
| 26 | google_calendar_list | Google read integration | L0 | Calendar list, conditional on Google config |
| 27 | MCP tools | McpManager tool_wrappers | Varies | Dynamic, per-server |
| 28 | Plugin/skill tools | SkillLoader plugins | Varies | Dynamic, per-plugin |
| 29 | agent_control tools | AgentControlTools | Internal | Agent lifecycle management |
| 30 | agent (compatibility) | AgentTool | Internal | Legacy sub-agent spawning |

### 3.2 Permission level mapping

The `PermissionLevel` enum is defined at `crates/fabric/src/types/tool.rs:20-29`:

```rust
pub enum PermissionLevel {
    L0,  // Read-only, no side effects
    L1,  // Write within sandbox
    L2,  // System-level changes
    L3,  // Destructive / irreversible
}
```

These map to `RiskLevel` for admission at `crates/corpus/src/tools/capability_executor.rs:71-78`:

- L0 -> ReadOnly
- L1 -> Sandboxed
- L2 -> SystemModify
- L3 -> Destructive

The `SecurityPolicy` at `crates/fabric/src/security/policy.rs:102-108` infers levels from tool names as a fallback, with file_read/system_status/process_list/memory_search at L0, file_write/bash_exec at L1, and everything else at L1.

### 3.3 Current activation gap

The current `code-agent.toml` at `agents/code-agent.toml:6` grants:

```toml
tools = ["file_read", "file_write", "bash_exec"]
```

This means **17 of 20 built-in tools** are fully implemented, registered, tested, but **never exposed** to any agent. The daemon bootstrap at `request.rs:293` creates `ToolRegistry::default()` with all 20 tools, but only the subset listed in agent profiles is included in the `AgentProfile.allowed_tools` field at `runtime.rs:46-50`.

The `load_agent_profiles()` function at `runtime.rs:11-69` resolves tool names from agent profiles against the catalog of tool definitions:

```rust
// runtime.rs:36-44
for name in &role.tools {
    let definition = catalog.get(name).cloned().with_context(|| {
        format!("Agent profile '{}' references unknown tool '{name}'", role.name)
    })?;
    tools.push(definition);
}
```

This means referencing a tool name in an agent's `.md` or `.toml` frontmatter is sufficient to grant it -- **no code change needed for Phase 1 activation**. The `AgentProfile` struct at `crates/fabric/src/types/agent_control.rs:84-95` carries `allowed_tools: Vec<String>` and is constructed from the resolved definitions.

## 4. Tiered agent profiles

### 4.1 Profile design principles

1. **Default = capable but safe.** The default profile should include all read-only tools plus sandboxed write tools that are well-tested.
2. **Tiers are cumulative.** Each higher tier includes the lower tier, avoiding configuration drift.
3. **Explicit model + limits.** Each profile declares its default model, iteration limits, and tool call budget.
4. **System prompt per profile.** Each profile has its own `.md` file with role-specific instructions.
5. **TOML frontmatter in `.md` files.** Follow the existing `AgentLoader` convention where agent definitions are `.md` files with YAML frontmatter (see `crates/executive/src/impl/agent_loader/mod.rs:30-42`). The `.toml` files at `agents/code-agent.toml` etc. are not used by the current `AgentLoader` which only scans `*.md` files. The `.toml` files appear to be legacy/alternative definitions.

### 4.2 Profile definitions

#### Profile: `safe-agent` (read-only, no side effects)

**File:** `agents/safe-agent.md`

```markdown
---
name: safe-agent
description: "Read-only agent with no side effects -- safe for untrusted input"
tools: file_read, glob, grep, file_search, code_graph, web_search, system_status, process_list, task_create, task_update, task_list, task_get
model: deepseek-v4
max_iterations: 10
role: Leaf
---

You are a read-only analysis agent. You can inspect code, search files, query
the web, check system status, and manage task lists. You cannot modify files,
execute commands, or affect any system state.

## Core rules
- Never attempt to write, delete, or modify files
- Never execute shell commands
- Use glob, grep, and file_search to explore the codebase
- Use code_graph for AST-level code analysis
- Use web_search to find documentation or external information
- Use task tools to track progress on complex analysis
- Report findings clearly with file paths and line numbers
```

**Tools granted (12):** file_read, glob, grep, file_search, code_graph, web_search, system_status, process_list, task_create, task_update, task_list, task_get

**Risk assessment:** L0-only tools. Zero mutation risk. All read-only operations.

---

#### Profile: `code-agent` (default -- read + write + execute)

**File:** `agents/code-agent.md` (replaces existing)

```markdown
---
name: code-agent
description: "Full code agent with read, write, execute, search, and web capabilities"
tools: file_read, file_write, bash_exec, glob, grep, file_search, code_graph, apply_patch, web_search, web_fetch, task_create, task_update, task_list, task_get
model: deepseek-v4
max_iterations: 25
role: Leaf
---

You are a general-purpose coding agent. You have full read/write access to the
workspace, can execute shell commands, search code, analyze ASTs, apply patches,
fetch web resources, and manage structured tasks.

## Core rules
- Read files before editing them
- Use glob/grep/file_search instead of bash_exec for code exploration
- Use code_graph for understanding cross-references and call graphs
- Use apply_patch for targeted edits (prefer over file_write for code changes)
- Use bash_exec only when no dedicated tool exists for the task
- Use web_search to find documentation, API references, or examples
- Use web_fetch sparingly to retrieve specific URLs
- Create tasks for multi-step work to track progress
- Report errors clearly and do not silently retry failed mutations
```

**Tools granted (14):** file_read, file_write, bash_exec, glob, grep, file_search, code_graph, apply_patch, web_search, web_fetch, task_create, task_update, task_list, task_get

**Risk assessment:** L0+L1 tools. Mutation possible through file_write, bash_exec, apply_patch. All writes are sandbox-gated by `ToolRunnerWithGuard` at `request.rs:426-428`.

---

#### Profile: `system-agent` (code-agent + system-level tools)

**File:** `agents/system-agent.md`

```markdown
---
name: system-agent
description: "System-level agent with kernel build, eBPF, and module management capabilities"
tools: file_read, file_write, bash_exec, glob, grep, file_search, code_graph, apply_patch, web_search, web_fetch, task_create, task_update, task_list, task_get, system_status, process_list, ebpf_compile, module_build, module_load, kernel_build
model: deepseek-v4
max_iterations: 30
role: Leaf
---

You are a system-level development agent with full access to kernel build, eBPF
compilation, and kernel module management. You have all code-agent capabilities
plus system-level tools.

## Additional rules
- ebpf_compile: compile eBPF programs from C source. Does NOT load them.
- module_build: build kernel modules. Does NOT load them.
- module_load: load/unload kernel modules. REQUIRES EXPLICIT USER APPROVAL.
- kernel_build: build and install a Linux kernel. REQUIRES EXPLICIT USER APPROVAL.
- system_status: check OS, arch, env vars. Output may be large.
- process_list: list running processes. Output may be large.
- Always ask for confirmation before module_load or kernel_build.
- Use ebpf_compile and module_build freely; they produce artifacts safely.
```

**Tools granted (20):** All 20 built-in tools.

**Risk assessment:** L2+L3 tools included. module_load and kernel_build are destructive. This profile should require explicit user approval for L3 operations.

---

#### Profile: `admin-agent` (all tools, no restrictions)

**File:** `agents/admin-agent.md`

```markdown
---
name: admin-agent
description: "Administrative agent with unrestricted access to all capabilities"
tools: file_read, file_write, bash_exec, glob, grep, file_search, code_graph, apply_patch, web_search, web_fetch, task_create, task_update, task_list, task_get, system_status, process_list, ebpf_compile, module_build, module_load, kernel_build
model: deepseek-v4
max_iterations: 50
role: Leaf
---

You are an administrative agent with unrestricted access to every Aletheon
capability. You can read, write, execute, build kernels, load modules, search
the web, and manage all system resources.

## Core rules
- You have ZERO restrictions. Every tool is available.
- You are responsible for your own safety.
- module_load and kernel_build can destabilize the system.
- Use with extreme caution.
- This profile is intended for trusted operators only.
```

**Tools granted (20):** All 20 built-in tools + all dynamically registered tools (Google, MCP, plugins, agent control, memory tools).

**Risk assessment:** Maximum. Should only be available through explicit operator authorization.

---

### 4.3 Profile comparison matrix

| Feature | safe-agent | code-agent | system-agent | admin-agent |
|---------|-----------|------------|-------------|-------------|
| Read-only tools | 12 | 8 (subset) | 8 (subset) | 8 (subset) |
| Write tools | 0 | 3 (file_write, bash_exec, apply_patch) | 3 | 3 |
| L2 tools | 0 | 0 | 2 (ebpf_compile, module_build) | 2 |
| L3 tools | 0 | 0 | 2 (module_load, kernel_build) | 2 |
| Web tools | web_search | web_search, web_fetch | web_search, web_fetch | web_search, web_fetch |
| Task tools | all 4 | all 4 | all 4 | all 4 |
| Max iterations | 10 | 25 | 30 | 50 |
| Max tool calls | 128 (default) | 128 (default) | 128 (default) | 128 (default) |
| Approval required | None | L3 auto | L3 auto | None |
| Dynamic tools | No | No | No | Yes (Google, MCP, plugins, agent) |

## 5. Tool activation mechanism

### 5.1 Current path (unchanged in Phase 1)

The activation path is already functional and needs NO code changes for Phase 1:

```text
1. AgentLoader::load_from_dir() reads agents/*.md files
   -> parses YAML frontmatter into AgentRole { name, tools, model, ... }
   -> at crates/executive/src/impl/agent_loader/mod.rs:55-76

2. load_agent_profiles() resolves tool names against catalog
   -> builds AgentProfile { id, system_prompt, model, allowed_tools, limits }
   -> at crates/executive/src/impl/daemon/bootstrap/runtime.rs:11-69

3. AgentProfileRegistry stores resolved profiles
   -> used by AgentTool for sub-agent spawning

4. RequestHandler::new() registers all tools in ToolRegistry::default()
   -> at crates/executive/src/impl/daemon/bootstrap/request.rs:293-296

5. discover_tool_extensions() builds ExtensionDescriptor list
   -> at crates/corpus/src/tools/capability_executor.rs:43-69

6. ExtensionGrant carries capabilities per-session
   -> at crates/corpus/src/service.rs:22-29

7. Tool definition filtering happens at runtime based on allowed_tools
   -> at various points in the execution pipeline
```

**Phase 1 requires zero code changes.** Adding tool names to the `tools:` field in `agents/*.md` frontmatter is sufficient to grant them. The `AgentLoader` already supports comma-separated tool names in YAML frontmatter at `mod.rs:121-131`.

### 5.2 Proposed improvements (Phases 2-3)

#### 5.2.1 AgentProfile struct extension

The existing `AgentProfile` at `crates/fabric/src/types/agent_control.rs:84-95` should be extended with profile metadata fields:

```rust
// Proposed extension (non-breaking additions)
pub struct AgentProfile {
    // Existing fields (unchanged)
    pub id: AgentProfileId,
    pub system_prompt: String,
    pub model: String,
    pub allowed_tools: Vec<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,

    // New fields (Phase 2)
    pub profile_name: String,           // "code-agent", "safe-agent", etc.
    pub risk_tier: RiskTier,            // ReadOnly, Sandboxed, System, Unrestricted
    pub approval_policy: ApprovalPolicy, // AutoDeny, PromptUser, AutoApprove
    pub tool_timeout_ms: u64,           // per-tool-call timeout
    pub inheritable: bool,              // can child agents inherit this profile
    pub parent_restriction: ParentRestriction, // same-or-safer tier enforcement
}
```

#### 5.2.2 Profile config in AppConfig

Add an `agent_profiles` section to `AppConfig` at `crates/executive/src/core/config/mod.rs`:

```toml
[agent_profiles]
default = "code-agent"

[agent_profiles.overrides.safe-agent]
max_iterations = 15
tool_timeout_ms = 30000

[agent_profiles.overrides.admin-agent]
approval_policy = "prompt_user"
```

This would be parsed into a new config struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfilesConfig {
    pub default: String,
    pub overrides: HashMap<String, ProfileOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileOverride {
    pub max_iterations: Option<usize>,
    pub max_tool_calls: Option<u32>,
    pub tool_timeout_ms: Option<u64>,
    pub approval_policy: Option<ApprovalPolicy>,
}
```

#### 5.2.3 Runtime profile switching

Add an admin command to switch the active profile at runtime:

```rust
// Proposed new RPC endpoint in handler/rpc/rpc_admin.rs
async fn set_agent_profile(
    &self,
    session_id: String,
    profile_name: String,
) -> Result<ProfileSwitchResult, RpcError>;
```

This would:
1. Resolve the named profile from AgentProfileRegistry
2. Validate the profile against current session constraints
3. Update the session's ExtensionGrant to reflect new allowed capabilities
4. Emit an event to the event spine for audit

#### 5.2.4 Parent-child profile enforcement

When a parent agent spawns a child agent (via AgentTool), enforce that the child's profile is strictly no-more-capable than the parent:

```rust
impl ParentRestriction {
    pub fn allows_child(&self, parent: &AgentProfile, child: &AgentProfile) -> bool {
        // Child must not have tools the parent doesn't
        for tool in &child.allowed_tools {
            if !parent.allowed_tools.contains(tool) {
                return false;
            }
        }
        // Child risk tier must be <= parent risk tier
        child.risk_tier <= parent.risk_tier
    }
}
```

## 6. Staged activation plan

### 6.1 Phase 1: Safe read-only tools (immediate, config-only)

**Changes:** Edit `agents/code-agent.md` frontmatter only. No Rust code changes.

Add to `code-agent.md` tools list:

```yaml
tools: file_read, file_write, bash_exec, glob, grep, file_search, code_graph,
       web_search, web_fetch, task_create, task_update, task_list, task_get
```

Create new agent profile files:

- `agents/safe-agent.md` -- read-only tools only
- `agents/system-agent.md` -- code-agent + L2/L3 tools
- `agents/admin-agent.md` -- all tools

**Justification:** These 8 additional tools (glob, grep, file_search, code_graph, web_search, web_fetch, and 4 task tools) are:

- **Already implemented, tested, and registered** in ToolRegistry::default()
- **Read-only or sandbox-safe** (web_fetch/web_search are L1 but sandbox-gated)
- **Enormous utility gain** -- code_graph gives AST analysis, file_search is ripgrep-powered, task tools give structured planning
- **Zero code risk** -- only YAML frontmatter changes

**Commit message:**
```
feat(agents): activate read-only and task tools in default agent profiles

Add glob, grep, file_search, code_graph, web_search, web_fetch, and
task_create/task_update/task_list/task_get to the default code-agent
profile. Add safe-agent (read-only), system-agent (L2/L3), and
admin-agent (unrestricted) profiles.

All tools are already registered in ToolRegistry::default() and pass
unit tests. This is a config-only change -- no Rust code modified.
```

**Validation:**
- `cargo test -p corpus -- tools::tools::registry` -- verify default registry unchanged
- `cargo test -p executive -- agent_loader` -- verify AgentLoader parses new profiles
- Daemon starts with `--agent-profile code-agent` and `tools/list` shows all 14 tools

### 6.2 Phase 2: Structured AgentProfile system (3 days)

**Changes:** New Rust types + config parsing + profile registry extension.

#### Day 1: Core types

1. Add profile metadata to `AgentProfile` at `crates/fabric/src/types/agent_control.rs:84-95`:
   - `profile_name: String`
   - `risk_tier: RiskTier` (new enum: ReadOnly, Sandboxed, System, Unrestricted)
   - `approval_policy: ApprovalPolicy` (new enum: AutoDeny, PromptUser, AutoApprove)
   - `tool_timeout_ms: u64`
   - `inheritable: bool`
   - Add `validate()` checks for new fields

2. Add `AgentProfilesConfig` to `crates/executive/src/core/config/mod.rs`:
   - `default: String`
   - `overrides: HashMap<String, ProfileOverride>`
   - Add to `AppConfig` as `pub agent_profiles: AgentProfilesConfig`

#### Day 2: Profile resolution and config integration

3. Extend `load_agent_profiles()` in `crates/executive/src/impl/daemon/bootstrap/runtime.rs:11-69`:
   - Accept `AgentProfilesConfig` parameter
   - Apply overrides (max_iterations, tool_timeout, etc.) during profile construction
   - Set `risk_tier` based on `allowed_tools` permission levels
   - Set `profile_name` from the AgentRole name
   - Log profile details at info level during startup

4. Add `AgentProfileRegistry::resolve_by_name()` method:
   - Look up profile by name string
   - Return resolved `AgentProfile` or error if not found

#### Day 3: Config files and validation

5. Create example config in `config/agent_profiles.toml`:
   ```toml
   [agent_profiles]
   default = "code-agent"

   [agent_profiles.overrides.safe-agent]
   max_iterations = 15
   tool_timeout_ms = 30000

   [agent_profiles.overrides.admin-agent]
   approval_policy = "prompt_user"
   ```

6. Add integration tests at `crates/executive/src/impl/daemon/bootstrap/runtime.rs` (extend existing test module):
   - Test profile loading with overrides
   - Test risk tier assignment
   - Test missing profile error handling
   - Test override validation

**Commit messages:**
```
feat(fabric): add profile metadata fields to AgentProfile

Add RiskTier, ApprovalPolicy, tool_timeout_ms, inheritable, and
profile_name fields to the AgentProfile struct. These enable tiered
profile definitions with explicit safety gating.
```
```
feat(executive): add AgentProfilesConfig and profile override system

Add AgentProfilesConfig to AppConfig with default profile selection
and per-profile overrides for iteration limits, timeouts, and
approval policy. Extend load_agent_profiles() to apply overrides
during profile construction.
```
```
test(executive): add integration tests for agent profile loading

Test profile resolution, risk tier assignment, override application,
and error handling for missing or invalid profiles.
```

### 6.3 Phase 3: Runtime profile switching (2 days)

**Changes:** RPC endpoint + session management + event spine integration.

#### Day 1: Core switching mechanism

1. Add `set_agent_profile` RPC method in `crates/executive/src/impl/daemon/handler/rpc/rpc_admin.rs`:
   - Accept `session_id` and `profile_name`
   - Resolve profile from AgentProfileRegistry
   - Validate: child profile cannot exceed parent capabilities
   - Update SessionGateway with new profile
   - Rebuild tool definitions for the session
   - Return new profile details

2. Add parent-child enforcement in `AgentProfileRegistry`:
   - `validate_child_profile(parent: &AgentProfile, child: &AgentProfile) -> Result<()>`
   - Ensures child tool set is a subset of parent tool set
   - Ensures child risk tier <= parent risk tier

3. Emit event to event spine on profile switch:
   ```rust
   canonical_event_spine.emit(ProfileSwitchEvent {
       session_id,
       previous_profile: String,
       new_profile: String,
       timestamp: clock.wall_now().0,
   });
   ```

#### Day 2: CLI + TUI integration

4. Add `--agent-profile` CLI flag to interactive client:
   - Override default profile per session
   - Example: `aletheon chat --agent-profile safe-agent`

5. Add `:profile` TUI command:
   - `:profile list` -- show available profiles
   - `:profile switch <name>` -- change active profile
   - `:profile current` -- show current profile info

6. Add integration tests for the full switch lifecycle.

**Commit messages:**
```
feat(executive): add runtime agent profile switching via RPC

Add set_agent_profile admin RPC endpoint that switches the active
profile for a session, enforcing parent-child capability constraints
and emitting events to the canonical event spine.
```
```
feat(interact): add agent profile selection to CLI and TUI

Add --agent-profile CLI flag for per-session profile override and
:profile TUI command for listing, switching, and inspecting active
agent profiles.
```

## 7. Quick wins -- activate immediately (Phase 1 actions)

These files need creation or modification. All changes are YAML frontmatter only.

### 7.1 Replace `agents/code-agent.md`

The existing file is a shell (`code-agent.toml` exists but `code-agent.md` may or may not). Create the `.md` file with extended tools.

### 7.2 Create `agents/safe-agent.md`

Read-only profile for untrusted input or safe exploration.

### 7.3 Create `agents/system-agent.md`

System profile for kernel/eBPF work.

### 7.4 Create `agents/admin-agent.md`

Admin profile for trusted operators.

### 7.5 Verify: `agents/net-agent.md` and `agents/fs-agent.md`

The existing `net-agent.md` (if any) and `fs-agent.md` should be updated to reference actual registered tool names (currently `net-agent.toml` lists `system_status` and `process_list` which are correct).

## 8. Tools needing hardening before activation

### 8.1 apply_patch (L1, 771 LOC)

**Status:** Large implementation with diff parsing, hunk application, conflict detection.
**Gap:** No structured patch format enforcement. The tool accepts free-form diff input.
**Required before default activation:** Validate that patches conform to unified diff format, enforce hunk context lines, reject ambiguous patches. This is tracked in the separate code-editing plan.
**Recommendation:** Include in code-agent profile but with a warning in the system prompt that patches must be well-formed unified diffs. The tool already has dry-run capability.

### 8.2 ebpf_compile (L2, 248 LOC)

**Status:** Compiles BPF bytecode from C source. Does not load.
**Gap:** No BPF verifier pass before returning. The compilation produces an artifact but does not check if the kernel verifier would accept it.
**Required before activation:** Add optional BPF verifier pass (`bpftool prog load` check) or clearly document that verification happens at load time.
**Recommendation:** Include in system-agent profile. The tool is read-only in effect (produces .o file, does not load).

### 8.3 module_load (L3, 292 LOC)

**Status:** Loads/unloads kernel modules via insmod/rmmod.
**Gap:** No signature verification. No kernel taint check before loading. Destructive on failure.
**Required before default activation:** Add kernel taint check before loading, verify module signatures when available, require explicit user confirmation in non-admin profiles.
**Recommendation:** Include only in admin-agent and system-agent profiles. Always require user approval.

### 8.4 kernel_build (L3, 541 LOC)

**Status:** Full kernel build + install pipeline with bootloader update.
**Gap:** Irreversible operation. Bootloader update can render system unbootable.
**Required before default activation:** Add pre-install checks (available disk space, boot partition mount, existing kernel backup), dry-run mode, rollback plan.
**Recommendation:** Include only in admin-agent profile. Always require user approval and confirm with a challenge-response.

### 8.5 bash_exec (L1, 116 LOC)

**Status:** Solid but test report noted 25% error rate on complex commands.
**Gap:** Output bounding, error classification, sandbox escape risk.
**Required before full activation:** Output size limits, timeout enforcement hardening, sandbox integration verification. This is tracked in the separate tool-execution-hardening plan.
**Recommendation:** Already in default profile. Accept current risk for now.

### 8.6 system_status and process_list (L0, 71+70 LOC)

**Status:** Basic implementations. Return full output without bounding.
**Gap:** No output truncation. `system_status` dumps all env vars (can be 100KB+). `process_list` returns full `ps aux` output.
**Required before activation:** Add output size limits (e.g., 64KB max) with truncation indicators in `ToolResultMeta`.
**Recommendation:** Include in code-agent profile with output bounding. Simple to implement: truncate `ToolResult.content` at 64KB and set `metadata.truncated = true`.

## 9. Implementation timeline

| Phase | Duration | Files | Risk | Dependencies |
|-------|----------|-------|------|-------------|
| Phase 1: Safe activation | 1 day | 4 .md files | None | None |
| Phase 2: Profile system | 3 days | fabric/agent_control.rs, executive/config/mod.rs, executive/daemon/bootstrap/runtime.rs | Low | None |
| Phase 3: Runtime switching | 2 days | executive/handler/rpc/rpc_admin.rs, interact/ CLI/TUI | Low | Phase 2 |
| **Total** | **6 days** | **~10 files** | | |

### Phase 1 detailed task list

1. **Read existing agent files:**
   - `agents/code-agent.md` (if exists)
   - `agents/fs-agent.md` (if exists)
   - `agents/net-agent.md` (if exists)

2. **Modify `agents/code-agent.md`:** Replace tools list with 14 tools.

3. **Create `agents/safe-agent.md`:** 12 read-only tools.

4. **Create `agents/system-agent.md`:** All 20 tools.

5. **Create `agents/admin-agent.md`:** All 20 tools + dynamic tool note.

6. **Delete or archive legacy `.toml` files:** `agents/code-agent.toml`, `agents/fs-agent.toml`, `agents/net-agent.toml` -- these are not read by `AgentLoader` which only scans `*.md` files. Archive as `agents/legacy/` if needed for reference.

7. **Verify:** Start daemon, check that `agent_tools` list includes all expected tools.

### Phase 2 detailed task list

1. Add `RiskTier` and `ApprovalPolicy` enums to `crates/fabric/src/types/agent_control.rs`.
2. Extend `AgentProfile` struct with new fields.
3. Add `AgentProfilesConfig` to `crates/executive/src/core/config/mod.rs`.
4. Add `ProfileOverride` struct.
5. Add `agent_profiles` field to `AppConfig`.
6. Modify `load_agent_profiles()` to accept and apply config overrides.
7. Add `AgentProfileRegistry::resolve_by_name()`.
8. Add profile resolution tests (extend existing test module at `runtime.rs:182-355`).
9. Add config parsing tests.
10. Add override application tests.
11. Update daemon bootstrap in `request.rs` to pass `AgentProfilesConfig`.

### Phase 3 detailed task list

1. Add `ProfileSwitchEvent` type.
2. Add `set_agent_profile` method to `AdminUseCases` trait.
3. Implement `set_agent_profile` in `AdminService`.
4. Add parent-child profile validation.
5. Emit event to canonical event spine on switch.
6. Add `--agent-profile` CLI flag.
7. Add `:profile` TUI command (list, switch, current).
8. Add integration tests for profile switch lifecycle.
9. Test with sub-agent spawning to verify capability inheritance.

## 10. Acceptance criteria

### Phase 1

- [ ] `cargo test -p corpus -- tools::tools::registry` passes
- [ ] `cargo test -p executive -- agent_loader` passes with new profiles
- [ ] Daemon starts and `tools/list` RPC returns 14+ tools for code-agent
- [ ] `tools/list` returns 12 tools for safe-agent
- [ ] `tools/list` returns 20 tools for system-agent and admin-agent
- [ ] No tool name errors in daemon logs at startup
- [ ] Existing legacy `.toml` files archived, not referenced by AgentLoader

### Phase 2

- [ ] `AgentProfile` serializes/deserializes with new fields
- [ ] `AgentProfilesConfig` parses from TOML correctly
- [ ] Config overrides apply correctly (e.g., max_iterations override)
- [ ] Missing profile reference produces clear error at startup
- [ ] Profile resolution by name works for all defined profiles
- [ ] `cargo test -p executive -- agent_loader` covers new behavior
- [ ] `cargo test -p fabric -- agent_profile_validate` covers new validation rules

### Phase 3

- [ ] `set_agent_profile("safe-agent")` restricts tools to L0 only
- [ ] `set_agent_profile("admin-agent")` from safe-agent is denied (escalation)
- [ ] `set_agent_profile("safe-agent")` from admin-agent is allowed (restriction)
- [ ] Profile switch event appears in event spine
- [ ] `--agent-profile safe-agent` CLI flag restricts tools on that session
- [ ] `:profile switch system-agent` in TUI changes tool list
- [ ] Child agent spawned from code-agent cannot use admin-agent profile
- [ ] Integration test covers full switch -> verify -> revert cycle

## 11. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Agent gets confused by too many tools | Medium | Low | Phase 1 only adds 8 tools. System prompt explicitly guides tool selection. |
| bash_exec used for exploration instead of grep/glob | Medium | Low | System prompt instructs preference for dedicated tools over bash_exec. |
| web_fetch/web_search incur network costs | Low | Medium | web_fetch has timeout; web_search is rate-limited by provider. Add per-session web tool budget in Phase 2. |
| module_load in system-agent causes kernel panic | Low | High | Only available in system-agent and admin-agent. System prompt requires confirmation. Add kernel taint check in hardening phase. |
| apply_patch corrupts files with malformed input | Medium | Medium | Tool has conflict detection. Add patch format validation before full activation. |
| Profile escalation (child getting more than parent) | Low | High | ParentRestriction enforcement in Phase 3 prevents this. |
| Legacy .toml files cause confusion | Low | Low | Archive to agents/legacy/ with README explaining migration to .md format. |

## 12. References

- `crates/corpus/src/tools/tools/registry.rs:109-184` -- ToolRegistry::default() with all 20 tool registrations
- `crates/fabric/src/types/tool.rs:20-29` -- PermissionLevel enum (L0-L3)
- `crates/fabric/src/types/agent_control.rs:84-95` -- AgentProfile struct
- `crates/executive/src/impl/agent_loader/mod.rs:12-26` -- AgentRole struct, 55-76 -- load_from_dir()
- `crates/executive/src/impl/daemon/bootstrap/runtime.rs:11-69` -- load_agent_profiles()
- `crates/executive/src/impl/daemon/bootstrap/request.rs:293-528` -- daemon tool registration and profile loading
- `crates/executive/src/core/config/mod.rs:41-58` -- AppConfig struct
- `crates/executive/src/core/config/agent.rs:19-54` -- ExecutiveConfig
- `crates/corpus/src/tools/capability_executor.rs:43-69` -- discover_tool_extensions()
- `crates/corpus/src/service.rs:22-29` -- ExtensionGrant struct
- `crates/fabric/src/security/policy.rs:102-108` -- SecurityPolicy tool level inference
- `agents/code-agent.toml` -- current default profile (3 tools)
- `agents/fs-agent.toml` -- file system agent profile (2 tools)
- `agents/net-agent.toml` -- network agent profile (2 tools)
