# Aletheon Tool Execution Hardening Plan

> **Status:** Proposed
>
> **Target branch:** `dev`
>
> **Aletheon baseline:** `e807e41`
>
> **Scope:** all tool execution paths in `crates/corpus/src/` and the sandbox pipeline in `crates/corpus/src/security/`; new `crates/exec-server/` binary crate for Phase 2
>
> **Execution rule:** each phase ships as a standalone PR that can be tested, gated, and rolled back independently. No phase depends on the next to be production-safe.

## 1. Executive conclusion

Aletheon's tool execution pipeline has a single sandbox dispatch point -- `ToolRunnerWithGuard::execute_tool_inner` at `crates/corpus/src/security/runner.rs:369` -- but only one tool path (`bash_exec`) goes through the sandbox. Every other tool (file_write, apply_patch, web_fetch, web_search, git operations, kernel builds, and the remaining 20+ structured tools) calls `tool.execute()` directly at runner.rs:414, bypassing all sandbox isolation. The comment at line 407-411 acknowledges this explicitly: "Path-mutating tools enforce canonical workspace confinement in their own implementation." This is a false reassurance -- confinement is a necessary but not sufficient property of sandboxing. Tools that spawn child processes (apply_patch runs `patch` via `Command::new`, runner.rs:90) or make network requests (web_fetch, web_search use `reqwest::Client` directly) also need process isolation, network policy enforcement, and resource limits.

The target state is:

```text
one sandbox dispatch point that routes ALL tool execution
one SandboxProfile per tool type describing allowed paths / network / resources
one exec-server child process for all command/IO execution with proper process lifecycle
one network policy engine with per-command allow/deny rules
one shell-escape detector with configurable warn/block behavior
```

The recommended order is: Phase 1 (universal sandbox wrapping) first because it closes the largest gap with the least new surface area, then Phase 2 (exec-server process isolation) for defense-in-depth, then Phase 3 (escape detection + network policy) for layered hardening.

## 2. Current state: sources of truth

This section anchors every claim to a specific file and line number. If the code drifts after this plan is written, the line numbers serve as audit anchors, not guarantees.

### 2.1. Sandbox dispatch gate: only bash_exec

**`crates/corpus/src/security/runner.rs:365-431`**

```rust
// 3. Shell commands execute through the command sandbox. Structured
// tools have no command string to sandbox; running an empty command
// made their first execution fail validation and then performed the
// real side effect as an unsandboxed validation retry.
let result = if tool_name == "bash_exec" {
    // ... sandbox.run(cmd, &sandbox_config, Duration::from_secs(30)) ...
} else {
    // Structured tools execute through their implementation with a
    // bounded timeout. Path-mutating tools enforce canonical workspace
    // confinement in their own implementation.
    const TOOL_TIMEOUT_SECS: u64 = 60;
    tool.execute(input.clone(), ctx)  // <-- unsandboxed
};
```

The `sandbox_config` built at runner.rs:373-380 contains only `working_dir` and `env_vars`. There is no allowed_paths, network policy, or resource limits in the config.

### 2.2. bash_exec: direct bash spawn, no exec-server

**`crates/corpus/src/tools/tools/bash_exec.rs:56-60`**

```rust
Command::new("bash")
    .arg("-c")
    .arg(command)
    .current_dir(&ctx.working_dir)
    .output(),
```

The tool spawns bash directly via `tokio::process::Command`. There is no separate exec-server process. Timeout is applied via `SystemTimer::timeout` but there is no process-group management, no SIGTERM-before-SIGKILL grace period, and no streaming output.

### 2.3. file_write: direct fs::write, no sandbox

**`crates/corpus/src/tools/tools/file_write.rs:50-78`**

```rust
let full_path = match validate_mutation_path(&ctx.working_dir, std::path::Path::new(path)) {
    Ok(path) => path, Err(error) => return ToolResult { ... }
};
fs::write(&full_path, content).await
```

`validate_mutation_path` (defined at `crates/corpus/src/tools/tools/mutation_path.rs:8`) provides workspace confinement and protected-component rejection, but the actual write happens in-process via `tokio::fs::write`. There is no resource limit on write size, no sandbox process wrapping, and no audit trail of the full written content (only a summary in the audit log).

### 2.4. apply_patch: spawns `patch` command unsandboxed

**`crates/corpus/src/tools/tools/apply_patch.rs:142-213`**

```rust
Command::new("patch")
    .arg("-p1")
    .arg("--directory")
    .arg(base_dir)
    .arg("--input")
    .arg(&tmp_patch)
    .output()
    .await;
```

The `apply_via_patch_command` function spawns `/usr/bin/patch` as a subprocess, writes temp files to the workspace, and removes them. None of this goes through the sandbox. The fallback `apply_patch_native` (line 256-311) performs direct `tokio::fs` read/write/remove operations. Both paths bypass `SandboxExecutor::run`. The only defense is `validate_mutation_path` at lines 74-87 for path confinement.

