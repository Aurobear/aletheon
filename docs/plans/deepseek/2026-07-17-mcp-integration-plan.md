# Aletheon MCP Integration Plan

> **Status:** Proposed
>
> **Target branch:** `dev`
>
> **Aletheon baseline:** `e807e41`
>
> **Scope:** MCP (Model Context Protocol) server management, tool discovery, tool
> execution, resource access, authentication, and daemon integration
>
> **Key sources:** `crates/corpus/src/tools/mcp/`, `crates/cognit/src/config/mod.rs`,
> `crates/executive/src/impl/daemon/bootstrap/request.rs`, local Codex source

## 1. Executive summary

Aletheon already has a working MCP client implementation in
`crates/corpus/src/tools/mcp/`. It supports three transports (Stdio,
StreamableHttp, SSE), bearer-token and OAuth authentication, tool discovery,
and registration into the daemon's `ToolRegistry`. MCP tools are available to
the model at `crates/executive/src/impl/daemon/bootstrap/request.rs:393-417`.

What remains is hardening, gap closure, and contract unification. The current
implementation has two competing `McpServerConfig` types, no per-server tool
filtering, no resource access, no elicitation support, no health-check
reconnection loop, and no notification-driven tool-list refresh.

This plan documents:

1. What exists today (grounded in source inspection).
2. What gaps remain.
3. A phased implementation sequence, each phase a self-contained PR.

## 2. Current state: what already exists

### 2.1 Module layout

```
crates/corpus/src/tools/mcp/
  mod.rs          -- pub mod re-exports; pub use manager::McpManager
  config.rs       -- McpConfig, McpServerConfig (internal), McpTransportConfig, McpTrustLevel
  client.rs       -- McpClient, McpConnectionManager, McpTool
  manager.rs      -- McpManager (facade for daemon)
  wrapper.rs      -- McpToolWrapper (Tool trait impl)
  transport.rs    -- McpTransport enum (Stdio/StreamableHttp/Sse), ToolNameConfig, CollisionStrategy
  auth.rs         -- BearerTokenAuth, McpOAuthProvider, OAuthEndpoints
  token_store.rs  -- TokenEntry, TokenKey, TokenStore

crates/cognit/src/config/
  mod.rs:664      -- McpServerConfig (TOML-facing, simplified)
  mod.rs:737      -- GbrainMemoryConfig (tightly coupled to MCP server lookup)
```

### 2.2 Transport architecture

`McpTransport` (`transport.rs:24`) is an enum with three variants:

- **Stdio** -- spawns a subprocess, communicates via stdin/stdout JSON-RPC lines.
  A tokio writer task pushes lines to stdin; a reader task reads lines from
  stdout and sends them over an mpsc channel.
- **StreamableHttp** -- HTTP POST with SSE response parsing. Falls back to SSE
  on non-auth connection failures via `connect_with_fallback()` (`transport.rs:526`).
- **Sse** -- HTTP GET long-poll to `<base_url>/sse` plus POST requests.

All variants implement the same JSON-RPC 2.0 request/response pattern
(`request()`, `notify()` at `transport.rs:315,371`). Auth errors (401/403)
are detected and propagated without fallback (`is_auth_error()` at `transport.rs:173`).

The HTTP response body is bounded to 1 MiB (`read_bounded_http_body`
at `transport.rs:474`).

### 2.3 Client and connection manager

`McpClient` (`client.rs:22`) owns one transport and one tool list per server:

```rust
pub struct McpClient {
    pub server_name: String,
    transport: McpTransport,
    next_id: u64,
    pub trust_level: McpTrustLevel,
    pub tools: Vec<McpTool>,
}
```

It has three constructors -- `connect_stdio`, `connect_http`, `connect_sse` --
each performing the MCP initialize handshake (`"protocolVersion": "2024-11-05"`)
followed by `tools/list` discovery.

`McpConnectionManager` (`client.rs:216`) holds a `HashMap<String, Arc<Mutex<McpClient>>>`
keyed by server name. `connect_all()` iterates over config, connects each
enabled server, and inserts successful connections. Failures are logged as
warnings -- a single failed server does not block others.

Tool-isError responses are detected (`client.rs:202-210`) and turned into
errors without leaking the server-provided error text (which might contain
credentials or request context).

### 2.4 Tool wrapper

`McpToolWrapper` (`wrapper.rs:12`) implements the `Tool` trait:

