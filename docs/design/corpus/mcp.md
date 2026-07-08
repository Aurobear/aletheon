> Migrated from docs/design/execution/mcp-integration.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# MCP 集成 (Model Context Protocol Integration)

> Aletheon 通过 MCP (Model Context Protocol) 接入第三方工具服务器，实现工具系统的开放扩展。

**模块编号:** 03 (MCP 子系统)
**关联模块:** [tool-system.md](tools.md), [sandbox.md](sandbox.md), [shared/traits.md](../base/types.md)
**最后更新:** 2026-06-07
**来源:** 从 `03-tool-system.md` 的 S3.4、S3.5、S3.7、S4.4、S4.5、S4.7 节提取。

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| MCP client (stdio) | ✅ Implemented | `mcp/client.rs` | McpClient: connect, initialize handshake, tools/list, tools/call |
| MCP transport (stdio) | ✅ Implemented | `mcp/transport.rs` | Spawns subprocess, stdin/stdout JSON-RPC |
| MCP connection manager | ✅ Implemented | `mcp/client.rs` | McpConnectionManager: multi-server lifecycle, get_all_tools() |
| MCP tool wrapper | ✅ Implemented | `mcp/wrapper.rs` | McpToolWrapper adapts MCP tools to Tool trait |
| MCP config | ✅ Implemented | `mcp/config.rs` | McpConfig, McpServerConfig, McpTransportConfig, McpTrustLevel |
| StreamableHTTP transport | ✅ Implemented | `mcp/transport.rs` | HTTP POST + SSE, `McpTransport::StreamableHttp` variant |
| SSE transport | ✅ Implemented | `mcp/transport.rs` | HTTP GET long-poll fallback, integrated in transport enum |
| connect_with_fallback() | ✅ Implemented | `mcp/transport.rs` | StreamableHTTP → SSE fallback, auth errors don't fall back |
| BearerTokenAuth | ✅ Implemented | `mcp/auth.rs` | Static token from env/config, error on missing |
| McpOAuthProvider | ✅ Implemented | `mcp/auth.rs` | OAuth 2.0 authorization code flow, CSRF state, TokenStore |
| ToolNameConfig + CollisionStrategy | ✅ Implemented | `mcp/transport.rs` | HashSuffix / NamespaceCompress normalization |
| McpNotification | ✅ Implemented | `mcp/transport.rs` | `notifications/tools/list_changed` parsing |

---

## 目录