### 2.5. web_fetch / web_search: direct reqwest, no network policy

**`crates/corpus/src/tools/tools/web_fetch.rs:80-168`**

```rust
let client = reqwest::Client::new();
// ... client.get(&url) / client.post(&url) ...
request.send().await
```

**`crates/corpus/src/tools/tools/web_search.rs:96-111`**

```rust
let client = reqwest::Client::new();
client.post(&api_url).header(...).json(&body).send().await
```

Both tools create an unrestricted `reqwest::Client` with no network policy, no URL allow/deny list, no protocol restrictions, and no sandbox process wrapping. Any URL can be fetched; any POST body can be sent.

### 2.6. Sandbox backend chain

**`crates/corpus/src/security/sandbox/executor.rs:14-40`**

```rust
// Priority order: Bubblewrap > Process > Noop
if let Some(bwrap) = BubblewrapBackend::probe(clock.clone()) {
    backends.push(Box::new(bwrap));
}
backends.push(Box::new(ProcessBackend { clock: clock.clone() }));
backends.push(Box::new(NoopBackend { clock }));
```

The `ProcessBackend` (`crates/corpus/src/security/sandbox/process.rs`) also calls `bash -c cmd` directly (line 63) -- it has the same no-isolation properties as the bash_exec tool itself. It provides resource limits via RLIMIT but no filesystem or network isolation.

### 2.7. SandboxConfig: minimal

**`crates/fabric/src/types/sandbox.rs:26-33`**

```rust
pub struct SandboxConfig {
    pub working_dir: String,
    pub env_vars: HashMap<String, String>,
}
```

No fields for: allowed paths, network policy, resource limits (memory, CPU), seccomp profile, or capture policy. The richer `SandboxProfile` at `crates/corpus/src/security/sandbox/profile.rs` exists with `read_roots`, `write_roots`, `deny_paths`, and `network_enabled`, but it is not wired into `SandboxConfig` or consumed by any execution path.

### 2.8. Complete audit of tool sandbox coverage

| Tool | Permission | Sandboxed? | Spawns subprocess? | Network? |
|---|---|---|---|---|
| bash_exec | L1 | YES (runner.rs:369) | bash directly | No |
| file_write | L1 | NO | No | No |
| file_read | L0 | NO | No | No |
| apply_patch | L1 | NO | `patch` command | No |
| web_fetch | L1 | NO | No | `reqwest::Client` |
| web_search | L1 | NO | No | `reqwest::Client` |
| grep | L0 | NO | No | No |
| glob | L0 | NO | No | No |
| file_search | L0 | NO | No | No |
| code_graph | L0 | NO | No | No |
| agent_control | L0 | NO | No | No |
| agent_tool | L0 | NO | No | No |
| task_tools | L0 | NO | No | No |
| script_tool | L1 | NO | Yes (script engine) | Possible |
| ebpf_compile | L1 | NO | Yes (compiler) | No |
| kernel_build | L1 | NO | Yes (make) | No |
| module_build | L1 | NO | Yes (make) | No |
| module_load | L2 | NO | Yes (insmod) | No |
| system_status | L0 | NO | No | No |
| process_list | L0 | NO | No | No |

Only 1 of ~20 tools goes through the sandbox.

## 3. Phase 1: Universal sandbox wrapping (1 PR, ~400 loc)

**Goal:** Every tool that mutates filesystem state or spawns subprocesses goes through the sandbox dispatch. Read-only tools are exempted by policy, not by code path.

**Risk:** Low. The sandbox backends already work for bash_exec. Phase 1 extends the same pattern to other tool categories.

### 3.1. Design

Replace the `if tool_name == "bash_exec"` gate at runner.rs:369 with a tool-category-based dispatch that classifies tools into sandbox profiles and routes them accordingly. The classification lives in the `SandboxProfile` type (already exists at `crates/corpus/src/security/sandbox/profile.rs`) but must be extended.

#### 3.1.1. New types in Fabric

Add to `crates/fabric/src/types/sandbox.rs`:

```rust
/// Per-tool sandbox configuration for the runner.
#[derive(Debug, Clone)]
pub struct SandboxProfileConfig {
    /// Directories the tool may read.
    pub read_roots: Vec<PathBuf>,
    /// Directories the tool may write.
    pub write_roots: Vec<PathBuf>,
    /// Paths explicitly denied (highest priority).
    pub deny_paths: Vec<PathBuf>,
    /// Whether network access is permitted.
    pub network_enabled: bool,
    /// Extra environment variables.
    pub env_vars: Vec<(String, String)>,
    /// Timeout override for this tool (None = use runner default).
    pub timeout_override: Option<Duration>,
    /// Maximum output bytes before truncation.
    pub max_output_bytes: usize,
}

/// Defines how the runner should execute a tool.
#[derive(Debug, Clone)]
pub enum ToolExecutionStrategy {
    /// Execute the tool command string through the sandbox.
    Sandboxed { config: SandboxProfileConfig },
    /// Execute in-process with workspace confinement only (read-only tools).
    InProcess,
    /// Execute via the network proxy (web_fetch, web_search).
    NetworkProxied { allowed_domains: Vec<String> },
    /// Tool requires exec-server (Phase 2); fall back to Sandboxed for now.
    ExecServerRequired { config: SandboxProfileConfig },
}
```