```rust
impl Tool for McpToolWrapper {
    fn name(&self) -> &str { &self.normalized_name }
    fn description(&self) -> &str { &self.mcp_tool.description }
    fn input_schema(&self) -> Value { self.mcp_tool.input_schema.clone() }
    fn permission_level(&self) -> PermissionLevel {
        match self.trust_level {
            McpTrustLevel::LocalTrusted => PermissionLevel::L0,
            McpTrustLevel::RemoteTrusted => PermissionLevel::L1,
            McpTrustLevel::Untrusted => PermissionLevel::L2,
        }
    }
    fn boxed_clone(&self) -> Box<dyn Tool> { /* ... */ }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult { /* ... */ }
}
```

On execution, it acquires the client mutex, calls `call_tool`, and serializes
the JSON response as pretty-printed text. Errors are surfaced as `is_error: true`
with the error message.

### 2.5 Tool name normalization

`ToolNameConfig` with `CollisionStrategy` (`transport.rs:49-137`) controls how
tool names from different servers are disambiguated:

- `PrefixServer` -- `<server>__<tool>` (default)
- `NumericSuffix` -- `<tool>`, `<tool>_2`, ...
- `FirstWins` -- first server claims the name; later duplicates are skipped

Names are truncated to `max_length` (default 64) with UTF-8-safe truncation
via `fabric::truncate_utf8_bytes`.

### 2.6 Authentication

Two providers, both implementing the `McpAuth` trait:

- `BearerTokenAuth` (`auth.rs:68`) -- reads token from an environment variable
  at request time (no caching, picks up runtime changes). Token values are
  redacted in `Display`/`Debug`.
- `McpOAuthProvider` (`auth.rs:187`) -- full OAuth 2.0 Authorization Code
  flow with PKCE, CSRF state validation (10-minute expiry), token persistence
  via `TokenStore`, and access-token refresh. Reuses the
  `corpus::tools::google::oauth::AsyncOAuthClient` infrastructure.

`TokenStore` (`token_store.rs`) persists `TokenEntry` records to a JSON file,
keyed by `TokenKey`. It also serves as the persistence layer for Google OAuth
tokens (`google/oauth.rs:4`).

### 2.7 Config

There are two `McpServerConfig` types:

1. **cognit** (`cognit/src/config/mod.rs:664`) -- string-based, TOML-facing:

```rust
pub struct McpServerConfig {
    pub name: String,
    pub transport: String,          // "stdio", "http", or "sse"
    pub command: Option<String>,
    pub url: Option<String>,
    pub bearer_token_env: Option<String>,
}
```

2. **corpus::mcp** (`corpus/src/tools/mcp/config.rs:23`) -- enum-based, internal runtime:

```rust
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,  // Stdio { command, args } | StreamableHttp { url } | Sse { url }
    pub trust: McpTrustLevel,
    pub enabled: bool,
    pub bearer_token_env: Option<String>,
}
```

The conversion from cognit (TOML) to corpus (runtime) is performed inline
during `McpManager::new()` construction from `config.mcp_servers` -- see
`executive/src/impl/daemon/bootstrap/request.rs:385`.

`AppConfig.mcp_servers` (`executive/src/core/config/mod.rs:47`) is typed
as `Vec<cognit::config::McpServerConfig>`. It is re-exported through
`executive/src/core/config/infra.rs:6`.

### 2.8 Daemon bootstrap integration

The bootstrap sequence at `executive/src/impl/daemon/bootstrap/request.rs:382-417`:

1. Constructs `McpConfig` from `config.mcp_servers`.
2. Creates `McpManager` and calls `connect_all()`.
3. Calls `mcp.tool_wrappers()` to get `Box<dyn Tool>` instances.
4. Registers each wrapper into the `ToolRegistry`.
5. If `gbrain_memory.enabled`, retains the manager as `retained_mcp` for
   GBrain recall/capture calls (which use the same authenticated connections).

### 2.9 Test coverage

The `manager.rs` tests (`:67-527`) cover:

- Empty config connect/discovery (3 tests)
- Unknown server error propagation
- Bearer token env var resolution and auth header transmission
- End-to-end: HTTP connect -> tool discovery -> tool call + result verification
- Tool-level `isError` handling (error text not leaked)
- HTTP 401 handling (server not connected, token not leaked in errors)
- Config serialization roundtrip for `bearer_token_env`
- Default `bearer_token_env` is `None`
- Disabled server is not connected

The `transport.rs` tests (`:564-750`) cover:

