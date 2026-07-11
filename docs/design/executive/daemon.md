# Aletheon Daemon (aletheon daemon)

> 持久运行的后台守护进程，通过 Unix socket 接收 CLI 请求，调度 LLM 推理与工具执行。
> 作为 systemd 服务运行，是 Aletheon 系统的核心入口。

**模块编号:** Daemon
**关联模块:** [cli](../interact/README.md), [cognitive-engine](../cognit/cognitive-engine.md), [perception](../corpus/perception.md)
**最后更新:** 2026-07-03

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| CLI entry point | ✅ Implemented | `crates/bin/src/main.rs` | clap-based arg parsing |
| .env loading | ✅ Implemented | `crates/executive/src/host/mod.rs` | `load_dotenv()` shared across hosts |
| TOML config loading | ✅ Implemented | `crates/executive/src/core/runtime_core.rs` | `AppConfig::load_layered()` |
| Provider registry init | ✅ Implemented | `crates/executive/src/core/runtime_core.rs` | `ProviderRegistry::from_config()` |
| Unix socket server | ✅ Implemented | `crates/executive/src/impl/daemon/server.rs` | Line-delimited JSON-RPC, concurrent connections |
| RequestHandler | ✅ Implemented | `crates/executive/src/impl/daemon/handler/mod.rs` | chat/clear/status/health/session.* methods |
| Chat → ReAct loop | ✅ Implemented | `crates/executive/src/impl/daemon/handler/chat.rs` | `ReActLoop::run_streaming()` with full tool calling |
| Streaming responses | ✅ Implemented | `crates/executive/src/core/react_loop/tool_exec.rs` | `ChannelEventSink` → JSON-RPC notifications (TextDelta, ToolCallStart, etc.) |
| Perception manager | ✅ Implemented | `crates/executive/src/core/runtime_core.rs` | Spawns in background task via bootstrap |
| Perception bridge | ✅ Implemented | `crates/executive/src/core/runtime_core.rs` | Event→Engine injection channel |
| Agent registry | ✅ Implemented | `crates/executive/src/impl/daemon/handler/mod.rs` | Config-based + builtin fallback |
| Memory system init | ✅ Implemented | `crates/executive/src/impl/daemon/handler/mod.rs` | CoreMemory + RecallMemory + FactStore |
| Graceful shutdown | ✅ Implemented | `crates/executive/src/impl/daemon/server.rs`, `host/mod.rs` | JoinSet connection drain (5s timeout), InterruptFlag, per-turn cancel token |
| Health check endpoint | ✅ Implemented | `crates/executive/src/impl/daemon/handler/rpc.rs` | `health` RPC: uptime, connections, sessions, version |
| Multi-session support | ✅ Implemented | `crates/executive/src/impl/daemon/handler/mod.rs` | HashMap-based session registry, `session.create/list/switch` RPC |
| SystemdHost | ✅ Implemented | `crates/executive/src/host/systemd.rs` | sd_notify(READY/WATCHDOG/STOPPING), SIGTERM handler |
| ContainerHost | ✅ Implemented | `crates/executive/src/host/container.rs` | Docker/Podman container lifecycle via CLI |
| RuntimeCore (host-agnostic) | ✅ Implemented | `crates/executive/src/core/runtime_core.rs` | Shared bootstrap for all host types |
| `aletheon exec` (CI/CD) | ✅ Implemented | `crates/bin/src/main.rs` | Non-interactive batch execution |

---

## 目录