Extend `SandboxConfig`:

```rust
pub struct SandboxConfig {
    pub working_dir: String,
    pub env_vars: HashMap<String, String>,
    // NEW FIELDS
    pub profile: Option<SandboxProfileConfig>,
}
```

#### 3.1.2. Strategy resolver

Add to `crates/corpus/src/security/` a new module `strategy.rs`:

```rust
/// Resolve the execution strategy for a given tool name.
pub fn resolve_strategy(tool_name: &str, permission_level: PermissionLevel) -> ToolExecutionStrategy {
    match tool_name {
        "bash_exec" => ToolExecutionStrategy::Sandboxed {
            config: SandboxProfileConfig {
                write_roots: vec![],  // filled at runtime from ctx.working_dir
                network_enabled: false,
                timeout_override: Some(Duration::from_secs(30)),
                max_output_bytes: 1_048_576, // 1MB
                ..Default::default()
            },
        },
        "file_write" | "apply_patch" => ToolExecutionStrategy::Sandboxed {
            config: SandboxProfileConfig {
                write_roots: vec![],  // filled at runtime
                network_enabled: false,
                timeout_override: None, // 60s default
                max_output_bytes: 1_048_576,
                ..Default::default()
            },
        },
        "web_fetch" | "web_search" => ToolExecutionStrategy::NetworkProxied {
            allowed_domains: vec![], // filled from config
        },
        "ebpf_compile" | "kernel_build" | "module_build" | "module_load" => {
            ToolExecutionStrategy::Sandboxed {
                config: SandboxProfileConfig {
                    write_roots: vec![],
                    network_enabled: false,
                    timeout_override: Some(Duration::from_secs(300)), // build tools get 5min
                    max_output_bytes: 4_194_304, // 4MB
                    ..Default::default()
                },
            }
        },
        "script_tool" => ToolExecutionStrategy::Sandboxed {
            config: SandboxProfileConfig {
                write_roots: vec![],
                network_enabled: false,
                timeout_override: None,
                max_output_bytes: 1_048_576,
                ..Default::default()
            },
        },
        // Read-only tools: in-process is fine
        _ => ToolExecutionStrategy::InProcess,
    }
}
```

#### 3.1.3. Modified runner dispatch

Replace runner.rs:365-431 with:

```rust
// 3. Route to the appropriate execution strategy.
let strategy = resolve_strategy(tool_name, tool.permission_level());

let result = match &strategy {
    ToolExecutionStrategy::Sandboxed { config } => {
        let mut sandbox_config = SandboxConfig {
            working_dir: ctx.working_dir.to_string_lossy().to_string(),
            env_vars: HashMap::from([...]),
            profile: Some(config.clone().with_runtime_roots(&ctx.working_dir)),
        };
        let timeout = config.timeout_override.unwrap_or(Duration::from_secs(60));
        match self.sandbox.run_with_tool(tool, input, &sandbox_config, timeout).await {
            Ok(sandbox_result) => ToolResult { ... },
            Err(e) => ToolResult { ... },
        }
    }
    ToolExecutionStrategy::NetworkProxied { allowed_domains } => {
        // Verify URL/payload against network policy before sending
        self.validate_network_request(tool_name, &input, allowed_domains)?;
        // Execute within sandbox (so network is intercepted by bwrap --unshare-net)
        let sandbox_config = SandboxConfig {
            working_dir: ctx.working_dir.to_string_lossy().to_string(),
            env_vars: HashMap::new(),
            profile: Some(SandboxProfileConfig { network_enabled: true, .. }),
        };
        // ...
    }
    ToolExecutionStrategy::InProcess => {
        // Read-only tools: existing direct execution is fine
        tool.execute(input.clone(), ctx).await
    }
    ToolExecutionStrategy::ExecServerRequired { .. } => {
        // Phase 2: fall back to Sandboxed until exec-server is available
        // (recurse with Sandboxed strategy)
    }
};
```

#### 3.1.4. SandboxExecutor extension

Add a `run_with_tool` method to `SandboxExecutor` at `crates/fabric/src/types/sandbox.rs` that serializes the tool input and executes it through the sandbox. For Phase 1, the implementation can use a helper binary or the existing tool's JSON input/execute contract wrapped inside the sandbox process.

```rust
impl SandboxExecutor {
    /// Execute a tool through the sandbox. The tool is serialized as a JSON
    /// command that the sandbox infrastructure can deserialize and execute
    /// inside the isolated environment.
    pub async fn run_with_tool(
        &self,
        tool: &dyn Tool,
        input: serde_json::Value,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult>;
}
```

