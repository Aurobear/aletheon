# MCP、Google、外部集成与 IPC — 代码级分析

> **日期:** 2026-07-17
>
> **方法:** 逐行扫描 `crates/corpus/src/tools/mcp/`、`crates/corpus/src/tools/google/`、`crates/executive/src/impl/channel/`、`crates/executive/src/impl/automation/`、`crates/fabric/src/ipc/backends/`、`crates/mnemosyne/src/backends/gbrain/`

## 概述

分析 Aletheon 的外部连通能力：MCP 协议支持、Google 生态集成、外部消息通道、自动化调度、IPC 传输、GBrain 外部知识图谱。

**结论：MCP Client/Server 均生产级（缺 Resources/Prompts）。Google Gmail/Calendar/Drive + OAuth2.0 完整。Telegram/Gmail 通道可用。自动化 Cron/Webhook/Script 工作。IPC 以 Unix Socket 为主（io_uring 部分）。GBrain 完整但默认关闭。Discord/Slack/Email delivery 是 stub。**

---

## 1. MCP 集成

### MCP Client

**路径:** `crates/corpus/src/tools/mcp/`（7 文件，~71KB）

| 能力 | 状态 | 证据 |
|------|------|------|
| **Tools 发现/调用** | ✅ 生产级 | `client.rs:59-62` `tools/list` 发现，`client.rs:188-212` `tools/call` 调用 |
| **Resources** | ❌ 不支持 | 无 `resources/list`、`resources/read` |
| **Prompts** | ❌ 不支持 | 无 `prompts/list`、`prompts/get` |
| **Protocol 握手** | ✅ 生产级 | `initialize` + `protocolVersion: "2024-11-05"` |
| **Transport: Stdio** | ✅ 生产级 | `transport.rs:184-235` — 子进程 stdin/stdout 管道 |
| **Transport: HTTP** | ✅ 生产级 | `transport.rs:242-248` — POST + SSE 响应解析 + content-type 自动检测 |
| **Transport: SSE** | ✅ 生产级 | `transport.rs:254-308` — GET long-poll + `data:` 行解析 |
| **Transport: Fallback** | ✅ 生产级 | `transport.rs:526-557` — HTTP → SSE 自动回退 |
| **Auth: Bearer Token** | ✅ 生产级 | `auth.rs:68-119` — env var + trait |
| **Auth: OAuth 2.0 + PKCE** | ✅ 生产级 | `auth.rs:187-374` — authorization code + CSRF + refresh + expiry |
| **Token 持久化** | ✅ 生产级 | `token_store.rs:168-265` — JSON 文件 + 加密 vault + 迁移 |
| **Tool 命名** | ✅ 生产级 | `transport.rs:48-137` — PrefixServer/NumericSuffix/FirstWins 冲突策略 |
| **Trust 映射** | ✅ 生产级 | `wrapper.rs:12-77` — LocalTrusted→L0, RemoteTrusted→L1, Untrusted→L2 |
| **Notification** | 🟡 部分 | `ToolsListChanged` 解析但不操作，`notifications/initialized` 未发送 |
| **测试** | ✅ 良好 | 17 个测试（mock HTTP server + auth + error + collision） |

### MCP Embedded Server

**文件:** `crates/executive/src/impl/daemon/mcp_embedded.rs`

将 daemon 工具通过 Unix socket 暴露为 MCP 协议：

| 方法 | 能力 |
|------|------|
| `initialize` | 返回 `protocolVersion: "2024-11-05"`, `serverInfo: "aletheon-embedded-mcp"` |
| `tools/list` | 从 `corpus.catalog()` 读取（需 `ExtensionGrant`），返回完整 tool definition |
| `tools/call` | 验证工具存在 → 通过 `CapabilityService` 调用 → MCP 格式化输出 |
| `ping` | 返回 `{}` |

**5 个测试。** 外部 MCP client 可通过 Unix socket 调用 aletheon daemon 中的任意工具。

---

## 2. Google 生态集成

**路径:** `crates/corpus/src/tools/google/`（10 文件）