- [1. 概述](#1-概述)
- [2. 架构](#2-架构)
- [3. 入口与启动流程](#3-入口与启动流程)
  - [3.1 CLI 参数](#31-cli-参数)
  - [3.2 配置加载](#32-配置加载)
  - [3.3 Provider 注册表初始化](#33-provider-注册表初始化)
  - [3.4 感知管理器启动](#34-感知管理器启动)
  - [3.5 RequestHandler 初始化](#35-requesthandler-初始化)
  - [3.6 Unix Socket 服务启动](#36-unix-socket-服务启动)
- [4. 当前设计](#4-当前设计)
  - [4.1 Unix Socket 服务器](#41-unix-socket-服务器)
  - [4.2 RequestHandler 请求分发](#42-requesthandler-请求分发)
  - [4.3 会话状态管理](#43-会话状态管理)
- [5. 配置参考](#5-配置参考)
- [6. 已识别缺陷](#6-已识别缺陷)

---

## 1. 概述

`aletheon daemon` 是 Aletheon 系统的持久后台进程。它：

1. 加载 TOML 配置和环境变量
2. 初始化 LLM Provider 注册表
3. 启动感知管理器（procfs polling、journald）
4. 通过 Unix socket 接收 CLI 的 JSON-RPC 请求
5. 将请求分发到认知引擎（AletheonExecutive）处理
6. 返回 JSON-RPC 响应

设计目标：单一进程、单一会话、低开销常驻服务。

---

## 2. 架构

```
┌──────────────────────────────────────────────────────────┐
│                     interact                          │
│           (single message / TUI / simple REPL)            │
└────────────────────────┬─────────────────────────────────┘
                         │ Unix Socket (JSON-RPC, line-delimited)
                         ▼
┌──────────────────────────────────────────────────────────┐
│                     aletheon daemon (daemon)                    │
│                                                          │
│  ┌─────────────────────────────────────────────────────┐ │
│  │                 UnixServer                           │ │
│  │  accept() → spawn handle_connection() per client     │ │
│  │  BufReader::read_line → JSON parse → handler.handle()│ │
│  └──────────────────────┬──────────────────────────────┘ │
│                         ▼                                │
│  ┌─────────────────────────────────────────────────────┐ │
│  │              RequestHandler                          │ │
│  │  "chat"   → LLM complete() → text response          │ │
│  │  "clear"  → reset pending_input                      │ │
│  │  "status" → runtime iteration + session_id           │ │
│  └──────────────────────┬──────────────────────────────┘ │
│                         ▼                                │
│  ┌─────────────────────────────────────────────────────┐ │
│  │              AletheonExecutive                         │ │
│  │  (ReAct loop, tool execution, agent dispatch)        │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌──────────────────┐  ┌───────────────────────────────┐ │
│  │ PerceptionManager│  │ PerceptionBridge               │ │
│  │ /proc, journald  │→ │ event_rx → injection_tx        │ │
│  └──────────────────┘  └───────────────────────────────┘ │
│                                                          │
│  ┌──────────────────┐  ┌───────────────────────────────┐ │
│  │ ProviderRegistry │  │ AgentRegistry                  │ │
│  │ LLM providers    │  │ Fs/Net/Code agents             │ │
│  └──────────────────┘  └───────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

---

## 3. 入口与启动流程

### 3.1 CLI 参数

入口文件: `crates/bin/src/main.rs`

```rust
#[derive(Subcommand)]
enum Commands {
    Daemon {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long)]
        env: Option<PathBuf>,
        #[arg(short, long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        container: Option<String>,
        #[arg(long, default_value_t = false)]
        enable_evolution: bool,
    },
    Exec { /* non-interactive execution options */ },
    Version,
}
```

`main()` 根据 `daemon` 子命令选择 `SystemdHost`、`ContainerHost` 或前台 `DaemonHost`，随后完成初始化并进入服务循环。

### 3.2 配置加载

代码位置: `runtime/src/impl/daemon/mod.rs`

启动顺序:

1. **加载 .env** — 搜索 `~/.aletheon/.env`，回退到 `./.env`。简单的 KEY=VALUE 解析器，不覆盖已存在的环境变量。
2. **加载 TOML 配置** — 搜索 `~/.aletheon/config.toml`，回退到 `/etc/aletheon/config.toml`。调用 `AppConfig::load_or_default()`，失败时使用默认配置。
3. **构建 DaemonConfig** — 从 AppConfig 和环境变量中提取字段。

```rust
pub struct DaemonConfig {
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub sandbox_preference: String,
}
```

环境变量回退:

| 字段 | 环境变量 | 默认值 |
|------|---------|--------|
| working_dir | `AGENT_WORKING_DIR` | `/tmp` |
| data_dir | `AGENT_DATA_DIR` | XDG data dir |
| system_prompt | `AGENT_SYSTEM_PROMPT` | `"You are a helpful system assistant."` |
| sandbox_preference | `AGENT_SANDBOX_PREFERENCE` | `"auto"` |

### 3.3 Provider 注册表初始化

```rust
let registry = ProviderRegistry::from_config(&app_config)?;
let (default_provider_config, default_model) = registry.resolve("")?;
```

`ProviderRegistry` 从 TOML 配置的 `[providers]` 表构建。`resolve("")` 返回默认 provider（配置中的第一个）。后续 `RequestHandler` 通过 `registry.resolve_and_create("")` 创建 `LlmProvider` trait object。

### 3.4 感知管理器启动

代码位置: `runtime/src/impl/daemon/mod.rs` `run()` 函数

```rust
let (event_tx, event_rx) = mpsc::channel::<PerceptionEvent>(256);
let (injection_tx, injection_rx) = mpsc::channel::<PerceptionInjection>(64);

// 启动 PerceptionManager（后台 tokio task）
let watch_paths = vec![PathBuf::from("/etc"), PathBuf::from("/var/log")];
tokio::spawn(async move {
    let mut manager = PerceptionManager::new(event_tx, watch_paths, true);
    manager.start().await;
});

// 启动 PerceptionBridge（后台 tokio task）
let mut bridge = PerceptionBridge::new(event_rx, injection_tx);
tokio::spawn(async move { bridge.run().await; });
```

- **PerceptionManager** — 轮询 `/proc`、监听 journald，产生 `PerceptionEvent`
- **PerceptionBridge** — 将事件转换为 `PerceptionInjection`，通过 channel 传递给引擎

### 3.5 RequestHandler 初始化

代码位置: `runtime/src/impl/daemon/handler.rs`

`RequestHandler::new()` 完成以下初始化:

1. **LLM Provider** — 从 registry 创建 `Arc<dyn LlmProvider>`
2. **Session** — 生成 UUID session_id，创建 `EventJournal` 和 `SessionStore`
3. **Memory** — `CoreMemory`（in-memory）+ `RecallMemory`（SQLite，路径 `data_dir/recall_memory.db`）
4. **Tools** — 注册 CoreMemoryAppend/Replace + MemorySearch 工具
5. **Security** — `SandboxExecutor` + `AuditLogger`（`data_dir/audit.jsonl`）+ `ToolRunnerWithGuard`
6. **Agent Registry** — 尝试从 `agents/` 目录加载配置 agent，回退到内置 FsAgent/NetAgent/CodeAgent
7. **AletheonExecutive** — 创建 runtime 实例

### 3.6 Unix Socket 服务启动

```rust
let unix_server = UnixServer::new(&socket, request_handler).await?;
unix_server.run().await?;
```

`run()` 是无限循环，接受连接后 spawn 独立 tokio task 处理。

---

## 4. 当前设计

### 4.1 Unix Socket 服务器

代码位置: `runtime/src/impl/daemon/server.rs`

协议: **行分隔 JSON-RPC**（每条消息以 `\n` 结尾）

```rust
pub struct UnixServer {
    listener: UnixListener,
    handler: RequestHandler,
}
```

连接处理流程:

```
client connect
  → BufReader::read_line()
  → serde_json::from_str()
  → handler.handle(request)
  → serde_json::to_string(response)
  → writer.write_all(response + "\n")
  → loop (read next line or EOF)
```

关键设计点:
- 启动时移除已存在的 stale socket 文件
- socket 权限固定为 `0660`、所有者组为 `aletheon`
- 安装时新增的 supplementary group 不会自动进入既有登录进程；使用
  `id -nG | grep -w aletheon` 检查，并通过重新登录、`newgrp aletheon`
  或临时执行 `sg aletheon -c 'aletheon'` 激活
- 每个连接独立 tokio task（支持并发客户端）
- 连接 EOF 时自动清理

### 4.2 RequestHandler 请求分发

代码位置: `runtime/src/impl/daemon/handler.rs`

```rust
pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value
```

支持的方法:

| 方法 | 参数 | 响应 | 说明 |
|------|------|------|------|
| `chat` | `params.message: string` | `result.response: string` (streaming via notifications) | 调用 ReActLoop 推理循环，支持工具调用 |
| `clear` | 无 | `result.status: "ok"` | 清除 pending_input |
| `status` | 无 | `result.iteration, config` | 查询 runtime 状态 |
| `health` | 无 | `result.uptime, connections, sessions, version` | 健康检查端点 |
| `session.create` | 无 | `result.session_id: string` | 创建新会话 |
| `session.list` | 无 | `result.sessions: array` | 列出所有活跃会话 |
| `session.switch` | `params.session_id: string` | `result.status: "ok"` | 切换活跃会话 |

流式响应通过 JSON-RPC notification（无 id 字段）推送：`TextDelta`, `ToolCallStart`, `ToolCallEnd`, `ToolCallResult` 等事件类型。

错误响应: JSON-RPC 标准错误格式，`code: -32000` (LLM error) 或 `-32601` (unknown method)。

**注意:** `chat` 方法现已完整走 `ReActLoop::run_streaming()` 推理循环，支持工具调用、多轮推理和安全策略检查。详见 `crates/executive/src/impl/daemon/handler/chat.rs`。

### 4.3 会话状态管理

会话管理通过 `SessionManager` (`crates/executive/src/impl/daemon/session_manager.rs`) 提供多会话支持:

- `session.create` — 创建新会话（UUID），持久化到 SQLite
- `session.list` — 列出所有活跃会话
- `session.switch` — 切换活跃会话
- 优雅关闭时自动持久化会话状态

---

## 5. 配置参考

TOML 配置文件路径: `~/.aletheon/config.toml`

```toml
[providers.anthropic]
name = "anthropic"
api_key = "sk-..."
base_url = "https://api.anthropic.com"
model = "claude-sonnet-4-20250514"
```

数据目录结构:

```
~/.local/share/aletheon/
├── recall_memory.db      # SQLite 记忆数据库
├── audit.jsonl           # 安全审计日志
└── sessions/             # EventJournal 会话日志
```

---

## 6. 已识别缺陷（已全部修复）

以下为历史缺陷记录，已在 P0-P2 稳定化阶段全部修复：

### 6.1 Chat 未走 ReAct 循环 ✅ 已修复

~~`chat` 方法直接调用 `llm.complete()`，绕过了 `AletheonExecutive` 的 ReAct 推理循环。~~

现已通过 `ReActLoop::run_streaming()` 完整集成，支持工具调用、多轮推理和安全策略检查。详见 `crates/executive/src/impl/daemon/handler/chat.rs`。

### 6.2 无流式响应 ✅ 已修复

~~同步 request-response 模式，CLI 必须等待完整响应才能显示。~~

现已通过 `ChannelEventSink` → JSON-RPC notification 实现流式 chunk 推送（TextDelta, ToolCallStart, ToolCallEnd 等）。详见 `crates/executive/src/core/react_loop/tool_exec.rs`。

### 6.3 无优雅关闭 ✅ 已修复

~~无 SIGTERM/SIGINT 处理，无连接排空逻辑。~~

现已实现：JoinSet 连接排空（5s 超时）、InterruptFlag 取消机制、per-turn cancel token。详见 `crates/executive/src/impl/daemon/server.rs` 和 `crates/executive/src/host/mod.rs`。

### 6.4 单会话限制 ✅ 已修复

~~每次 daemon 启动创建一个 session，无多会话或会话恢复支持。~~

现已通过 `SessionManager` 实现 HashMap-based 多会话注册表，支持 `session.create/list/switch` RPC 方法。详见 `crates/executive/src/impl/daemon/session_manager.rs`。