For Phase 1, the concrete implementation wraps the existing `run` method and uses a sandbox-internal tool dispatch script that reads JSON from stdin, executes the tool, and writes the result to stdout. This avoids requiring a new binary while still routing through sandbox backends.

### 3.2. Acceptance criteria

- [ ] `file_write` calls go through the sandbox pipeline (verified by audit log showing `sandbox_backend` set)
- [ ] `apply_patch` calls go through the sandbox pipeline
- [ ] Sandbox backends (bubblewrap, process, noop) are exercised for tool types beyond bash_exec
- [ ] Read-only tools (file_read, grep, glob, file_search, code_graph) continue to execute in-process without sandbox overhead
- [ ] `SandboxProfile` is consumed by `SandboxConfig` and enforced by at least the bubblewrap backend
- [ ] Existing tests in `runner.rs` pass; new tests cover file_write and apply_patch through the sandbox path
- [ ] Canary test: verify that a file_write outside the workspace is blocked by the sandbox (not just by `validate_mutation_path`)

### 3.3. Commit message

```
feat(corpus): route all mutating tools through the sandbox pipeline

Replace the bash_exec-only sandbox gate in ToolRunnerWithGuard with a
tool-category-based strategy resolver (SandboxProfile + ToolExecutionStrategy).
file_write, apply_patch, ebpf_compile, kernel_build, module_build, module_load,
and script_tool now go through the sandbox. Read-only tools remain in-process.

Introduces SandboxProfileConfig (fabric), ToolExecutionStrategy (fabric),
and resolve_strategy (corpus). Backward-compatible: existing behavior is
unchanged for tools that were already sandboxed (bash_exec) or read-only.
```

## 4. Phase 2: Exec-server process isolation (2 PRs, ~1200 loc)

**Goal:** All command execution and filesystem I/O goes through a separate OS process with reduced privileges, proper process lifecycle management, and output streaming.

**Risk:** Medium. Introduces a new binary crate and a new IPC protocol. Must be feature-gated and tested in CI before it becomes the default path.

### 4.1. Architecture

```
Aletheon Daemon
  |
  +-- spawns exec-server child process on startup
  |     |
  |     +-- exec-server binary (new crate: crates/exec-server/)
  |           |
  |           +-- Unix socket or stdin/stdout JSON-RPC transport
  |           +-- reduced privilege (separate user/pid namespace via bwrap)
  |           +-- process/start, process/signal, process/terminate
  |           +-- fs/read, fs/write, fs/readDir, fs/metadata, fs/walk
  |           +-- all paths restricted to workspace + allowed roots
  |
  +-- ToolRunnerWithGuard
        |
        +-- routes writes/executions through exec-server adapter
        +-- falls back to in-process when exec-server is unavailable
```

### 4.2. New crate: `crates/exec-server/`

```
crates/exec-server/
  Cargo.toml
  src/
    main.rs            -- entry point, JSON-RPC over stdin/stdout
    protocol.rs        -- request/response types + JSON-RPC framing
    process.rs         -- process lifecycle (spawn, signal, terminate)
    filesystem.rs       -- sandboxed filesystem operations
    sandbox.rs          -- sandbox enforcement (profile checks)
```

#### 4.2.1. Protocol: JSON-RPC over stdin/stdout

The exec-server reads JSON-RPC requests from stdin (one per line) and writes JSON-RPC responses to stdout. This avoids the complexity of Unix socket permissions and is compatible with bwrap's `--unshare-net` (which breaks Unix sockets in some configurations).

**Authentication:** The daemon writes a shared secret to stdin as the first message. The exec-server validates it and responds with a handshake confirmation. If the secret does not match, the exec-server exits immediately.

```rust
/// Request envelope.
#[derive(Debug, Serialize, Deserialize)]
struct ExecServerRequest {
    id: u64,
    method: String,
    params: serde_json::Value,
}

/// Response envelope.
#[derive(Debug, Serialize, Deserialize)]
struct ExecServerResponse {
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<ExecServerError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExecServerError {
    code: i32,
    message: String,
    data: Option<serde_json::Value>,
}
```

**RPC methods:**