1. [概述](#1-概述)
2. [传输方式选择](#2-传输方式选择)
3. [工具发现与注册](#3-工具发现与注册)
4. [OAuth 认证](#4-oauth-认证)
5. [工具名规范化](#5-工具名规范化)
6. [安全模型](#6-安全模型)
7. [集成点](#7-集成点)
8. [错误处理](#8-错误处理)
9. [实现要点](#9-实现要点)
10. [参考来源](#10-参考来源)

---

## 1. 概述

MCP (Model Context Protocol) 是 Anthropic 提出的开放协议，允许外部进程以标准化方式向 AI Agent 暴露工具、资源和提示词。Aletheon 作为 MCP 客户端，通过 `McpConnectionManager` 统一管理多个 MCP 服务器的生命周期。

**核心组件:**

| 组件 | 职责 |
|------|------|
| `McpClient` | 单服务器连接，传输层抽象，工具调用代理 |
| `McpConnectionManager` | 多服务器生命周期管理，并发启动，进程树清理 |
| `McpToolWrapper` | 将 MCP 工具适配为 Aletheon 的 `Tool` trait |
| `ToolNameConfig` | 工具名规范化策略，碰撞处理 |

---

## 2. 传输方式选择

MCP 定义三种传输方式，Aletheon 目前仅实现 stdio：

| 传输 | 连接方式 | 适用场景 | 状态 |
|------|----------|----------|------|
| **stdio** | 子进程 stdin/stdout | 本地工具、CLI 包装 | ✅ 已实现 |
| **StreamableHTTP** | HTTP POST + SSE 响应 | 远程服务器（推荐） | ✅ 已实现 |
| **SSE** | HTTP GET 长连接 | 旧版远程服务器（兼容） | ✅ 已实现 |

**决策树:**

```
服务器是否在本机？
├── 是 → stdio（启动子进程，零配置）
└── 否 → 服务器是否支持 StreamableHTTP？
    ├── 是 → StreamableHTTP
    └── 否 → SSE（仅作兼容回退）
```

**传输配置结构:**

**McpTransportConfig** — 三种传输配置：stdio（子进程 stdin/stdout）、StreamableHttp（HTTP POST + SSE）、Sse（HTTP GET 长连接，兼容回退）。

**回退策略:** StreamableHTTP 失败时自动降级到 SSE。认证错误时不回退（避免暴露凭证到不同端点）。

---

## 3. 工具发现与注册

### 3.1 发现流程

MCP 服务器通过 `tools/list` RPC 方法暴露可用工具。Aletheon 在连接建立后调用此方法获取工具列表。

```
McpConnectionManager::start_all()
  ├── 对每个服务器并发启动 McpClient
  ├── 调用 client.list_tools() → Vec<McpTool>
  ├── 对每个 McpTool:
  │   ├── normalize_tool_name() → 规范化名称
  │   ├── apply ToolFilter → 检查 enabled/disabled 列表
  │   └── McpToolWrapper::new() → 适配为 Tool trait
  └── 注册到 ToolRegistry
```

### 3.2 Schema 映射

MCP 的 `inputSchema` (JSON Schema) 直接映射到 Aletheon 的 `Tool::input_schema()` 返回值。处理要点：

- 确保 `properties` 字段存在（OpenAI 兼容性要求）
- `outputSchema` 校验失败时使用容忍性重试（`TolerantListToolsResultSchema`）

### 3.3 动态更新

MCP 服务器可通过 `notifications/tools/list_changed` 通知工具列表变更。Aletheon 监听此通知并重新拉取工具列表，更新 `ToolRegistry`。

---

## 4. OAuth 认证

### 4.1 认证方式

MCP 支持两种认证方式：

| 方式 | 状态 | 说明 |
|------|------|------|
| Bearer Token | ✅ 已实现 | `BearerTokenAuth`，静态 token 从环境变量或配置文件读取 |
| OAuth 2.0 | ✅ 已实现 | `McpOAuthProvider`，授权码流程 + CSRF state + TokenStore |

### 4.2 Bearer Token

**BearerTokenAuth** — 从环境变量读取 token，空/缺失时显式报错。静态 token，无需刷新。

### 4.3 OAuth 2.0

`McpOAuthProvider` 实现标准三阶段 OAuth 授权码流程：

- `authorize_url()` — 构建用户浏览器访问的授权 URL
- `callback(code, state)` — 用授权码交换 token
- `get_headers()` — 返回 `Authorization` header，token 过期时自动刷新
- Token 存储通过 `TokenStore` 管理
- CSRF state 验证（`pending_states` HashMap）

---

## 5. 工具名规范化

### 5.1 问题

MCP 工具名限制 64 字节。Aletheon 多服务器场景下：
- 截断后名称语义丢失
- SHA1 哈希后缀不可读
- 多服务器碰撞概率高

### 5.2 规范化策略

```rust
struct ToolNameConfig {
    max_length: usize,           // 默认 64
    prefix_server_name: bool,    // 是否添加 mcp__<server>__ 前缀
    collision_strategy: CollisionStrategy,
}

enum CollisionStrategy {
    HashSuffix,      // 截断 + SHA1 后 6 位（当前实现）
    NamespaceCompress, // 压缩命名空间前缀（改进方案）
}
```

**HashSuffix（当前）:** 超出 64 字节时截断，碰撞时追加 `_{sha1[..6]}`。

**NamespaceCompress（改进）:** 将 `mcp__server_name__tool_name` 压缩为 `mcp__s1__tool_name`（服务器名哈希），保留工具名语义。

### 5.3 工具别名注册

`ToolAliasRegistry` 维护规范化名称与原始 MCP 名称的映射，用于日志、审计和调试时的可追溯性。

---

## 6. 安全模型

### 6.1 信任等级

| 信任等级 | 来源 | 沙箱策略 | 速率限制 |
|----------|------|----------|----------|
| **LocalTrusted** | 本机 stdio，用户显式配置 | 无沙箱 (NoopBackend) | 无 |
| **RemoteTrusted** | 远程服务器，已认证 | 进程沙箱 (ProcessBackend) | 100 req/min |
| **Untrusted** | 未知来源，未认证 | 完整沙箱 (BubblewrapBackend) | 20 req/min |

```rust
enum McpTrustLevel {
    LocalTrusted,
    RemoteTrusted,
    Untrusted,
}
```

### 6.2 工具过滤

每个服务器可配置 `ToolFilter`，包含 `enabled` 和 `disabled` 白/黑名单。过滤在 `tools/list` 和 `tools/call` 两处边界检查。

### 6.3 可见性控制

MCP 工具可通过 `_meta.ui.visibility` 字段控制是否对 LLM 可见（`tool_is_model_visible` 检查）。

---

## 7. 集成点

### 7.1 Tool Trait 适配

**McpToolWrapper** — 将 MCP 工具适配为 Aletheon 的 Tool trait，将 `tools/call` 调用代理到远程服务器。
- 代码位置: `mcp/tool_adapter.rs`
- 默认 Deferred 暴露级别，通过 tool_search/tool_describe/tool_call 桥接发现

### 7.2 LoopDetector 交互

MCP 工具调用经过与内置工具相同的 `LoopDetector` 检测。`McpToolWrapper` 的错误返回格式化为可操作指引（`format_mcp_error`），帮助 Agent 理解失败原因。

### 7.3 权限级别

MCP 工具默认为 `Deferred` 暴露级别（通过 `tool_search` / `tool_describe` / `tool_call` 桥接发现）。高频使用的 MCP 工具可在配置中提升为 `Immediate`。

### 7.4 多模态输出

MCP 协议支持 `type: "image"`（base64）和 `type: "resource"`（URI 引用）content block。Aletheon 的 `ToolResultContent` 枚举已定义对应变体：Text, Image, FileRef, Resource。

---

## 8. 错误处理

### 8.1 传输层故障

| 故障类型 | 处理策略 |
|----------|----------|
| 连接超时 | `startup_timeout` (默认 30s)，超时标记服务器为 `Failed` |
| 工具调用超时 | `tool_timeout` (默认 120s)，返回超时错误给 Agent |
| 服务器进程崩溃 | 进程退出检测，标记为 `Disconnected`，按配置重试 |
| 网络中断 (HTTP) | 自动重连，指数退避 |

### 8.2 协议层错误

MCP 协议错误通过 `McpError` 枚举统一处理：

```rust
enum McpError {
    TransportError(String),      // 传输层故障
    ProtocolError { code: i32, message: String }, // JSON-RPC 错误
    ToolNotFound(String),        // 工具不存在
    AuthRequired,                // 需要重新认证
    Timeout,                     // 调用超时
}
```

`format_mcp_error()` 将错误转换为可操作的文本指引，注入到 `ToolResult` 的错误输出中。

### 8.3 容错策略

- **连接级:** 单服务器故障不影响其他服务器（独立 `McpClient` 实例）
- **工具级:** 单工具调用失败不阻断整个回合（`LoopDetector` 计数）
- **启动级:** 并发启动时使用 `JoinSet` + `CancellationToken`，部分服务器失败不阻塞启动完成

---

## 9. 实现要点

| 项目 | 说明 |
|------|------|
| **MCP 客户端** | `mcp/client.rs` — McpClient (stdio) + McpConnectionManager (多服务器) |
| **传输层** | `mcp/transport.rs` — McpTransport Stdio 变体 (子进程 stdin/stdout) |
| **工具适配** | `mcp/wrapper.rs` — McpToolWrapper 适配 Tool trait，权限基于信任等级 |
| **配置** | `mcp/config.rs` — McpConfig + McpServerConfig + McpTransportConfig + McpTrustLevel |

**已实现的文件:** `crates/corpus/src/impl/mcp/` (mod.rs, client.rs, config.rs, transport.rs, wrapper.rs)

**未实现的设计规格（本文档保留）:**
- 资源/提示词 API

---

## 10. 参考来源

| 来源 | 关键内容 | 借鉴内容 |
|------|----------|----------|
| Anthropic SDK | MCP protocol support | stdio / StreamableHTTP / SSE 传输 |
| OpenCode MCP | `McpService` (index.ts:237-271) | 状态枚举: connected/disabled/failed/needs_auth |
| OpenCode MCP | `StreamableHTTPClientTransport` → `SSEClientTransport` fallback | 远程传输回退链 |
| OpenCode MCP | `McpOAuthProvider` + CSRF state | OAuth 流程: authorize → callback → state 验证 |
| OpenCode MCP | Tool name `sanitize()` | 非 `[a-zA-Z0-9]` 替换为 `_` |
| OpenCode MCP | `TolerantListToolsResultSchema` | outputSchema 校验失败容忍性重试 |
| OpenCode MCP | `ToolListChangedNotification` handler | 动态工具列表更新事件 |
| OpenCode MCP | Process tree cleanup via `pgrep -P` | 关闭时清理 stdio 子进程的所有后代 |
| Codex MCP | `McpConnectionManager` | 集中管理所有服务器，HashMap<String, AsyncManagedClient> |
| Codex MCP | `AsyncManagedClient` shared future | 异步惰性初始化，多调用者共享同一个启动 future |
| Codex MCP | `normalize_tools_for_model_with_prefix` | 64B 上限 + SHA1 碰撞哈希 + `mcp__` 前缀开关 |
| Codex MCP | `ToolFilter { enabled, disabled }` | 每服务器工具过滤器 |
| Codex MCP | `JoinSet` + `CancellationToken` 并发启动 | 并行服务器启动 + 关闭传播 |
| Codex MCP | Per-server `startup_timeout` / `tool_timeout` | 30s 启动超时 + 120s 工具超时默认值 |

---

## Implementation Summary

> **MCP 全传输层已实现** — stdio、StreamableHTTP、SSE 传输均可用，OAuth 2.0 认证已实现，工具名规范化和动态通知已实现。

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| MCP client | ✅ 已实现 | `mcp/client.rs` | stdio 连接 + initialize + tools/list + tools/call |
| McpConnectionManager | ✅ 已实现 | `mcp/client.rs` | 多服务器生命周期管理 |
| McpToolWrapper | ✅ 已实现 | `mcp/wrapper.rs` | MCP 工具 → Tool trait 适配 |
| McpTransport (stdio) | ✅ 已实现 | `mcp/transport.rs` | stdio 子进程传输 |
| McpTransport (StreamableHTTP) | ✅ 已实现 | `mcp/transport.rs` | HTTP POST + SSE |
| McpTransport (SSE) | ✅ 已实现 | `mcp/transport.rs` | HTTP GET 长连接 |
| connect_with_fallback() | ✅ 已实现 | `mcp/transport.rs` | StreamableHTTP → SSE, auth errors don't fall back |
| BearerTokenAuth | ✅ 已实现 | `mcp/auth.rs` | 静态 token 认证 |
| McpOAuthProvider | ✅ 已实现 | `mcp/auth.rs` | OAuth 2.0 授权码流程 + CSRF + TokenStore |
| ToolNameConfig + CollisionStrategy | ✅ 已实现 | `mcp/transport.rs` | HashSuffix / NamespaceCompress |
| McpNotification | ✅ 已实现 | `mcp/transport.rs` | `notifications/tools/list_changed` |
| McpConfig | ✅ 已实现 | `mcp/config.rs` | 服务器配置 + 信任等级 |

**代码路径:** `crates/corpus/src/impl/mcp/` (6 files: mod.rs, client.rs, config.rs, transport.rs, wrapper.rs, auth.rs)