- All three collision strategies with expected output
- Tool name truncation (byte-count, UTF-8 safety)
- Notification parsing (`tools/list_changed`, unknown, request-not-notification)
- `is_auth_error` detection (401, 403, other)
- Collision strategy serialization roundtrip
- `ToolNameConfig` defaults

The `auth.rs` tests (`:424-871`) cover:

- BearerTokenAuth: env var read, empty/missing env, display redaction
- `McpAuth` trait: `get_headers`, `is_expired`, `refresh`
- TokenStore: save/load roundtrip, remove
- Token expiry detection
- OAuth: authorize_url contains required params, CSRF rejection, double-consume rejection
- OAuth: `is_expired` with stored tokens, `get_headers` uses stored token, empty headers when expired
- `parse_token_response` full/minimal
- `purge_expired_states`

## 3. Gaps and remaining work

### 3.1 Duplicate `McpServerConfig` types (HIGH)

Two structs with the same name, different shapes. The cognit version is the
TOML surface; the corpus version is the runtime type. The conversion is
currently implicit -- callers copy fields by hand. This is fragile and will
break silently when fields are added to one but not the other.

**Resolution:** Unify into a single type. The cognit type should be the source
of truth for TOML deserialization. The corpus runtime should either use it
directly or have an explicit `From<McpServerConfig>` conversion.

### 3.2 No per-server tool filtering (HIGH)

There is no allowlist or denylist mechanism. Every tool discovered from an MCP
server is registered unconditionally. For untrusted or third-party MCP servers,
a user must be able to restrict which tools are exposed to the model.

**Resolution:** Add `allowlist` and `denylist` fields to `McpServerConfig`:

```toml
[[mcp_servers]]
name = "external-search"
transport = "http"
url = "https://search.internal/mcp"
allowlist = ["search", "lookup"]
# Only "search" and "lookup" are registered; others are silently skipped.
```

### 3.3 No resource access (MEDIUM)

The MCP protocol defines `resources/list`, `resources/read`, and
`resources/templates/list`. None of these are implemented. Aletheon tools
cannot read MCP resources or subscribe to resource updates.

**Resolution:** Implement resource discovery and read. Expose resources through
a `McpResourceProvider` that can be queried by tools or the model directly.

### 3.4 No elicitation support (MEDIUM)

MCP servers can request user approval via `elicitation/create`. The current
implementation does not handle elicitation requests. If an MCP server sends
one, it is silently dropped or causes an error.

**Resolution:** Wire MCP elicitation through Aletheon's existing approval
system (`corpus::security::socket_approval::SocketApprovalGate` and the
approval repository at `approval_repository` in bootstrap).

### 3.5 No health-check reconnection loop (MEDIUM)

`McpConfig::health_check_interval_sec` (`config.rs:8`) is defined but never
used. There is no background task that periodically pings connected servers or
reconnects on failure. If an MCP server crashes after the initial connect, its
tools remain in the registry but return errors on every call.

**Resolution:** Add a background health-check task that:
- Periodically sends `ping` or re-runs `tools/list` on each connected server.
- On failure, marks the server as disconnected and removes its tools from the
  registry (or flags them as unavailable).
- On recovery, re-discovers tools and re-registers them.

### 3.6 No `tools/list_changed` notification handling (LOW)

`McpNotification::ToolsListChanged` is parsed (`transport.rs:161`) but never
acted upon. If an MCP server dynamically adds or removes tools, the daemon
does not refresh its tool list.

**Resolution:** On receipt of `tools/list_changed`, re-query `tools/list` and
update the `McpClient.tools` list. Notify the tool registry of additions/removals.

### 3.7 Permission level overrides not configurable (LOW)

The mapping from `McpTrustLevel` to `PermissionLevel` is hardcoded in
`McpToolWrapper::permission_level()`:

```rust
McpTrustLevel::LocalTrusted  => PermissionLevel::L0,  // Read-only
McpTrustLevel::RemoteTrusted => PermissionLevel::L1,  // Write within sandbox
McpTrustLevel::Untrusted     => PermissionLevel::L2,  // System-level changes
```

A user cannot override the permission level of a specific MCP tool. A
RemoteTrusted server's tools are all L1, even if one tool is read-only.

Also note: the mapping seems inverted. `Untrusted` should produce the *most
restrictive* level (L0 or L1), not L2 (system-level changes). This is a
potential security issue.

**Resolution:** Add per-tool permission overrides in config. Fix the
`Untrusted` mapping to be more restrictive.

### 3.8 No `supports_parallel_tool_calls` detection (LOW)