| Method | Params | Returns | Description |
|---|---|---|---|
| `handshake` | `{ secret: string }` | `{ version: "1.0" }` | Authenticate the connection |
| `process/start` | `{ command, argv, cwd, env, timeout_ms, sandbox_profile, network_policy }` | `{ pid: u32, handle: string }` | Spawn a process |
| `process/read` | `{ handle: string, stream: "stdout" \| "stderr", max_bytes: usize }` | `{ data: string, eof: bool }` | Read buffered output |
| `process/write` | `{ handle: string, data: string }` | `{ bytes_written: usize }` | Write to stdin |
| `process/signal` | `{ handle: string, signal: "SIGTERM" \| "SIGKILL" }` | `{}` | Send signal to process group |
| `process/terminate` | `{ handle: string }` | `{ exit_code: i32, stdout: string, stderr: string, elapsed_ms: u64 }` | SIGTERM, wait 500ms, SIGKILL, collect output |
| `fs/read` | `{ path: string }` | `{ content: string, size: usize }` | Read entire file (capped at 100MB) |
| `fs/readChunk` | `{ handle: string, offset: usize, size: usize }` | `{ data: string, eof: bool }` | Read a chunk from open handle |
| `fs/write` | `{ path: string, content: string }` | `{ bytes_written: usize }` | Write file (sandbox-restricted) |
| `fs/list` | `{ path: string }` | `{ entries: [{ name: string, kind: "file" \| "dir" }] }` | List directory |
| `fs/metadata` | `{ path: string }` | `{ size: u64, modified: string, is_file: bool, is_dir: bool }` | File metadata |
| `fs/walk` | `{ path: string, max_depth: usize }` | `{ files: [{ path: string, ... }] }` | Recursive walk |
| `fs/remove` | `{ path: string }` | `{}` | Delete file/directory |
| `fs/copy` | `{ source: string, dest: string }` | `{}` | Copy file/directory |
| `fs/open` | `{ path: string, mode: "read" \| "write" \| "append" }` | `{ handle: string }` | Open file handle |
| `fs/close` | `{ handle: string }` | `{}` | Close file handle |
| `shutdown` | `{}` | `{}` | Graceful shutdown |

#### 4.2.2. Process lifecycle management

**`crates/exec-server/src/process.rs`:**

```rust
/// A managed process handle inside the exec-server.
struct ManagedProcess {
    child: tokio::process::Child,
    handle: String,
    stdout_buffer: Vec<u8>,
    stderr_buffer: Vec<u8>,
    stdout_eof: bool,
    stderr_eof: bool,
    started_at: tokio::time::Instant,
    timeout: Option<Duration>,
    sandbox_profile: SandboxProfileConfig,
    network_policy: Option<NetworkPolicy>,
    /// Process group ID for tree-kill.
    pgid: Option<u32>,
}

/// Spawn a process, create a process group, and start concurrent stdout/stderr
/// readers that fill bounded buffers (1MB per stream).
async fn start_process(req: ProcessStartRequest) -> Result<ProcessStartResponse>;

/// Send SIGTERM to the process group. After `grace_ms` (default 500ms),
/// send SIGKILL to any surviving processes.
async fn terminate_process(handle: &str) -> Result<ProcessTerminateResponse>;

/// Drain the stdout/stderr buffers and return the collected output.
async fn collect_output(handle: &str, max_bytes: usize) -> Result<CollectedOutput>;
```

**`ExecExpiration` type (in Fabric):**

```rust
#[derive(Debug, Clone)]
pub enum ExecExpiration {
    /// Hard timeout after N milliseconds.
    Timeout(u64),
    /// Cancelled by external signal.
    Cancellation,
    /// Either timeout or cancellation, whichever comes first.
    TimeoutOrCancellation { timeout_ms: u64 },
}
```

