# 工具系统与沙箱执行 (Tool System & Sandbox Execution)

> 定义工具分类、生命周期与沙箱隔离执行模型，确保工具调用安全可控。

**模块编号:** 03
**关联模块:** [cognitive-engine](../core/cognitive-engine.md), [memory-system](../core/memory-system.md)
**最后更新:** 2026-06-06
**注:** 本文档为 `03-tool-system.md` 的副本，已更新跨文件引用路径。
沙箱相关内容已拆分至 [sandbox.md](sandbox.md)。
MCP 集成相关内容已拆分至 [mcp-integration.md](mcp-integration.md)。

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Tool trait | ✅ Implemented | `tool/mod.rs` | Unified tool interface with permission_level, exposure, concurrency_class |
| Tool registry | ✅ Implemented | `tool/registry.rs` | Registration, lookup, visibility filtering |
| 5 built-in tools | ✅ Implemented | `tool/bash_exec.rs`, `file_read.rs`, `file_write.rs`, `process_list.rs`, `system_status.rs` | Core tools working |
| OutputManager | ✅ Implemented | `tool/output.rs` | Output bounding and truncation |
| Tool exposure tiers | ⬜ Planned | — | Direct/Deferred/Hidden designed, not implemented |
| Tool parallelism | ⬜ Planned | — | RwLock gate + JoinSet designed, not implemented |
| MCP integration | ⬜ Planned | — | See [mcp-integration.md](mcp-integration.md) |
| BM25 tool search | ⬜ Planned | — | CatalogEntry + scoring designed |
| Toolset composition | ⬜ Planned | — | Toolset + include chain designed |

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
  - [2.1 工具分类](#21-工具分类)
  - [2.2 工具 Trait](#22-工具-trait)
  - [2.3 沙箱执行流程](#23-沙箱执行流程)
  - [2.4 沙箱配置](#24-沙箱配置)
  - [2.5 沙箱执行器](#25-沙箱执行器)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [3.1 P0: 工具输出的上下文管理](#31-p0-工具输出的上下文管理)
  - [3.2 P1: 工具分层暴露 (ToolExposure)](#32-p1-工具分层暴露-toolexposure)
  - [3.3 P1: 工具并行执行](#33-p1-工具并行执行)
  - [3.4 P1: MCP 集成](#34-p1-mcp-集成)
  - [3.5 P2: MCP OAuth 不完整](#35-p2-mcp-oauth-不完整)
  - [3.6 P1: 沙箱后端可移植性](#36-p1-沙箱后端可移植性)
  - [3.7 P2: MCP 工具名 64 字节限制](#37-p2-mcp-工具名-64-字节限制)
- [4. 改进设计](#4-改进设计)
  - [4.1 Bounded Output + Overflow to File](#41-bounded-output--overflow-to-file)
  - [4.2 ToolExposure 分层暴露](#42-toolexposure-分层暴露)
  - [4.3 工具并行执行 (ToolCallExecutor)](#43-工具并行执行-toolcallexecutor)
  - [4.4 MCP 集成](#44-mcp-集成)
  - [4.5 MCP OAuth 认证与多模态输出](#45-mcp-oauth-认证与多模态输出)
  - [4.6 沙箱后端可移植性](#46-沙箱后端可移植性)
  - [4.7 MCP 工具名规范化改进](#47-mcp-工具名规范化改进)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

工具系统是 OS-Agent 与外部世界交互的唯一通道。它定义了：

1. **工具分类** — 按权限级别和功能域组织工具
2. **工具 Trait** — 统一的工具接口规范
3. **沙箱执行** — bubblewrap + seccomp + cgroups 的进程隔离
4. **审计与回滚** — 所有工具调用可审计、关键操作可回滚

工具系统的设计目标是：**安全第一，性能其次**。每个工具调用都经过权限检查、沙箱隔离、结果收集、审计记录的完整流程。

---

## 2. 当前设计

### 2.1 工具分类

```
系统工具 (L0-L1):
├── bash_exec        执行 shell 命令
├── file_read        读取文件
├── file_write       写入文件
├── file_search      搜索文件 (rg/find)
├── process_list     列出进程
├── service_control  systemd 服务控制
└── network_info     网络状态查询

感知工具 (L0):
├── screen_capture   屏幕截图 + OCR
├── system_status    CPU/内存/磁盘状态
├── log_stream       实时日志流
└── event_query      查询系统事件

控制工具 (L1-L2):
├── service_manage   启停/重启服务
├── package_install  安装软件包
├── network_config   网络配置修改
└── firewall_rule    防火墙规则管理

记忆工具 (L0):
├── core_memory_append   追加核心记忆
├── core_memory_replace  替换核心记忆
├── recall_search        搜索回忆记忆
└── archival_search      搜索归档记忆

委托工具:
└── delegate             委托任务给其他 Agent
```

**权限级别说明：**
- **L0** — 自动执行，无需通知（读取、搜索、记忆更新）
- **L1** — 通知后执行，做了告诉用户（安装软件、修改配置、服务管理）
- **L2** — 需要确认，做之前先问（删除文件、sudo、防火墙规则）
- **L3** — 禁止（`rm -rf /`、修改内核模块、关闭安全服务）

### 2.2 工具 Trait

> **See [shared/traits.md](../shared/traits.md) for the canonical `Tool` trait definition.**
> The fields described below are the design rationale for each method.

**设计要点：**
- `input_schema()` 返回 JSON Schema，用于 LLM function calling 的参数描述
- `permission_level()` 决定调用前的审批流程
- `needs_sandbox()` 决定是否在 bubblewrap 沙箱中执行
- `exposure()` 控制工具对 LLM 的可见性（见 4.2）
- `concurrency_class()` 控制并行执行策略（见 4.3）
- `ToolContext` 提供当前会话上下文（工作目录、用户身份、环境变量）

### 2.3 沙箱执行流程

> 沙箱相关内容已拆分至 [sandbox.md](sandbox.md)，此处保留概述。

```
ToolUseBlock { name: "bash", input: {cmd: "make"} }
     │
     ▼
┌─────────────┐
│ 权限检查     │  L0→自动 L1→通知 L2→确认 L3→拒绝
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 沙箱创建     │  bubblewrap + seccomp + cgroups
│             │  namespace 隔离
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 执行命令     │  带超时、资源限制
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 收集结果     │  stdout + stderr + exit_code
│             │  + 副作用追踪
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 审计记录     │  写入 audit.jsonl
└──────┬──────┘
       │
       ▼
ToolResultBlock { content: "...", is_error: false }
```

### 2.4 沙箱配置

> 沙箱相关内容已拆分至 [sandbox.md](sandbox.md)，此处保留概述。

```yaml
bubblewrap:
  --ro-bind /usr /usr         # 系统只读
  --bind /home/user /home/user # 工作目录可写
  --tmpfs /tmp                 # 临时目录
  --unshare-net               # 默认无网络
  --die-with-parent           # 父进程死则子进程死

seccomp:
  禁止: mount, umount, reboot, kexec, ...
  允许: 文件操作, 进程管理, 网络 (按需)

cgroups:
  CPU: 50% 上限
  Memory: 2G 上限
  IO: best-effort
```

### 2.5 沙箱执行器

> 沙箱相关内容已拆分至 [sandbox.md](sandbox.md)，此处保留概述。

**SandboxExecutor** — 多后端沙箱执行器，自动选择最佳可用后端（BubblewrapBackend / ProcessBackend / NoopBackend）。
- 代码位置: `sandbox/executor.rs`
- 执行流程：创建 namespace + cgroup → 应用 seccomp filter → 执行命令 → 收集结果 → 清理

---

## 3. 已识别缺陷

### 3.1 P0: 工具输出的上下文管理

**问题描述：** 一次工具调用可能产生巨大的输出（例如 `cat /var/log/syslog`、`find /`、`make` 编译日志），这些输出直接进入 `ToolResultBlock.content`，可能一次性撑爆 LLM 的上下文窗口。

**当前行为：**
- 工具输出原样返回到 `ContentBlock::ToolResult { content: "...", is_error: false }`
- 没有大小限制，没有截断，没有溢出策略
- 一次 `find /` 可能返回数 MB 文本，直接导致 API 调用失败或上下文溢出

**影响：**
- 推理循环因 token 超限而中断
- 大量低质量输出淹没有价值信息
- 上下文压缩被迫频繁触发，浪费推理资源

**参考现状：**
- Codex 的 `output_token_limit` 机制：对工具输出设置 token 上限
- Hermes 的工具输出截断：超过阈值自动截断并提示 Agent

### 3.2 P1: 工具分层暴露 (ToolExposure)

**问题描述：** 当前所有工具平铺暴露给 LLM，没有分层。当工具数量增长（系统工具 + 感知工具 + 记忆工具 + MCP 工具 + 委托工具），LLM 的工具选择准确率下降，且工具定义本身占用大量上下文。

**当前行为：**
- 所有注册工具的 `description` + `args_schema` 都注入到系统提示中
- 没有"最近使用优先"或"按需发现"机制
- Agent 无法动态发现新工具

**参考现状：**
- Codex 的 `ToolExposure` 分层：Direct（始终可用）、Deferred（通过 `tool_search` 发现）、Hidden（系统内部使用）
- Hermes 的 progressive disclosure：根据任务上下文动态调整可用工具集

### 3.3 P1: 工具并行执行

**问题描述：** 当前工具调用是严格串行的。Agent 在一次推理中可能产生多个独立的只读工具调用（例如同时读取 3 个文件），串行执行浪费时间。

**当前行为：**
- `for tool_call in response.tool_calls()` 逐个执行
- 没有并发控制，没有依赖分析
- 只读操作之间没有并行优化

**参考现状：**
- OpenCode 的 `FiberSet`：并发执行工具调用，通过 joinset 管理
- Codex 的并行工具执行：只读工具始终并行，写入工具需检查路径冲突

### 3.4 P1: MCP 集成

**问题描述：** 当前工具系统是封闭的，只能使用内置工具。需要支持 Model Context Protocol (MCP) 以接入外部工具服务器。

**当前行为：**
- 没有 MCP 客户端
- 没有工具发现机制（MCP 的 `tools/list`）
- 没有传输层抽象（stdio / StreamableHTTP / SSE）

**参考现状：**
- Anthropic SDK 原生支持 MCP
- Codex 的 MCP 工具自动注册为 Deferred 级别

### 3.5 P2: MCP OAuth 不完整

**问题描述：** 当前 MCP client 仅实现了 Bearer Token 认证方式，但企业级 MCP server（如 Jira、Confluence、内部 API 网关）普遍要求 OAuth 2.0 认证流程。Bearer Token 是静态凭证，无法满足企业 SSO 集成（OIDC / SAML bridge）、Token 自动刷新（access token 过期后通过 refresh token 续期）、细粒度权限控制（OAuth scopes）以及多租户 MCP server 的用户级鉴权等场景。此外，`ToolOutput` trait 虽定义了多模态输出能力（图片、文件引用），但实际实现不完整——工具执行结果只能返回纯文本，无法返回图片或文件引用，未充分利用 MCP 协议的多模态 content block 能力。

**影响：**
- 企业可用性：无法接入企业内部 OAuth 保护的 MCP server
- 安全性：Bearer Token 长期有效，泄露后无自动轮换机制
- 工具能力：截图工具、图表工具等无法将结果以图片形式返回给 LLM

**来源文档：** `gap-analysis/phase-2/tool-system/mcp-oauth-and-multimodal-output.md`

### 3.6 P1: 沙箱后端可移植性

**问题描述：** 工具系统的沙箱执行强依赖 bubblewrap（bwrap）作为进程隔离机制。bubblewrap 需要 Linux user namespace 支持（`unshare(CLONE_NEWUSER)`），但在 Docker 容器（默认）、WSL2、Kubernetes Pod、systemd-nspawn、Android、嵌入式 Linux 等常见环境中受限或不可用。

**影响：**
- Docker 开发环境不可用（默认配置下 bwrap 无法工作）
- WSL2 用户受 namespace 限制影响
- Android 端完全不可用（无 bubblewrap）
- 开发者被迫使用 `--privileged` 破坏容器安全隔离

**来源文档：** `gap-analysis/phase-3/tool-system/sandbox-backend-portability.md`

### 3.7 P2: MCP 工具名 64 字节限制

**问题描述：** MCP 工具名规范化函数 `normalize_tool_name()` 将工具名限制在 64 字节以内，超出部分截断，碰撞时追加 SHA1 哈希后缀。截断后的名称对 LLM 不友好（语义丢失），SHA1 哈希后缀不可读，多服务器场景下碰撞概率高。

**影响：**
- LLM 工具选择准确率下降（截断和哈希后的工具名降低语义密度）
- 调试效率降低（需要额外映射步骤还原原始名称）

**来源文档：** `gap-analysis/phase-5/tool-system/mcp-tool-64byte-limit.md`

---

## 4. 改进设计

### 4.1 Bounded Output + Overflow to File

解决 S3.1 工具输出撑爆上下文的问题。参考 Hermes 的三层防御模型、OpenCode 的 head+tail 字节感知截断、Codex 的 per-call truncation policy。

#### 4.1.1 三层防御架构

```
Layer 1: Per-tool capture limits        (进程级，工具自限)
    ↓
Layer 2: Per-result persistence          (单结果级，溢出到文件)
    ↓
Layer 3: Per-turn aggregate budget       (回合级，聚合裁剪)
```

每层独立运作，前一层是后一层的前置过滤器。Layer 1 减少进入 Layer 2 的数据量；Layer 2 减少进入 Layer 3 的残余量；Layer 3 保证整回合上下文不超限。

#### 4.1.2 核心数据结构

- **CaptureConfig** — 沙箱执行器的捕获配置，stdout/stderr 最大捕获字节数（默认 1MB each）
- **SandboxResult** — 沙箱执行结果（分离 stdout/stderr），含 `stdout_truncated` / `stderr_truncated` 标记
- **OutputConfig** — 工具输出处理配置，含 `max_output_chars`（默认 100K）、`overflow_dir`、`truncation` 策略、`tool_overrides`、`pinned_thresholds`、`retention_days`
- **TruncationPolicy** — 截断策略，含 `head_lines`（默认 50）、`tail_lines`（默认 20）、可选字节预算
- **TurnBudgetConfig** — 回合级预算配置，`turn_budget_chars`（默认 200K）、`preview_chars`（默认 1500）
- **ProcessedOutput** — 处理结果枚举：`Inline { content }` 或 `Overflow { summary, overflow_path, total_chars }`

#### 4.1.3 Layer 1: Per-Tool Capture Limits

在进程级限制 stdout/stderr 捕获量（默认 1MB per stream），防止海量数据进入后续流程。捕获逻辑使用 UTF-8 安全截断。合并双流输出时 stderr 标记 `[stderr]` 前缀，任一流截断则整体标记截断。

#### 4.1.4 Layer 2: Per-Result Persistence

单个工具结果超限时，溢出到文件，返回 head+tail 摘要。阈值解析优先级：`pinned_thresholds > per-call override > tool_overrides > default`。

**`PINNED_THRESHOLDS` 的作用：** `file_read` 的阈值为 `usize::MAX`，意味着读取任何文件都不会触发持久化。这防止了"读取已持久化文件 -> 输出再次被持久化"的无限循环（Hermes 发现的关键 bug）。

#### 4.1.5 Layer 3: Per-Turn Aggregate Budget

一轮推理中可能有多个工具结果，每个都不超限，但总和超限（默认 200K chars）。`enforce_turn_budget` 对未溢出的结果按大小降序排列，优先持久化最大的。

#### 4.1.6 Output Cleanup Lifecycle

溢出文件需要定期清理（每小时），默认保留 7 天。

#### 4.1.7 Search Result Bounding

`file_search` 等搜索工具需要结果数量限制（默认 50）和截断标记，防止返回海量匹配。

#### 4.1.8 Per-Call Truncation Policy

不同的工具调用可以有不同的截断策略。解析优先级：call override > tool default > global default。

#### 4.1.9 Edge Cases

| 场景 | 处理方式 |
|------|----------|
| 溢出文件写入失败 | 回退到内联截断 + marker |
| 输出为空 | 直接返回 Inline |
| 输出全是二进制 | Layer 1 的 UTF-8 lossy 转换处理 |
| file_read 读取已持久化文件 | pinned_thresholds 为 usize::MAX，不触发二次持久化 |
| 单字符超大输出（无换行） | take_prefix_bytes 按字节截断 |
| 多工具结果总和刚好超限 | enforce_turn_budget 从最大的开始持久化 |

#### 4.1.10 与其他模块的集成

| 模块 | 集成点 |
|------|--------|
| cognitive-engine | `ToolResult.content` 作为 `ContentBlock::ToolResult` 写入消息历史 |
| memory-system | `ToolOutput::to_audit_json()` 供审计存储 |
| reasoning-loop | Layer 3 在推理循环的回合边界调用 |
| observability | `log_preview()` 供 telemetry 使用 |
| sandbox | Layer 1 的 `CaptureConfig` 传入沙箱执行器 |

### 4.2 ToolExposure 分层暴露

解决 S3.2 工具过多导致选择准确率下降的问题。参考 Codex 的 4 级 `ToolExposure` 枚举，Hermes 的 progressive disclosure + BM25 搜索 + 三工具桥接。

#### 4.2.1 ToolExposure 枚举

```rust
enum ToolExposure {
    Direct,           // 始终在模型可见工具列表中
    Deferred,         // 注册但初始不暴露，通过 tool_search 发现
    DirectModelOnly,  // 对模型可见，排除在 code-mode 嵌套工具之外
    Hidden,           // 仅系统内部调度使用
}
```

| 级别 | 模型可见 | Code-mode 嵌套 | 需要搜索元数据 | 典型用途 |
|------|---------|----------------|---------------|---------|
| `Direct` | 是 | 是 | 否 | 核心工具 |
| `Deferred` | 初始否 | 是 | 是 | MCP 工具、低频系统工具 |
| `DirectModelOnly` | 是 | 否 | 否 | delegate、权限敏感工具 |
| `Hidden` | 否 | 否 | 否 | 审计、内部状态查询 |

#### 4.2.2 渐进式披露与阈值门控

当 Deferred 工具的 schema 总量低于上下文窗口的 `threshold_pct` (10%) 时，`tool_search` 是空操作——所有工具直接通过。只有当 schema 超过阈值才激活桥接工具。估算使用 `CHARS_PER_TOKEN = 4.0` 的粗略换算。

#### 4.2.3 三工具桥接

Hermes 使用 3 个桥接工具替代所有 Deferred 工具：`tool_search`（搜索）、`tool_describe`（获取完整 schema）、`tool_call`（调用）。`tool_describe` 让模型在调用前加载完整 JSON Schema，避免盲调用。

**`tool_call` 的 Hook 透传：** 执行器在处理 `tool_call` 桥接工具时，应将内部实际工具名暴露给 hooks，使安全钩子和审计系统看到真实调用目标。

#### 4.2.4 BM25 搜索与目录索引

升级 `description.contains(query)` 子串匹配为 BM25 评分。使用 `CatalogEntry` 预分词索引（name 按 snake_case 拆词 + description + 参数名），标准 BM25 参数 k1=1.5, b=0.75，BM25 零结果时降级到子串匹配。

#### 4.2.5 工具集抽象 (Toolset)

Hermes 将工具组织为可组合的 toolset（core, system, perception, memory, network, full），支持包含链（带环检测）和会话级过滤。

#### 4.2.6 工具注册增强

`ToolRegistration` 增加工具集来源、`source` 分类、`last_used`/`use_count` 追踪、`check_fn` 可用性探针（带 30s TTL 缓存）、`dynamic_schema_overrides`、`max_result_size`。Shadow 保护防止不同 toolset 的同名工具互相覆盖。

#### 4.2.7 工具循环防护 (ToolGuardrails)

防止 Agent 陷入工具调用死循环。`ToolGuardrailController` 使用 `ToolCallSignature`（tool_name + args SHA256）检测三种模式：

| 模式 | 警告阈值 | 阻断阈值 |
|------|----------|----------|
| 相同工具+参数连续失败 | 2 | 5 |
| 同一工具任何参数连续失败 | 3 | 8 |
| 幂等工具返回相同结果 | 2 | 5 |

幂等工具白名单：`file_read`, `file_search`, `process_list`, `system_status` 等只读工具。

#### 4.2.8 暴露过滤与 Schema 组装

在推理循环的每次 LLM 调用前，根据当前暴露级别和工具集过滤组装最终的工具 schema 列表。

### 4.3 工具并行执行 (ToolCallExecutor)

解决 S3.3 串行执行效率低的问题。综合参考 Codex 的 `RwLock` 并行/串行门控、OpenCode 的流式即时派发、Hermes 的线程池 + 预检 + 结果排序模式。

#### 4.3.1 设计目标

1. **只读工具始终并行** — `file_read`, `file_search`, `system_status` 等无副作用工具最大化并发
2. **写入工具按路径冲突串行** — 同路径写入顺序执行，不同路径写入可并行
3. **副作用工具始终串行** — `bash_exec`, `service_control` 等未知副作用工具独占执行
4. **流式即时派发** — 工具调用在 LLM 流中到达时立即派发
5. **结果保序** — 返回顺序与 LLM 发出的 `tool_call` 顺序一致
6. **可中断** — 用户中断时能取消正在执行的工具
7. **最大并发限制** — 可配置上限（默认 8）

#### 4.3.2 工具并发分类

```rust
enum ConcurrencyClass {
    ReadOnly,                    // 只读，始终可并行
    Write { paths: Vec<PathBuf> }, // 写入，通过路径冲突检测决定并行/串行
    SideEffect,                  // 未知副作用，始终串行
}
```

| 工具 | ConcurrencyClass | 理由 |
|------|-----------------|------|
| `file_read` | `ReadOnly` | 纯读取 |
| `file_search` | `ReadOnly` | 纯搜索 |
| `system_status` | `ReadOnly` | 状态查询 |
| `file_write` | `Write { paths }` | 写文件，可声明目标路径 |
| `bash_exec` | `SideEffect` | 任意命令，副作用未知 |

#### 4.3.3 RwLock 并行门控

借鉴 Codex 的 `parallel_execution: Arc<RwLock<()>>` 模式：只读工具获取读锁（共享），串行工具获取写锁（独占）。`PathConflictDetector` 对写入工具的同目录路径使用信号量互斥。

#### 4.3.4 PathConflictDetector

写入工具的路径冲突检测：同目录（父目录级别）视为冲突，需串行执行。每个路径组使用容量 1 的 Semaphore。

#### 4.3.5 流式即时派发 (Eager Spawn)

工具调用在 LLM 流中到达时立即派发，不等流结束。第一个工具在第一个 `ToolCallStart` 事件到达时就开始执行。延迟收益：工具数量 >= 2 时，总延迟减少量约等于 `(N-1) * avg_tool_latency`。

#### 4.3.6 结果保序

无论工具完成顺序如何，返回结果必须与 LLM 发出的 `tool_call` 顺序一致。使用 `results[index] = ...` 模式收集后按 `original_index` 排序。

#### 4.3.7 预执行守卫管线 (Pre-flight Guardrail)

借鉴 Hermes 的三层阻断：在工具进入沙箱前执行守卫检查（作用域检查、插件钩子、毁灭性命令检测、L3 权限级别硬阻断）。被阻断的工具不消耗沙箱资源。

#### 4.3.8 取消传播 (CancellationToken)

两种取消模式：

| 工具 | CancelMode | 理由 |
|------|-----------|------|
| `file_read` | Immediate | 快速操作 |
| `bash_exec` | Graceful (3s) | 子进程需要清理时间 |
| `service_control` | Graceful (5s) | systemd 操作需完成 |
| MCP 工具 | Immediate | 网络超时已处理 |

#### 4.3.9 活动心跳 (Activity Heartbeat)

在并发工具执行期间每 5 秒报告状态（已完成数/总数/耗时），防止网关超时断开。

#### 4.3.10 整体数据流

```
LLM Stream
    ├── TextDelta ──→ accumulate text buffer
    └── ToolCallStart ──→ GuardrailPipeline.check()
                              │
                    ┌─────────┴─────────┐
                    │ Allow             │ Block
                    ▼                   ▼
              Classify              Error ToolCallResult
              concurrency_class()
                    │
        ┌───────────┼───────────┐
        ▼           ▼           ▼
    ReadOnly    Write       SideEffect
    │           │           │
    ▼           ▼           ▼
  read()     read()      write()
  lock       lock +      lock
             path_sem    (独占)
    │           │           │
    └───────────┼───────────┘
                ▼
         JoinSet::spawn() → CancellationToken.race()
                ▼
      collect_ordered_results() (保序)
                ▼
      Layer 2 → Layer 3 → ContentBlock::ToolResult
```

#### 4.3.13 Edge Cases

| 场景 | 处理方式 |
|------|----------|
| 所有工具都是 SideEffect | 全部串行 |
| 用户中断时工具正在执行 | cancel() 触发 CancellationToken + abort handles |
| LLM 流中途断开 | 已派发的工具继续执行到完成 |
| 工具执行 panic | JoinSet 捕获 JoinError，合成错误结果 |
| Write 工具路径为空 | 退化为 SideEffect |
| 同路径并发写入 | PathConflictDetector 信号量确保串行 |

### 4.4 MCP 集成

> MCP 集成相关内容已拆分至 [mcp-integration.md](mcp-integration.md)。

支持 MCP 的三种传输方式：stdio、StreamableHTTP、SSE。完整的生命周期管理由 `McpConnectionManager` 统一负责。

### 4.5 MCP OAuth 认证与多模态输出

> MCP 相关内容已拆分至 [mcp-integration.md](mcp-integration.md)。

Phase 2 保持 Bearer Token 作为唯一认证方式，但定义 OAuth 认证的 trait 占位；同时实现多模态工具输出。

### 4.6 沙箱后端可移植性

> 沙箱相关内容已拆分至 [sandbox.md](sandbox.md)。

定义 `SandboxBackend` trait，实现三种后端，运行时按环境自动选择。

### 4.7 MCP 工具名规范化改进

> MCP 相关内容已拆分至 [mcp-integration.md](mcp-integration.md)。

---

## 5. 实现要点

| 项目 | 说明 |
|------|------|
| **工具 Trait** | `agent-core/src/tool.rs` — Tool trait + ToolRegistry + `ToolOutput` trait |
| **输出处理** | `agent-core/src/tool/output/` — 三层防御: `capture.rs` (Layer 1), `persistence.rs` (Layer 2), `turn_budget.rs` (Layer 3) |
| **截断策略** | `agent-core/src/tool/output/truncation.rs` — `TruncationPolicy` + UTF-8 安全的 head/tail 切分 |
| **多模态输出** | `agent-core/src/tool_result.rs` — `ToolContent` enum + `ToolResult` |
| **ToolExposure** | `agent-core/src/tool.rs` — 4 级枚举 + `visible_tools()` |
| **阈值门控** | `agent-core/src/tool_search/config.rs` — `ToolSearchConfig`, `should_activate_tool_search()` |
| **三工具桥接** | `agent-core/src/tool_search/bridge.rs` — `ToolSearchTool` + `ToolDescribeTool` + `ToolCallBridge` |
| **BM25 搜索** | `agent-core/src/tool_search/catalog.rs` — `CatalogEntry`, `ToolCatalog` |
| **工具集** | `agent-core/src/toolset.rs` — `Toolset`, `ToolsetRegistry` |
| **注册增强** | `agent-core/src/tool.rs` — shadow 保护, `check_fn` TTL 缓存, `dynamic_schema_overrides` |
| **工具循环防护** | `agent-core/src/tool_guardrails.rs` — `ToolGuardrailController` |
| **并行执行** | `agent-core/src/tool_runner.rs` — ToolCallExecutor (RwLock gate + JoinSet + CancellationToken) |
| **回合预算** | `agent-core/src/tool/output/turn_budget.rs` — `enforce_turn_budget()` |
| **溢出清理** | `agent-core/src/tool/output/persistence.rs` — `cleanup_overflow_dir()` 7 天保留 |
| **MCP 客户端** | `agent-core/src/mcp/client.rs` — McpClient + 三种 Transport |
| **MCP 连接管理器** | `agent-core/src/mcp/manager.rs` — McpConnectionManager |
| **MCP 工具适配** | `agent-core/src/mcp/tool_adapter.rs` — McpToolWrapper + normalize_tool_name |
| **MCP 配置** | `agent-core/src/mcp/config.rs` — McpServerConfig |
| **MCP 错误** | `agent-core/src/mcp/error.rs` — McpError enum |
| **MCP 资源/提示词** | `agent-core/src/mcp/resources.rs` — list_all_resources + read_resource |
| **沙箱执行器** | `agent-core/src/sandbox.rs` — SandboxExecutor + `CaptureConfig` + `SandboxResult` |
| **MCP OAuth** | `agent-core/src/mcp/auth.rs` — `McpAuthProvider` trait + BearerTokenAuth + OAuthAuth 骨架 |
| **沙箱后端** | `agent-core/src/sandbox/backend.rs` — `SandboxBackend` trait + 三后端 |
| **工具名配置** | `agent-core/src/mcp/tool_name.rs` — `ToolNameConfig` + `CollisionStrategy` |
| **内置工具** | `agent-tools/src/` — bash, file_ops, system, memory_tools, delegate, file_search |

---

## 6. 参考来源

| 来源 | 关键内容 | 借鉴内容 |
|------|----------|----------|
| Codex | `ToolExposure` enum | 4 级暴露: Direct / Deferred / DirectModelOnly / Hidden |
| Codex | `ToolExecutor<Invocation>` trait | `exposure()`, `search_info()`, `supports_parallel_tool_calls()` |
| Codex | `output_token_limit` | 工具输出 token 限制 |
| Codex | `RwLock<()>` parallel gating | read-lock=shared, write-lock=exclusive |
| Codex | CancellationToken + terminal outcome | Two-phase abort: immediate vs graceful |
| Hermes | Progressive disclosure | 阈值门控: threshold_pct=10%, auto/on/off 模式 |
| Hermes | 三工具桥接 | `tool_search` + `tool_describe` + `tool_call` |
| Hermes | BM25 搜索 | `CatalogEntry` 预分词, k1=1.5 b=0.75 |
| Hermes | `ToolCallGuardrailController` | 精确失败/同工具失败/幂等无进展检测 |
| Hermes | 三层输出防御 | per-tool cap / per-result persist / per-turn 200K budget |
| Hermes | Concurrent tool execution | ThreadPoolExecutor (max 8), per-thread interrupt |
| OpenCode | `tool-output-store.ts` | `ToolOutputStore`: write/truncate/bound/cleanup |
| OpenCode | Head+tail preview | 按行数+字节预算的 head/tail 切分, UTF-8 安全 |
| OpenCode | `MAX_CAPTURE_BYTES = 1MB` | 进程级 stdout/stderr 独立捕获限制 |
| OpenCode | `FiberSet` (tokio JoinSet) | 并发工具调用管理 |
| OpenCode | Stream-integrated eager spawn | 工具在 LLM 流中到达时立即派发 |
| Anthropic SDK | MCP protocol support | stdio / StreamableHTTP / SSE 传输 |

---

## Implementation Summary

**Code Locations:**
- `argos/crates/agent-core/src/tool/mod.rs` — Tool trait definition, ToolRegistry, ToolOutput trait
- `argos/crates/agent-core/src/tool/registry.rs` — Registration, lookup, visibility filtering
- `argos/crates/agent-core/src/tool/bash_exec.rs`, `file_read.rs`, `file_write.rs`, `process_list.rs`, `system_status.rs` — 5 built-in tools
- `argos/crates/agent-core/src/tool/output/` — Three-layer output defense: `capture.rs`, `persistence.rs`, `turn_budget.rs`, `truncation.rs`, `pruner.rs`, `config.rs`

**Key Types/Traits Implemented:**
- `Tool` trait — unified tool interface with `input_schema()`, `permission_level()`, `needs_sandbox()`, `exposure()`, `concurrency_class()`
- `ToolRegistry` — registration, lookup, visibility filtering
- `ToolOutput` trait — output lifecycle: `log_preview()`, `to_model_content()`, `to_audit_json()`
- Output defense pipeline — `capture_output()`, `process_result()`, `enforce_turn_budget()` in `tool/output/`

**Test Coverage:** 120 unit tests + 6 integration tests in `tests/output_defense.rs`. Covers capture, truncation, persistence, turn budget, pruner, and compressor.

**Not Yet Implemented:** ToolExposure 4-level system, BM25 tool search, Toolset composition, ToolGuardrails, ToolCallExecutor parallel execution, MCP integration, sandbox backend multi-backend support.