The MCP initialize response includes a `capabilities` object that can declare
`supports_parallel_tool_calls`. This is not inspected. If a server supports
parallel calls, Aletheon's `ConcurrencyClass` system could schedule them
concurrently.

**Resolution:** Read `supports_parallel_tool_calls` from the initialize
response and set `ConcurrencyClass::ReadOnly` on tools from servers that
support parallelism.

### 3.9 GBrain coupling (LOW)

`GbrainMemoryConfig` at `cognit/src/config/mod.rs:737` is tightly coupled to
MCP concept (`server_name` field is used to look up an MCP client). This
config should be generalized so that any memory backend can use MCP tools
without being named "gbrain".

**Resolution:** Generalize the memory-to-MCP bridge so that any MCP server
can provide memory tools. Keep backward compatibility for the gbrain config key.

## 4. Implementation phases

### Phase 1 -- Unification and hardening (1 PR)

**Goal:** Unify `McpServerConfig` types, add tool filtering, fix permission
mapping, add health checks.

Scope:
- `crates/cognit/src/config/mod.rs` -- extend `McpServerConfig` with
  `allowlist`, `denylist`, `permission_overrides`, `trust` (enum), `enabled`,
  `args` (for stdio)
- `crates/corpus/src/tools/mcp/config.rs` -- remove the duplicate
  `McpServerConfig`, add `From<cognit::McpServerConfig>` conversion
- `crates/corpus/src/tools/mcp/client.rs` -- apply tool filtering in
  `get_all_tools()`, add health-check loop
- `crates/corpus/src/tools/mcp/wrapper.rs` -- fix `Untrusted` mapping to L1
  (sandboxed), add per-tool override support
- `crates/executive/src/impl/daemon/bootstrap/request.rs` -- simplify
  MCP bootstrap to use unified config

**Commit message:**
```
feat(mcp): unify config types, add tool filtering and health checks

- Merge cognit McpServerConfig and corpus McpServerConfig into one type
- Add allowlist/denylist tool filtering per server
- Add per-tool permission level overrides
- Fix Untrusted trust level mapping (L2 -> L1)
- Add background health-check task with configurable interval
- Wire health_check_interval_sec from McpConfig

Co-Authored-By: Claude <noreply@anthropic.com>
```

Verification:
```bash
cargo test -p aletheon-corpus mcp
cargo test -p aletheon-executive --test integration
cargo build --workspace
```

### Phase 2 -- Resources and notifications (1 PR)

**Goal:** Implement MCP resource access and `tools/list_changed` handling.

Scope:
- `crates/corpus/src/tools/mcp/client.rs` -- add `list_resources()`,
  `read_resource()`, `list_resource_templates()` to `McpClient`
- `crates/corpus/src/tools/mcp/manager.rs` -- expose
  `get_resources()`, `read_resource()` on `McpManager`
- `crates/corpus/src/tools/mcp/transport.rs` -- handle
  `tools/list_changed` notification by re-discovering tools
- `crates/corpus/src/tools/mcp/wrapper.rs` -- create
  `McpResourceProvider` as a tool that can read MCP resources
- New pure-rust tools in `crates/corpus/src/tools/tools/`:
  `mcp_resource_read` -- read a named MCP resource from a server

**Commit message:**
```
feat(mcp): add resource access and tools/list_changed handling

- Implement resources/list, resources/read, resources/templates/list
- Add McpResourceProvider tool for reading MCP resources
- Handle tools/list_changed notification with tool re-discovery
- Expose resources through McpManager API

Co-Authored-By: Claude <noreply@anthropic.com>
```

Verification:
```bash
cargo test -p aletheon-corpus mcp
cargo build --workspace
```

### Phase 3 -- Elicitation and parallel tool calls (1 PR)

**Goal:** Wire MCP elicitation through Aletheon's approval system. Detect and
use `supports_parallel_tool_calls`.

Scope:
- `crates/corpus/src/tools/mcp/client.rs` -- handle `elicitation/create`
  responses, detect `supports_parallel_tool_calls` from initialize
- `crates/corpus/src/tools/mcp/manager.rs` -- route elicitation to
  approval system
- `crates/corpus/src/tools/mcp/wrapper.rs` -- set `ConcurrencyClass::ReadOnly`
  on tools from servers that support parallel calls
- `crates/executive/src/impl/daemon/bootstrap/request.rs` -- wire
  elicitation approval to `SocketApprovalGate` and `ApprovalRepository`