**`ExecRequest` type (in Fabric):**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub expiration: ExecExpiration,
    pub capture_policy: CapturePolicy,
    pub sandbox_profile: SandboxProfileConfig,
    pub network_policy: Option<NetworkPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapturePolicy {
    /// Capture both stdout and stderr, capped at max_bytes each.
    Full { max_bytes_per_stream: usize },
    /// Stream output back as delta events.
    Streaming { max_total_bytes: usize },
    /// Discard output entirely (for fire-and-forget commands).
    None,
}
```

#### 4.2.3. Filesystem protocol

All filesystem operations in the exec-server enforce the sandbox profile. Before any `fs/*` operation, the server checks:

1. The target path is within `write_roots` (for writes) or `read_roots \cup write_roots` (for reads).
2. The target path is not in `deny_paths`.
3. Symlink traversal is resolved and checked against the profile.
4. Operations outside permitted paths return error code `-32002` (sandbox denial).

File handles support up to 128 concurrent open handles. Chunked reads support streaming large files without buffering the entire content in memory. Maximum file size for reads is 100MB (configurable).

### 4.3. Daemon integration

**`crates/executive/src/impl/daemon/server.rs`** -- the daemon spawns the exec-server on startup:

```rust
fn spawn_exec_server(config: &DaemonConfig) -> Result<ExecServerHandle> {
    let child = tokio::process::Command::new(&config.exec_server_binary_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;
    // Authenticate with shared secret
    // Return handle for subsequent RPC calls
}
```

**`crates/executive/src/impl/channel/daemon_adapter.rs`** -- an `ExecServerClient` wraps the stdin/stdout transport:

```rust
pub struct ExecServerClient {
    stdin: tokio::io::BufWriter<ChildStdin>,
    stdout: tokio::io::BufReader<ChildStdout>,
    pending: HashMap<u64, oneshot::Sender<ExecServerResponse>>,
}
```

### 4.4. Acceptance criteria

- [ ] `crates/exec-server/` compiles as a standalone binary
- [ ] Handshake + authentication works (wrong secret = rejection)
- [ ] `process/start` spawns a subprocess with proper process-group handling
- [ ] `process/terminate` sends SIGTERM, waits 500ms, then SIGKILL
- [ ] Concurrent stdout/stderr readers fill bounded buffers (no OOM on infinite output)
- [ ] `process/read` returns buffered chunks, reports eof
- [ ] All `fs/*` methods enforce the sandbox profile (path confinement + deny list)
- [ ] `fs/write` refuses to write outside workspace
- [ ] `fs/read` refuses to read outside allowed roots
- [ ] `fs/walk` respects max_depth
- [ ] Symlink traversal is resolved before profile check (no TOCTOU via symlinks)
- [ ] 128 concurrent file handles work without deadlock
- [ ] Daemon spawns exec-server on startup and reconnects on crash
- [ ] Feature-gated: `--exec-server` CLI flag to enable; falls back to Phase 1 behavior when disabled
- [ ] Integration test: bash_exec through exec-server produces identical output to in-process execution

### 4.5. Commit messages

**PR 1 (new crate + protocol):**
```
feat(exec-server): introduce exec-server binary with JSON-RPC protocol

New crate crates/exec-server/ implements a child process that handles:
- process/start, process/read, process/write, process/signal, process/terminate
- fs/read, fs/readChunk, fs/open, fs/close, fs/write, fs/list, fs/metadata,
  fs/walk, fs/remove, fs/copy
- handshake authentication via shared secret

All operations enforce sandbox profile (path confinement, deny list).
Process lifecycle: SIGTERM grace period (500ms) before SIGKILL.
Concurrent stdout/stderr readers with bounded buffers (1MB per stream).
128 concurrent file handles. 100MB max file size.

Introduces ExecRequest, ExecExpiration, CapturePolicy in Fabric.
```

**PR 2 (daemon integration):**
```
feat(executive): integrate exec-server into daemon lifecycle

Daemon spawns exec-server on startup (feature-gated behind --exec-server).
ExecServerClient wraps stdin/stdout JSON-RPC transport.
ToolRunnerWithGuard routes tool execution through exec-server adapter
when available, falling back to Phase 1 sandbox path when unavailable.

Integration test verifies bash_exec produces identical output through
both paths.
```

## 5. Phase 3: Shell escape detection and network policy (1 PR, ~500 loc)

**Goal:** Detect and block (or warn on) shell escape attempts; enforce per-command network policy.

**Risk:** Low. These are additive security layers that can be toggled warn-only in production and tightened over time.

### 5.1. Shell escape detection

Add `crates/corpus/src/security/escape_detector.rs`:

```rust
/// Detects potential shell escape / sandbox bypass patterns in command strings.
pub struct ShellEscalationDetector {
    /// If true, suspicious commands are blocked. If false, they are warned.
    mode: ShellEscapeMode,
    /// Additional custom patterns beyond the built-in set.
    custom_patterns: Vec<Regex>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEscapeMode {
    /// Log a warning but allow execution.
    WarnOnly,
    /// Block execution and return an error.
    Block,
}

#[derive(Debug, Clone)]
pub struct ShellEscapeVerdict {
    pub allowed: bool,
    pub matched_patterns: Vec<String>,
    pub severity: ShellEscapeSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEscapeSeverity {
    Low,
    Medium,
    High,
}

impl ShellEscalationDetector {
    /// Built-in detection patterns:
    /// - Heredocs: `<<` with delimiter (potential file injection)
    /// - Exec wrappers: `exec ` prefix (overwrites current process)
    /// - Eval: `eval ` (dynamic code execution)
    /// - Subshell with side effects: `$(...)` or backticks
    /// - Process substitution: `<(...)` or `>(...)`
    /// - Chained command separators beyond `&&` / `||`: `;` (command injection)
    /// - Reverse shell patterns: `/dev/tcp/`, `nc -e`, `python -c socket`
    /// - Privilege escalation: `sudo`, `su`, `pkexec`
    /// - /proc filesystem manipulation: `/proc/sys/`, `/proc/self/mem`
    pub fn builtin_patterns() -> Vec<(Regex, ShellEscapeSeverity, &'static str)>;

    /// Scan a command string and return a verdict.
    pub fn scan(&self, command: &str) -> ShellEscapeVerdict;
}
```

**Integration point:** `ToolRunnerWithGuard::execute_tool_inner`, after policy check but before sandbox execution. For `bash_exec` commands and any tool with `Sandboxed` strategy, scan the command string.

### 5.2. Network policy

Add to `crates/fabric/src/types/sandbox.rs`:

```rust
/// Network policy for a specific command or tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Hostnames or IP ranges that are explicitly allowed.
    /// If non-empty, only these hosts may be contacted.
    pub allow_hosts: Vec<String>,
    /// Hostnames or IP ranges that are explicitly denied (highest priority).
    pub deny_hosts: Vec<String>,
    /// Allowed protocols (e.g. ["tcp", "udp"]). If empty, no restriction.
    pub allow_protocols: Vec<String>,
    /// Allowed port ranges (e.g. ["80", "443", "8080-8090"]).
    /// If empty, no port restriction.
    pub allow_ports: Vec<String>,
    /// Whether DNS resolution is permitted.
    pub allow_dns: bool,
}

impl NetworkPolicy {
    /// Check whether a URL is permitted under this policy.
    pub fn allows_url(&self, url: &str) -> Result<(), String>;
}
```

**Integration points:**

1. **bash_exec with network:** The sandbox config sets `network_enabled: false` by default. If the user requests network access in a bash_exec command, the command must be explicitly approved. The bubblewrap backend enforces via `--unshare-net` (which creates an isolated network namespace). When network is enabled, a proxy or iptables rules enforce the `NetworkPolicy`.

2. **web_fetch / web_search:** Before sending the HTTP request, `validate_network_request` checks the URL against the `NetworkPolicy`. In Phase 1, this is an in-process check. In Phase 2, the exec-server enforces it.

3. **Seccomp filter:** The bubblewrap backend can inject a seccomp BPF filter that denies `socket()`, `connect()`, and `bind()` syscalls when `network_enabled: false`. This is a future enhancement listed here for completeness.

### 5.3. Acceptance criteria

- [ ] `ShellEscalationDetector` detects heredocs, exec wrappers, eval, subshell escapes, and reverse shells
- [ ] `ShellEscapeMode::Block` prevents execution; `ShellEscapeMode::WarnOnly` logs and continues
- [ ] Integration with `ToolRunnerWithGuard` -- escape detection runs before sandbox execution
- [ ] `NetworkPolicy::allows_url` correctly handles allow_hosts, deny_hosts, and protocol filters
- [ ] `web_fetch` and `web_search` validate URLs against policy before sending
- [ ] `bash_exec` with `network_enabled: true` requires explicit approval
- [ ] Configuration: shell escape detection mode and default network policy are configurable via aletheon.toml or environment variables

### 5.4. Commit message

```
feat(corpus): add shell escape detection and network policy enforcement

ShellEscalationDetector: detects heredocs, exec wrappers, eval, subshell
escapes, reverse shells, and privilege escalation patterns. Configurable
mode (warn-only or block).

NetworkPolicy: per-command network access control with allow_hosts,
deny_hosts, allow_protocols, and allow_ports. Integrated into web_fetch,
web_search, and bash_exec (when network is enabled). Bubblewrap backend
enforces via --unshare-net.

Both features are additive: default behavior is warn-only for escape
detection and allow-all for network, preserving backward compatibility.
```

## 6. Security invariants

The following invariants must hold after each phase. Violating any of them is a blocking regression.

### Phase 1

1. **No tool mutates filesystem outside `validate_mutation_path` confinement.** The sandbox must enforce the same path restrictions that `validate_mutation_path` provides, plus any additional `SandboxProfile` restrictions.
2. **Read-only tools do not incur sandbox overhead.** The `InProcess` strategy continues to use `tool.execute()` directly.
3. **All existing bash_exec behavior is preserved.** The sandbox config, timeout, and output handling must be identical.

### Phase 2

4. **The exec-server runs as a separate OS process with its own PID namespace.** It cannot access the parent daemon's memory.
5. **Path confinement is enforced server-side, not client-side.** The exec-server validates all paths against its sandbox profile before performing any operation. A compromised daemon cannot trick the exec-server into writing outside the workspace.
6. **Process tree is killed on timeout.** SIGTERM to the process group, 500ms grace, then SIGKILL. No orphaned child processes.
7. **Output is bounded.** No unbounded buffer growth. Max 1MB per stream, streaming delta events if the daemon requests them.

### Phase 3

8. **Shell escape detection cannot be disabled by the agent.** The detection mode is set by the operator and enforced at the runner level, not by tool input.
9. **Network policy is applied before the first byte of a request is sent.** No "speculative" network access.

## 7. Files changed

### Phase 1

| File | Change |
|---|---|
| `crates/fabric/src/types/sandbox.rs` | Add `SandboxProfileConfig`, `ToolExecutionStrategy`, extend `SandboxConfig` |
| `crates/corpus/src/security/strategy.rs` | New: `resolve_strategy` function |
| `crates/corpus/src/security/runner.rs` | Replace bash_exec-only gate with strategy dispatch |
| `crates/corpus/src/security/sandbox/executor.rs` | Add `run_with_tool` method |
| `crates/corpus/src/security/sandbox/profile.rs` | Wire `SandboxProfile` into `SandboxProfileConfig` |
| `crates/corpus/src/security/mod.rs` | Export `strategy` module |
| `crates/corpus/src/security/sandbox/process.rs` | Accept and apply `SandboxProfileConfig` from `SandboxConfig` |
| `crates/corpus/src/security/sandbox/bubblewrap.rs` | Accept and apply `SandboxProfileConfig` via bwrap args |

### Phase 2

| File | Change |
|---|---|
| `crates/exec-server/Cargo.toml` | New crate manifest |
| `crates/exec-server/src/main.rs` | Entry point, JSON-RPC loop over stdin/stdout |
| `crates/exec-server/src/protocol.rs` | Request/response types |
| `crates/exec-server/src/process.rs` | ManagedProcess, spawn/signal/terminate |
| `crates/exec-server/src/filesystem.rs` | Sandboxed filesystem operations |
| `crates/exec-server/src/sandbox.rs` | Profile enforcement |
| `crates/fabric/src/types/sandbox.rs` | Add `ExecRequest`, `ExecExpiration`, `CapturePolicy`, `NetworkPolicy` |
| `crates/fabric/src/types/mod.rs` | Export new types |
| `crates/executive/src/impl/daemon/server.rs` | Spawn exec-server on startup |
| `crates/executive/src/impl/channel/daemon_adapter.rs` | `ExecServerClient` for RPC transport |
| `crates/executive/Cargo.toml` | Add optional dependency on exec-server protocol types |
| `Cargo.toml` (workspace) | Add `exec-server` to workspace members |

### Phase 3

| File | Change |
|---|---|
| `crates/corpus/src/security/escape_detector.rs` | New: `ShellEscalationDetector` |
| `crates/fabric/src/types/sandbox.rs` | Add `NetworkPolicy` type |
| `crates/corpus/src/security/runner.rs` | Integrate escape detection before sandbox execution |
| `crates/corpus/src/tools/tools/web_fetch.rs` | Validate URL against network policy |
| `crates/corpus/src/tools/tools/web_search.rs` | Validate URL against network policy |
| `crates/corpus/src/security/mod.rs` | Export `escape_detector` module |

## 8. Rollout plan

| Phase | PR | Gating mechanism | Rollback |
|---|---|---|---|
| 1 | Single PR | CI + existing test suite + new sandbox coverage tests | Revert PR (no new binary, no config change) |
| 2a | exec-server crate + protocol | Feature gate `--exec-server` (default off) | Disable flag, no code change |
| 2b | Daemon integration | Same feature gate; integration test in CI | Disable flag |
| 3 | Single PR | Shell escape: warn-only by default; network: allow-all by default | Revert PR (all new behavior behind config flags) |

## 9. References

### Codex patterns consulted

- `exec-server` binary: separate process, Noise protocol over WebSocket, 12 FS methods, 128 concurrent handles, chunked streaming
- `ExecParams`: command, cwd, expiration (Timeout/Cancellation/TimeoutOrCancellation), capture_policy, env, network, sandbox_permissions
- `consume_output()`: concurrent stdout/stderr readers, capped buffers, streaming delta events, SIGTERM (50ms) before SIGKILL
- `ShellEscalationDetector`: heredocs, exec wrappers, subshell escapes

### Aletheon codebase anchors

| Symbol | Path | Line |
|---|---|---|
| `ToolRunnerWithGuard` | `crates/corpus/src/security/runner.rs` | 54 |
| `execute_tool_inner` | `crates/corpus/src/security/runner.rs` | 204 |
| sandbox gate `if tool_name == "bash_exec"` | `crates/corpus/src/security/runner.rs` | 369 |
| unsandboxed `tool.execute()` | `crates/corpus/src/security/runner.rs` | 414 |
| `BashExecTool::execute` | `crates/corpus/src/tools/tools/bash_exec.rs` | 47 |
| `Command::new("bash")` | `crates/corpus/src/tools/tools/bash_exec.rs` | 56 |
| `FileWriteTool::execute` | `crates/corpus/src/tools/tools/file_write.rs` | 45 |
| `ApplyPatchTool::execute` | `crates/corpus/src/tools/tools/apply_patch.rs` | 46 |
| `apply_via_patch_command` | `crates/corpus/src/tools/tools/apply_patch.rs` | 142 |
| `WebFetchTool::execute` | `crates/corpus/src/tools/tools/web_fetch.rs` | 52 |
| `WebSearchTool::execute` | `crates/corpus/src/tools/tools/web_search.rs` | 45 |
| `SandboxConfig` | `crates/fabric/src/types/sandbox.rs` | 26 |
| `SandboxExecutor` | `crates/fabric/src/types/sandbox.rs` | 141 |
| `SandboxProfile` | `crates/corpus/src/security/sandbox/profile.rs` | 10 |
| `create_default_executor` | `crates/corpus/src/security/sandbox/executor.rs` | 14 |
| `ProcessBackend::execute` | `crates/corpus/src/security/sandbox/process.rs` | 47 |
| `validate_mutation_path` | `crates/corpus/src/tools/tools/mutation_path.rs` | 8 |