| 服务 | 文件 | 状态 | 关键特性 |
|------|------|------|---------|
| **Gmail** | `gmail.rs:93-344` | ✅ 生产级 | `search_messages()`, `important_unread()`, `read_message()` + ingress channel |
| **Calendar** | `calendar.rs:23-89` | ✅ 生产级 | `list_events()` + timezone + 时间范围 |
| **Drive** | `drive.rs:9-72` | ✅ 生产级 | `get_json()` + `download()` |
| **OAuth 2.0** | `oauth.rs:53-489` | ✅ 生产级 | Authorization URL + PKCE exchange + refresh + revoke + credential access |
| **API Client** | `client.rs:73-263` | ✅ 生产级 | Token auth + 401 auto-refresh + 429 rate-limit retry（`Retry-After`）+ cancellation + response bounding |
| **Gmail Sync** | `gmail_sync.rs` | ✅ 生产级 | History API 增量同步 → `ExternalEventEnvelope` |
| **Calendar Sync** | `calendar_sync.rs` | ✅ 生产级 | 有界窗口 + 增量同步 |
| **Drive Sync** | `drive_sync.rs` | ✅ 生产级 | 选择文件过滤 + MIME allowlist + 内容下载 |

**安全限制:** `oauth.rs:346-349` — 仅只读 scope（OpenId/GmailReadonly/CalendarReadonly/DriveReadonly），写操作在构造时被拒绝。

**Google Tools** (`tools.rs`):
- `GoogleGmailSearchTool`（行 25-106）
- `GoogleGmailReadTool`（行 108-181）
- `GoogleCalendarListTool`（行 183-274）— timezone 支持

---

## 3. 外部消息通道

**路径:** `crates/executive/src/impl/channel/`

### Channel Router

**文件:** `router.rs`

| 能力 | 状态 | 关键特性 |
|------|------|---------|
| Trigger 分类 | ✅ 生产级 | `/start`→Greeting, `/chat`→Chat, `/goal|/goals...`→GoalCommand, 纯文本→Chat |
| Approval 通知 | ✅ 生产级 | apply/view_diff/revision/reject 渲染 + outbox 持久化 |
| Goal 进度通知 | ✅ 生产级 | at-least-once 投递 |
| 持久存储 | ✅ 生产级 | SQLite inbox/outbox/cursor/bindings |
| At-least-once | ✅ 生产级 | `router.rs:907-962` — flush 重发但不重执行 LLM turn |

### Telegram

**文件:** `telegram/mod.rs`

- Long-poll via `GetUpdates` + `SendMessage`
- Multi-account 路由
- Google query 检测（"today's events", "important unread", 中文查询）
- Inline keyboards

### Gmail Channel

**文件:** `gmail/mod.rs` + 5 子模块

- Ingress: `GmailChannelStore` — 发件人验证 + 分类（Ask/Goal/Memory/Doc/Notification/Quarantine）
- Goal drafts: 从 incoming email 创建 Goal
- Sender policy 验证

### Daemon Adapter

**文件:** `daemon_adapter.rs`

- Turn executor: Channel Router → `DaemonTurnOrchestrator`
- Goal executor: → `ObjectiveStore` CRUD
- Approval executor: → `ApplyCoordinator`
- Gmail draft approval: confirm/edit/reject

---

## 4. 自动化调度

**路径:** `crates/executive/src/impl/automation/`

| 组件 | 文件 | 状态 | 关键特性 |
|------|------|------|---------|
| **Cron** | `cron.rs:11-161` | ✅ 生产级 | 完整 5 字段 cron parser (`*/range/step/list/exact`)，每日限制 |
| **Webhook** | `webhook.rs:22-43` | ✅ 生产级 | HMAC-SHA256 验证（constant-time），wildcard event 匹配 |
| **Script** | `script.rs:7-47` | ✅ 生产级 | `/bin/sh -c` 执行，env var 注入，stderr 捕获 |
| **Delivery** | `delivery.rs:29-89` | 🟡 部分 | Local/Stdout/Webhook 工作；Telegram URL template；**Discord/Slack/Email 是 placeholder（仅 `info!` 日志）** |
| **CRUD** | `mod.rs:106-134` | ✅ 生产级 | add/remove/list/get + duplicate-id 防护 |
| **[SILENT]** | `mod.rs:89` | ✅ 生产级 | 输出匹配 `[SILENT]` 时跳过 delivery |

---

## 5. IPC 传输

**路径:** `crates/fabric/src/ipc/backends/`