**Commit message:**
```
feat(mcp): add elicitation support and parallel tool call detection

- Route MCP elicitation/create through Aletheon approval system
- Detect supports_parallel_tool_calls from MCP initialize response
- Set ConcurrencyClass::ReadOnly for tools on parallel-capable servers

Co-Authored-By: Claude <noreply@anthropic.com>
```

Verification:
```bash
cargo test -p aletheon-corpus mcp
cargo test -p aletheon-executive --test integration
cargo build --workspace
```

### Phase 4 -- Streamable HTTP hardening and OAuth polish (1 PR)

**Goal:** Harden the StreamableHttp transport, add OAuth discovery, and
generalize the GBrain memory bridge.

Scope:
- `crates/corpus/src/tools/mcp/transport.rs` -- add connection pooling,
  retry with exponential backoff, timeout configuration
- `crates/corpus/src/tools/mcp/auth.rs` -- add OAuth metadata discovery
  (RFC 8414), support `client_secret_basic` and `client_secret_post`
- `crates/cognit/src/config/mod.rs` -- generalize `GbrainMemoryConfig`
  to `McpMemoryConfig` with backward-compatible alias
- `crates/executive/src/impl/daemon/bootstrap/request.rs` -- use
  generalized memory-MCP bridge

**Commit message:**
```
feat(mcp): harden StreamableHttp, polish OAuth, generalize memory bridge

- Add connection pooling and retry with exponential backoff for HTTP
- Add OAuth metadata discovery (RFC 8414)
- Generalize GbrainMemoryConfig to McpMemoryConfig with backwards compat
- Add request timeout configuration per server

Co-Authored-By: Claude <noreply@anthropic.com>
```

Verification:
```bash
cargo test -p aletheon-corpus mcp
cargo test -p aletheon-executive
cargo build --workspace
```

## 5. Config TOML examples

### Current (already working):

```toml
[[mcp_servers]]
name = "gbrain"
transport = "http"
url = "https://gbrain.internal/mcp"
bearer_token_env = "GBRAIN_READ_TOKEN"
```

### After Phase 1:

```toml
[[mcp_servers]]
name = "external-search"
transport = "http"
url = "https://search.internal/mcp"
bearer_token_env = "SEARCH_TOKEN"
enabled = true
trust = "RemoteTrusted"
allowlist = ["search", "lookup"]
denylist = ["delete_index"]

[[mcp_servers]]
name = "local-tools"
transport = "stdio"
command = "/usr/local/bin/mcp-server"
args = ["--config", "/etc/mcp/server.toml"]
trust = "LocalTrusted"

[[mcp_servers]]
name = "safety-checker"
transport = "sse"
url = "https://safety.internal/mcp"
bearer_token_env = "SAFETY_TOKEN"
permission_overrides = { "classify_content" = "L0", "flag_content" = "L1" }
```

### After Phase 2 (resources):

```toml
# No config changes needed. Resources are discovered at runtime.
# Tools like mcp_resource_read are automatically registered.
```

### After Phase 3 (elicitation):

```toml
# Elicitation approval is routed through the existing approval system.
# No additional config needed unless the user wants to auto-approve:
[daemon.approval]
auto_approve_mcp_elicitation = false  # default
```

## 6. Type definitions (Phase 1 target)

New unified `McpServerConfig` in `crates/cognit/src/config/mod.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,

    #[serde(default)]
    pub transport: McpTransport,

    /// For stdio: command path
    #[serde(default)]
    pub command: Option<String>,

    /// For stdio: command arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// For http/sse: server URL
    #[serde(default)]
    pub url: Option<String>,

    /// Environment variable containing bearer token
    #[serde(default)]
    pub bearer_token_env: Option<String>,

    #[serde(default = "default_mcp_trust")]
    pub trust: McpTrustLevel,

    #[serde(default = "default_true")]
    pub enabled: bool,

    /// If set, only these tools are registered
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// If set, these tools are excluded
    #[serde(default)]
    pub denylist: Vec<String>,

    /// Override permission level for specific tools
    #[serde(default)]
    pub permission_overrides: HashMap<String, PermissionLevel>,

    /// Health check interval in seconds (0 = disabled)
    #[serde(default = "default_health_interval")]
    pub health_check_interval_sec: u64,

    /// Request timeout in milliseconds for HTTP/Sse transports
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTransport {
    #[serde(rename = "stdio")]
    Stdio,
    #[serde(rename = "http")]
    StreamableHttp,
    #[serde(rename = "sse")]
    Sse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTrustLevel {
    LocalTrusted,
    RemoteTrusted,
    Untrusted,
}
```