| 后端 | 文件 | 状态 | 关键特性 |
|------|------|------|---------|
| **Unix Socket** | `unix_socket.rs:30-268` | ✅ Tier 1 生产级 | Per-agent channels，length-prefixed bincode 帧，broadcast，`is_available()` 始终 true |
| **io_uring** | `io_uring.rs:16-249` | 🟡 Feature-gated | Kernel ≥5.1 检测，eventfd 通知。真实 ring setup/write 可用但 `recv` 不完整（从 eventfd 读而非 IPC 数据） |
| **Shared Memory** | `shared_mem.rs:7-279` | 🟡 单进程 | `memfd_create` + `mmap` ring buffer，仅单进程（无跨进程 fd 传递） |
| **JSON-RPC** | `json_rpc.rs:11-93` | ✅ 生产级 | Line-delimited JSON-RPC 桥接到内部 channel |
| **Transport Adapter** | `transport_adapter.rs:21-106` | ✅ 生产级 | `IpcBackend` → `Transport` trait + AgentMessage → Envelope |

**IpcManager** (`manager.rs:89-362`): 自动检测 — kernel ≥5.10 且非容器/WSL → io_uring，否则 Unix socket。始终保留 Unix socket fallback。send-with-fallback 模式。

---

## 6. GBrain 外部知识图谱

**路径:** `crates/mnemosyne/src/backends/gbrain/`（7 文件）

| 组件 | 状态 | 关键特性 |
|------|------|---------|
| `GbrainBackend` | ✅ 生产级 | `record()` → spool 提交，`recall()` → 远程查询 + timeout + 去重 + 时间验证 + sensitivity 过滤 + 内容上限 |
| Page 合约 | ✅ 生产级 | YAML frontmatter + Markdown body，`PAGE_SCHEMA_VERSION = "aletheon.memory/v1"`，128KB 上限，阻止控制指令（`<dasein_mutation>` 等） |
| Reconciliation | ✅ 生产级 | Upsert/Supersede/Tombstone 操作 |
| Spool | ✅ 生产级 | Crash-safe SQLite spool（5 个表） + 指数退避重试（1s→60s，12 次，24h 上限） |
| 迁移 | ✅ 生产级 | V2 schema + 向后兼容 ALTER TABLE |

**默认状态:** `enabled: false`（`config.rs:52-83`）。Transport-agnostic — 实际 MCP HTTP transport 通过 `SupplementalMemoryTransport` trait 外部提供。

---

## 7. Feature Flag 控制的组件

| Crate | Feature | 控制的组件 |
|-------|---------|-----------|
| corpus | `dbus`/`input`/`display`/`a11y`/`ocr-tesseract` | 平台 drivers |
| corpus | `acix` | 复合：input + display + a11y + ocr |
| corpus | `sandbox-primitives` | Bubblewrap sandbox |
| fabric | `io_uring` | 真实 io_uring kernel ring |
| fabric | `network-tests` | Unix socket 网络测试 |
| dasein | `rollback-btrfs` | BTRFS 快照回滚 |
| mnemosyne | `cognitive-memory` | 语义/程序/自我记忆（daemon 默认关闭） |
| mnemosyne | `vector-lance`/`vector-qdrant` | 向量存储 |
| bin | `integration-tests` | 集成测试 |

**MCP、Google、GBrain、automation、channels 无 feature flag**，始终编译。

---

## 总结表

| 集成 | 状态 | 备注 |
|------|------|------|
| MCP Client | ✅ 生产级 | Tools 完整；Resources/Prompts 不支持 |
| MCP Embedded Server | ✅ 生产级 | Unix socket 暴露 daemon 工具 |
| Google Gmail | ✅ 生产级 | 只读 scope |
| Google Calendar | ✅ 生产级 | 只读 scope |
| Google Drive | ✅ 生产级 | 只读 scope |
| Google OAuth 2.0 | ✅ 生产级 | PKCE + refresh + revoke |
| web_fetch | ✅ 生产级 | GET/POST，1MB cap |
| web_search | ✅ 生产级 | 外部 API |
| Telegram Channel | ✅ 生产级 | Long-poll + inline keyboards |
| Gmail Channel | ✅ 生产级 | Ingress + 分类 + sender 验证 |
| Channel Router | ✅ 生产级 | SQLite-backed at-least-once |
| Cron Automation | ✅ 生产级 | 5 字段 parser |
| Webhook Automation | ✅ 生产级 | HMAC-SHA256 |
| Script Runner | ✅ 生产级 | `/bin/sh -c` |
| Discord/Slack/Email Delivery | ❌ Stub | 仅 `info!` 日志 |
| Unix Socket IPC | ✅ 生产级 | Tier 1 |
| io_uring IPC | 🟡 Feature-gated | Ring 可用但 recv 不完整 |
| Shared Memory IPC | 🟡 单进程 | 无跨进程 fd 传递 |
| GBrain Backend | ✅ 生产级 | 默认关闭 |