## 7. Security considerations

### 7.1 Trust level to permission mapping

The corrected mapping after Phase 1:

```rust
McpTrustLevel::LocalTrusted  => PermissionLevel::L0,   // read-only (local is safe)
McpTrustLevel::RemoteTrusted => PermissionLevel::L1,   // write within sandbox
McpTrustLevel::Untrusted     => PermissionLevel::L1,   // sandboxed (NOT L2)
```

Per-tool overrides in config can promote or demote individual tools.

### 7.2 Token safety

- Bearer tokens are read from environment variables at request time, never
  stored in memory beyond a single HTTP request header.
- Token values are never logged. `BearerTokenAuth::Display` always shows
  `<redacted>`.
- Tool `isError` responses are converted to generic errors without copying
  server-provided text (which may contain credentials from the request context).
- HTTP response bodies are bounded to 1 MiB.

### 7.3 Tool isolation

- MCP tools are registered with `mcp__` (or `<server>__`) prefix to prevent
  collision with built-in tools.
- All MCP tool executions go through the standard Corpus execution guard
  (`ToolRunnerWithGuard` at bootstrap line 426), including sandbox enforcement,
  audit logging, and socket approval gating.
- MCP tool results pass through `StormBreaker` output guardrail.
- Stdio MCP server stdout/stderr are piped and read programmatically; they are
  not exposed to the model except through the JSON-RPC response protocol.

### 7.4 Process lifecycle

- Stdio MCP servers are spawned as child processes. The `Child` handle is held
  in `McpTransport::Stdio._child`. When the daemon shuts down, dropping the
  transport drops the child, which is killed by the OS.
- The daemon does not currently implement graceful shutdown (SIGTERM before
  SIGKILL) for MCP subprocesses. This is a Phase 4 item.

## 8. Definition of completion

The MCP integration is complete when:

1. `McpServerConfig` is a single type used throughout the stack, with fields
   for all supported features (transport, auth, filtering, overrides).
2. Per-server tool allowlist/denylist filtering is honored during tool
   registration.
3. MCP resources can be listed and read through a standard tool interface.
4. MCP elicitation requests are routed through the Aletheon approval system
   and are visible to the user.
5. A background health-check task reconnects failed servers and refreshes tool
   lists on `tools/list_changed` notifications.
6. `supports_parallel_tool_calls` from the initialize handshake affects
   `ConcurrencyClass` assignment.
7. The trust-level-to-permission-level mapping is correct and overridable.
8. All new code paths have test coverage comparable to the existing MCP tests
   (end-to-end mock server, auth header verification, error containment).
9. Config documentation in `docs/` reflects the complete MCP server stanza
   with examples for each transport type.

## 9. What not to do

- Do not rewrite the MCP transport layer. The current `McpTransport` enum
  with Stdio/StreamableHttp/Sse is working and well-tested.
- Do not replace `reqwest` with a different HTTP client. It is already the
  project's HTTP client.
- Do not add an `rmcp` crate dependency. The current hand-rolled JSON-RPC
  transport is minimal, auditable, and has no protocol-level bugs found.
- Do not create a new `crates/mcp-gateway/` crate. The existing
  `crates/corpus/src/tools/mcp/` module is the right home.
- Do not make MCP a hard daemon dependency. The daemon must start and operate
  normally with zero MCP servers configured.
- Do not expose MCP server stdout/stderr to the model or log them at level
  higher than `trace` without sanitization.
- Do not allow MCP tool names without the `mcp__` prefix when
  `tool_name_prefix` is enabled (default).
- Do not change the `Tool` trait or `ToolRegistry` interface for MCP.
  `McpToolWrapper` already implements `Tool` correctly.

## 10. References

- MCP Specification: https://spec.modelcontextprotocol.io/
- Existing MCP code: `crates/corpus/src/tools/mcp/`
- Daemon bootstrap: `crates/executive/src/impl/daemon/bootstrap/request.rs:382-417`
- Tool trait: `crates/fabric/src/types/tool.rs:93-115`
- Tool registry: `crates/corpus/src/tools/tools/registry.rs:9-80`
- Cognit config: `crates/cognit/src/config/mod.rs:664-694`
- Executive config: `crates/executive/src/core/config/mod.rs:25,47`
- Infrastructure config re-exports: `crates/executive/src/core/config/infra.rs:6`
- Codex MCP reference: `McpConnectionManager`, `McpCatalogBuilder`, `McpHandler`
  in local Codex source (reference patterns only, not to be copied directly)
