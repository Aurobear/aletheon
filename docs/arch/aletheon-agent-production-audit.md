# Aletheon 生产级 Agent 与通用 Runtime 架构审计

> 文档版本：2.1
>
> 更新日期：2026-07-19
>
> 审计对象：Aletheon `dev` 分支，提交 `294e76c`
>
> 范围：主 Agent、工具系统、subagent、Pi Runtime、通用 Runtime 接口、Host Platform、Hardware Control、生产验证

---

## 0. 文档目的与核心决策

本文解决五个问题：

1. 为什么 Aletheon 当前表现为“代理能力弱”。
2. 文件搜索、代码定位、编辑、Shell、Git、测试等工具还缺什么。
3. Pi、Codex、Grok Build、Hermes 或其他 Agent 应如何被 Aletheon 调用，而不与具体实现耦合。
4. Aletheon 应该自己实现 Coding Agent，还是以 Pi 为默认编码执行器。
5. Linux/Windows/macOS 宿主能力与机器人/设备控制应如何解耦并分别落地。

核心结论：

> Aletheon 已经有 Agent OS 控制平面的主体，但编码执行平面存在多处未接通。短期应让 Pi 成为受 Aletheon 管理的默认 Coding Runtime；长期通过通用 Capability Runtime Framework 接入 Pi、Codex、Grok Build、Hermes、ROS 和 Native Cognit，而不是在 Executive、Goal 或 Kernel 中继续加入具体 Runtime 特判。

推荐职责划分：

```text
Executive：路由、生命周期、预算、恢复和治理
Cognit：理解任务、形成 WorkOrder、设置验收条件、选择能力
Dasein：身份、用户关系和长期目标约束
Agora：共享任务状态、证据和多 Agent 协作
Mnemosyne：长期记忆、经验和技能
Kernel：Agent、Process、Operation、Capability、Sandbox、Checkpoint
Host Platform：Linux/Windows/macOS 的进程、文件、网络、Sandbox、服务和桌面能力
Hardware Control：设备对象、Provider、遥测、控制租约与机器人安全边界
Capability Runtime：Pi / Codex / Grok / Hermes / Native Cognit / ROS
Verifier：根据证据决定任务是否真正成功
```

第一目标仍是完成可靠的“定位 → 修改 → 测试 → 修复 → 验证”闭环。与此同时可以独立推进 Hardware API 与模拟器；Android、远程平台和真实机器人执行器不应早于相应的 Host、安全租约、模拟/HIL 门槛。

---

## 1. 当前状态判断

### 1.1 Aletheon 已经具备什么

Aletheon 当前并不是空壳，已经包含：

- Kernel Agent admission、预算、层级、Process、Operation 和 supervision。
- Executive daemon、Turn pipeline、AgentControlService 和持久化。
- Agent Profile loader 和工具白名单。
- Corpus 工具注册、Sandbox、审批、审计和能力执行。
- Cognit ReActLoop、压缩接口、reflection、tool budget 和 optional verifier seam。
- Dasein、Agora、Mnemosyne 等长期主体性和认知模块。
- Native Cognit child runtime。
- Pi one-shot coding runtime 和 Pi resident RPC runtime。
- Goal worker/reviewer runtime。
- worktree、安全恢复和部分 verification infrastructure。

问题不是“没有架构”，而是核心用户路径没有把这些能力完整连起来。

### 1.2 当前成熟度分布

| 领域 | 成熟度 | 结论 |
| --- | --- | --- |
| Daemon/持久化 | 中高 | 基础设施较强 |
| Agent 生命周期 | 中高 | 已有较完整控制面 |
| Sandbox/审批 | 中高 | 方向正确，但特性开关和工具路径不一致 |
| Native Agent Loop | 中低 | 能运行，但 Profile、上下文、并行和验证不足 |
| 编码工具 | 中低 | 工具存在，但行为不统一、缺关键能力 |
| subagent | 中 | 控制平面存在，主 Agent 默认不可达 |
| Pi 集成 | 中 | 两条路径各完成一半，没有形成默认生产链路 |
| Runtime 解耦 | 低到中 | 有通用骨架，具体 Runtime 仍被多处特判 |
| 编码任务评测 | 低 | 缺真实成功率门禁 |

### 1.3 为什么“代码很多，但 Agent 仍然弱”

当前工程投入较多集中在：

- 控制结构。
- 安全机制。
- 状态对象。
- 生命周期。
- 设计模块。

但编码成功率主要取决于另一组能力：

- 模型能否看到正确工具。
- 工具是否在正确 workspace 工作。
- 工具结果是否完整回到模型。
- Agent 是否有足够迭代预算。
- Agent 是否能持续修改和恢复错误。
- 是否加载项目指令。
- 是否能运行长命令、测试和 diagnostics。
- 是否有 verifier 阻止虚假完成。

Aletheon 当前正是在这些路径上存在断点。

---

## 2. 当前 Agent 调用链与断点

### 2.1 理想调用链

```text
用户请求
  ↓
Executive 解析 Thread/Workspace/Principal
  ↓
解析完整 Agent Profile
  ↓
Cognit 生成任务计划与工具调用
  ↓
Governed Capability 进行授权和 Sandbox 执行
  ↓
工具结果和证据进入 Trajectory
  ↓
Cognit 继续迭代
  ↓
Verifier 检查 diff、测试和验收条件
  ├─ 失败：把结构化证据送回同一 Agent
  └─ 通过：生成 Verified RuntimeReceipt
```

### 2.2 当前主要断点

```text
Agent Profile max_iterations
  └─ 默认 0 被错误折算为 1

Active Profile
  └─ 只应用 allowed_tools，Prompt/模型/预算未完整进入主 Turn

Agent control tools
  └─ Profile 编译后才注册，主 Agent 默认看不到

Pi Runtime
  └─ 已注册但主 Cognit 没有可见入口，production 默认关闭

文件搜索
  └─ file_search 相对路径可能基于 daemon cwd

工具输出
  └─ 单结果仅保留 8 KB，跨 Turn 又丢失工具结构

执行循环
  └─ 多工具顺序执行，并行 batching 没有接入生产路径

完成语义
  └─ 无工具调用即可结束，生产 verifier 未接入
```

---

## 3. P0：直接影响代理能力的确定性问题

### 3.1 `max_iterations = 0` 被解析成 1

默认配置：

```toml
[agent]
max_iterations = 0
```

Cognit ReActLoop 把 `0` 定义为不限制迭代。但 Profile loader 使用：

```rust
let max_iterations = overrides
    .and_then(|ov| ov.max_iterations)
    .unwrap_or(role.max_iterations)
    .min(config.max_iterations)
    .max(1);
```

结果：

```text
role.max_iterations.min(0).max(1) == 1
```

所以 `code-agent` 的 20 次、`admin-agent` 的 50 次都会变成一次。

直接影响：

- child Agent 很难完成一次以上的模型—工具循环。
- 无法形成读取、修改、测试、修复的连续行为。
- 工具报错后没有恢复空间。
- subagent 外观表现为“一次回答就结束”。

建议统一上限类型，不再用裸 `usize` 同时表达 unlimited：

```rust
pub enum IterationLimit {
    Unlimited,
    Limited(NonZeroUsize),
}
```

如果短期保持整数：

```rust
fn combine_limits(profile: usize, global: usize) -> usize {
    match (profile, global) {
        (0, 0) => 0,
        (0, global) => global,
        (profile, 0) => profile,
        (profile, global) => profile.min(global),
    }
}
```

证据文件：

- `config/default.toml`
- `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- `crates/cognit/src/harness/linear/mod.rs`

### 3.2 主 Agent Profile 没有完整生效

当前主 Turn 的不可变 Profile 快照只有：

```rust
pub struct ActiveAgentProfileSnapshot {
    pub profile_name: String,
    pub allowed_tools: HashSet<String>,
}
```

而完整 Agent Profile 已经拥有：

- `system_prompt`
- `model`
- `allowed_tools`
- `max_iterations`
- input/output token limits
- `max_tool_calls`
- `max_elapsed_ms`
- risk tier
- approval policy
- tool timeout

主 Turn 仍然使用：

```rust
model_policy: None
```

因此当前 `code-agent` 对主 Agent 的实际作用主要是工具白名单，而不是完整行为、模型和预算 Profile。

应建立一次性解析的：

```rust
pub struct ResolvedTurnProfile {
    pub id: AgentProfileId,
    pub system_prompt: String,
    pub model_policy: ModelPolicy,
    pub tool_policy: ToolPolicy,
    pub iteration_limit: IterationLimit,
    pub budget: RuntimeBudget,
    pub approval_policy: ApprovalPolicy,
    pub verifier_policy: VerifierPolicy,
}
```

同一份快照必须供以下模块使用：

- ContextAssembler
- ModelRouter
- Cognit harness
- Capability disclosure
- Capability execution
- Agent admission
- Verifier
- Observability

### 3.3 默认 Prompt 仍是通用聊天助手 Prompt

当前默认 Prompt 近似：

```text
You are a helpful AI assistant with tools. Use tools when appropriate.
```

`agents/code-agent.md` 也只描述基础六步流程，没有强制：

- 读取项目指令。
- 先检查工作区和已有修改。
- 实际修改而非只建议。
- 修改后测试。
- 测试失败继续修复。
- 未验证不得宣称完成。
- 保留用户未提交工作。
- 长任务维护可恢复状态。
- 用证据报告阻塞。

Prompt 不能替代 verifier，但当前连最低限度的编码行为契约都不足。

### 3.4 主生产回路没有 Completion Verifier

ReActLoop 有 optional verifier seam，但默认 `None`，主生产 bootstrap 没有安装 Coding Verifier。

当前核心完成条件实际上是：

```text
模型没有继续请求工具 → 接受自然语言为最终答案
```

对编码任务，这只能表示模型想停止，不能表示任务成功。

必须把完成状态拆分为：

```rust
pub enum CompletionStatus {
    SucceededVerified,
    SucceededUnverified,
    FailedVerification,
    Blocked,
    BudgetExhausted,
    Cancelled,
}
```

---

## 4. subagent 与 Runtime 可达性

### 4.1 当前注册顺序

关键启动顺序：

1. 注册普通 Corpus tools。
2. 获取 ToolDefinition catalog。
3. 加载和编译 Agent Profiles。
4. 注册 Native Cognit、Goal 和 Pi runtimes。
5. 创建 AgentControlService。
6. 最后注册 `agent_spawn/wait/send/cancel/list` 和 `agent`。

Profiles 在第 3 步已经固化工具白名单，无法引用第 6 步才出现的 Agent control tools。

### 4.2 默认 Profile 没有任何委派工具

现有 Profile 都没有：

- `agent`
- `agent_spawn`
- `agent_wait`
- `agent_send`
- `agent_cancel`
- `agent_list`

主 Turn 又会严格执行 Profile 过滤，因此后注册的工具仍然不会呈现给默认 `code-agent`。

结论：

> 当前默认主 Agent 无法创建 subagent，也无法主动选择 `pi-rpc`。

### 4.3 高层 `agent` 工具固定 Native Cognit

兼容 `agent` 工具内部使用固定 Runtime：

```rust
NativeCognitRuntime::runtime_id()
```

所以它不是通用委派接口。真正支持 runtime ID 的是低层 `agent_spawn`，但该工具对主 Agent 不可见。

### 4.4 推荐修复

启动顺序拆成两段：

1. 在 Profile 编译前注册稳定的 Agent control definitions。
2. Profile Catalog 完成后注册依赖 Profile 的高层 delegate tools。

主模型优先看到高层接口：

```text
delegate_code
delegate_review
delegate_research
```

而不是直接要求模型正确填写复杂、敏感的底层 `AgentSpawnRequest`。

底层 `agent_spawn` 可以保留给受信任的 orchestrator 或高级 Profile。

---

## 5. 编码工具系统审计

### 5.1 当前工具矩阵

| 能力 | 当前实现 | 主要问题 | 建议 |
| --- | --- | --- | --- |
| 文件读取 | `file_read` | 路径边界、hash、编码和输出预算不足 | 重构为 workspace_read |
| 文件覆盖 | `file_write` | 无 stale edit 检测 | 增加 expected hash |
| 结构化修改 | `apply_patch` | 缺事务、checkpoint 和验证 | 保留并增强 |
| 内容搜索 | `grep` | 全局 limit 错误 | 合并到 workspace_search |
| 内容搜索 | `file_search` | cwd 错误，与 grep 重复 | 合并或移除 |
| 文件发现 | `glob` | 相对 root、ignore、性能和 cursor | 改为 find/list |
| 代码图 | `code_graph` | Rust-only、非语义、cwd 错误 | 降级或接 LSP |
| Shell | `bash_exec` | schema 与生产执行路径分裂 | CommandHost + session API |
| Task | `task_*` | daemon 共享内存、非持久 | 映射 Kernel Operation |
| Git | 无专用工具 | 依赖 Shell 文本 | 增加结构化 Git tools |
| Diagnostics | 无 | 无法可靠定位编译/LSP 错误 | 增加 diagnostics |
| Test | 无语义工具 | 只靠 Shell | 增加 test_run/receipt |
| Artifact | 输出层有雏形 | 模型无法稳定分页读取 | 增加 artifact_read |
| 项目指令 | 未发现生产实现 | 不加载 AGENTS.md | 增加 InstructionResolver |

### 5.2 `file_search` 使用错误工作目录

`file_search` 接收 `ToolContext`，但执行 `rg`、`grep` 和 `find` 时没有：

```rust
.current_dir(&ctx.working_dir)
```

默认 `path = "."` 因而可能基于 daemon 启动目录，而不是当前用户 workspace。

表现：

- 正确查询返回空结果。
- 搜索到错误仓库。
- 测试使用绝对临时路径时通过，生产相对路径失败。

证据文件：

- `crates/corpus/src/tools/tools/file_search.rs`
- 对照正确使用 cwd 的 `crates/corpus/src/tools/tools/grep.rs`

### 5.3 搜索的 `max_results` 不是全局上限

`grep` 和 `file_search` 的 ripgrep 路径使用：

```bash
rg --max-count N
```

这个参数限制每个文件，不限制所有文件的总结果。实现随后收集完整 stdout，没有真正 `.take(max_results)`。

影响：

- 大仓库输出失控。
- 模型只看到后续 8 KB head/tail 片段。
- 中间的关键命中被丢弃。

正确实现：

- 使用 ripgrep JSON stream。
- 逐条结构化解析。
- 达到全局 limit 后停止收集或终止子进程。
- 返回 continuation cursor。
- 返回匹配文件数、匹配总数和截断状态。

### 5.4 `grep`、`file_search` 和 `glob` 应统一

推荐公共 API：

```rust
pub struct WorkspaceSearchRequest {
    pub mode: SearchMode, // Content | Files
    pub query: String,
    pub path: RelativeWorkspacePath,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub context_lines: u32,
    pub limit: NonZeroU32,
    pub cursor: Option<SearchCursor>,
}
```

Linux Platform 可使用 ripgrep，但 Agent API 不应直接复制 ripgrep 参数。

### 5.5 `glob` 的具体问题

- 传入相对 `root` 时不一定与 workspace cwd 合并。
- 没有统一排除 `.git`、`target`、`node_modules`。
- 自写递归 matcher，边界和性能风险高。
- 固定 1000 结果，无 cursor。
- 缺少稳定排序、深度和元数据。

### 5.6 `code_graph` 不是语义代码图

当前 `code_graph`：

- 只支持 Rust grammar。
- 没有使用 `ToolContext`。
- 相对路径基于 daemon cwd。
- callers 只是语法 `call_expression` 搜索。
- refs 更接近标识符节点匹配。
- 不处理模块、类型、宏和 trait dispatch。

短期应改名为 `rust_syntax_search` 或移出默认工具面。真正语义能力应通过 LSP：

- rust-analyzer
- clangd
- pyright
- TypeScript language server
- definition/references/workspace symbol/diagnostics

### 5.7 `file_read` 的边界和结果契约不足

已有 offset、limit 和行号，但缺少：

- workspace confinement。
- symlink escape 检查。
- 文件 hash 和 mtime。
- total lines。
- 二进制和编码检测。
- 可分页 artifact。

建议响应：

```json
{
  "path": "src/main.rs",
  "start_line": 1,
  "end_line": 200,
  "total_lines": 842,
  "sha256": "...",
  "content": "..."
}
```

### 5.8 编辑需要 optimistic concurrency

`file_write` 直接覆盖，容易根据旧上下文覆盖用户或其他 Agent 的新修改。

建议所有编辑型工具支持：

```rust
expected_sha256: Option<String>
```

发生冲突时返回 `StaleWorkspaceView`，要求 Agent重新读取，而不是静默覆盖。

`apply_patch` 已支持 structured patch 和 `patch_delta`，应保留并增加：

- expected hashes。
- 多文件事务或明确部分成功语义。
- patch 前 checkpoint。
- patch 后 authoritative diff artifact。
- format/diagnostics receipt。

### 5.9 `bash_exec` 有两套不一致执行路径

`BashExecTool` 自己实现：

- `timeout_seconds`
- streaming
- stdout/stderr 捕获
- 大输出 overflow

但 `ToolRunnerWithGuard` 对 `bash_exec` 特判，直接调用 SandboxExecutor，并使用另一套硬编码超时和结果组装。

可能导致：

- schema 中 timeout 在生产路径不生效。
- 工具实现中的 overflow 被绕开。
- streaming/non-streaming 行为不一致。
- 结果最终仍被 Cognit 截成 8 KB。

应抽象单一 `CommandHost`，由 Sandbox backend 实现隔离，工具层只保留一套 schema、超时、输出和 artifact 逻辑。

### 5.10 缺少持久终端

生产编码任务需要：

```text
exec_start
exec_poll
exec_write
exec_kill
```

支持：

- 长构建和长测试。
- PTY/交互 stdin。
- 后台服务。
- 增量日志。
- steering 时取消。
- daemon 恢复后的进程状态。

### 5.11 Task tools 不应使用 daemon 全局 HashMap

当前 task tools：

- daemon 级共享。
- 不按 session/root Agent 隔离。
- 重启丢失。
- 不与 Kernel Operation/Goal 统一。

应改为 Kernel durable operation graph 的受控客户端，而不是独立 Todo 系统。

---

## 6. 上下文、并行与工具证据

### 6.1 主历史只保留 6 条文本消息

当前 `MAX_HISTORY_MESSAGES = 6`，历史转换又会丢掉原始 tool use/result blocks。

因此跨 Turn 容易丢失：

- 已读文件。
- 已执行命令。
- 测试失败。
- 修改原因。
- subagent 结果。
- 用户 steering 前状态。

应持久保存完整 trajectory，模型上下文由 compaction 选择，而不是先破坏性转换为最后 6 条纯文本。

### 6.2 单个工具结果模型可见上限只有 8 KB

```rust
MAX_TOOL_RESULT_BYTES = 8_000
```

head/tail 截断优于只保留头部，但仍不足以可靠承载编译错误、搜索结果、大 diff 和测试失败。

正确架构：

```text
完整工具输出 → Artifact Store
模型上下文   → 结构化摘要 + ArtifactRef
需要细节     → artifact_read 分页读取
```

### 6.3 并行 batching 已实现但没有接入生产循环

`partition_tool_calls` 已能把只读工具分为 Parallel batch，但实际 ReActLoop 仍按顺序 `for` 执行。

应并行执行无依赖只读调用：

- read
- list/find/grep
- git status/log/show
- memory search
- web fetch/search

写入、Shell、测试和具有依赖关系的调用保持顺序。

---

## 7. 当前通用 Runtime 层：有骨架，但未真正解耦

### 7.1 已有通用部分

Aletheon 已经具有正确方向的基础类型：

| 接口/类型 | 当前作用 |
| --- | --- |
| `AgentControlPort` | spawn、wait、send、cancel、list |
| `AgentSpawnRequest.runtime_id` | 指定目标 Runtime |
| `AgentRuntimeRegistry` | 根据 RuntimeId 注册和解析实现 |
| `AgentRuntimeLauncher` | 通用 launch 入口 |
| `AgentRuntimeEvent` | Started、Progress、Tool、Terminal |
| `AgentResult` | output、usage、evidence、artifacts |
| `SubAgentRuntime` | 旧 runtime 兼容层 |

理论调用已经可以写成：

```rust
AgentSpawnRequest {
    runtime_id: RuntimeId("pi-rpc".into()),
    profile_id: AgentProfileId("code-agent".into()),
    task: "...".into(),
    ..
}
```

所以项目不是没有通用层，而是当前通用层只覆盖了生命周期的一小部分。

### 7.2 仍然存在的具体耦合

#### Bootstrap 直接构造 Pi

daemon bootstrap 直接调用：

```text
PiRuntime::prepare
PiRpcRuntime::prepare
register_pi_runtime
```

增加 Codex、Hermes、Grok Build 时仍需修改 daemon composition root。

#### Goal Coordinator 特判 `pi-coder`

Goal 路径根据固定 `PI_CODER_RUNTIME_ID` 判断编码任务，并解析 `PiAttemptRequest`。

Goal 层因此知道 Pi 的具体协议，不符合 Runtime 可替换性。

#### AgentControlService 根据名称判断 Pi

当前存在通过 runtime ID 是否包含 `pi` 来选择 storage/worktree 行为的逻辑。

这种字符串判断应由 Runtime Manifest 声明的 workspace/persistence capability 取代。

#### 高层 `agent` 工具固定 Native Cognit

高层委派接口不能选择 Runtime，底层可选择 Runtime 的工具又不可见。

#### Pi Adapter 没有执行完整通用策略

`AgentSpawnRequest` 中已有 tool allowlist、Profile 和多种预算，但 Pi RPC adapter 主要使用 task、workspace 和 elapsed timeout，没有完整映射其余策略。

### 7.3 结论

> 当前 `AgentRuntimeLauncher + Registry` 是插件点，不是完整的 Runtime Protocol。要实现真正不耦合，需要加入 Runtime Manifest、通用 WorkOrder、标准事件、标准 Receipt、Runtime Broker 和 transport adapters。

---

## 8. 目标：Capability Runtime Framework

### 8.1 设计原则

1. Executive、Goal、Kernel 不认识 Pi/Codex/Hermes 的具体类型。
2. 上层传递 WorkOrder 和能力要求，不传 Pi 专用请求。
3. Runtime Adapter 负责把通用协议翻译成 CLI、RPC、ACP、MCP 或 SDK。
4. Runtime 输出必须投影为标准事件和 Receipt。
5. Runtime 是否能逐工具拦截、是否可恢复、如何管理 workspace，都由 Manifest 声明。
6. 用户可以明确指定 Runtime，也可以让 Broker 自动选择。

### 8.2 推荐 crate 结构

```text
crates/
  runtime-api/
    manifest.rs
    capability.rs
    work_order.rs
    lifecycle.rs
    events.rs
    receipt.rs
    transport.rs

  runtime-broker/
    registry.rs
    selector.rs
    health.rs
    policy.rs

  runtime-native-cognit/
  runtime-pi/
  runtime-codex/
  runtime-grok-acp/
  runtime-hermes/
```

依赖规则：

```text
runtime-api 不依赖任何具体 Runtime
runtime-pi 只依赖 runtime-api 和 Pi transport 实现
runtime-codex 只依赖 runtime-api 和 Codex transport 实现
Executive 依赖 runtime-api + runtime-broker
Goal 不直接依赖 Pi 类型
Kernel 不判断 pi-rpc 字符串
```

### 8.3 Runtime Manifest

每个 Runtime 声明自己的能力，而不是让上层通过名字猜测：

```rust
pub struct RuntimeManifest {
    pub id: RuntimeId,
    pub aliases: Vec<RuntimeAlias>,
    pub display_name: String,
    pub implementation_version: String,

    pub capabilities: BTreeSet<RuntimeCapability>,
    pub interaction_modes: BTreeSet<InteractionMode>,
    pub transports: BTreeSet<RuntimeTransport>,

    pub workspace_mode: WorkspaceMode,
    pub resumability: RuntimeResumability,
    pub tool_governance: ToolGovernance,
    pub concurrency: RuntimeConcurrency,
}
```

Pi Manifest 示例：

```rust
RuntimeManifest {
    id: RuntimeId("pi/coding".into()),
    aliases: vec![RuntimeAlias("pi".into())],
    capabilities: set![
        CodeRead,
        CodeSearch,
        CodeEdit,
        Shell,
        Test,
    ],
    interaction_modes: set![
        OneShot,
        Resident,
        Steering,
        FollowUp,
    ],
    transports: set![JsonlStdio],
    workspace_mode: WorkspaceMode::IsolatedWorktree,
    resumability: RuntimeResumability::Session,
    tool_governance: ToolGovernance::Observed,
}
```

### 8.4 通用 WorkOrder

Goal/Cognit 不应创建 `PiAttemptRequest`，而应创建：

```rust
pub struct WorkOrder {
    pub objective: String,
    pub task_kind: TaskKind,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub context: ContextBundle,
    pub workspace: WorkspaceRef,
    pub required_capabilities: BTreeSet<RuntimeCapability>,
    pub verification: VerificationPlan,
}
```

示例：

```rust
WorkOrder {
    objective: "修复 max_iterations=0 被解析为 1",
    task_kind: TaskKind::CodeModification,
    acceptance_criteria: vec![
        "0 保持 unlimited 语义",
        "非零上限取 profile/global 较小值",
        "加入回归测试",
    ],
    required_capabilities: set![
        CodeRead,
        CodeSearch,
        CodeEdit,
        Shell,
        Test,
    ],
    ..
}
```

Pi Adapter 把它转换成 Pi Prompt；Codex Adapter 转成 Codex Turn；Grok Adapter 转成 ACP session；Hermes Adapter 转成 Hermes Session。

### 8.5 RuntimeLaunchSpec

```rust
pub struct RuntimeLaunchSpec {
    pub work_order: WorkOrder,
    pub workspace: WorkspacePolicy,
    pub instructions: InstructionBundle,
    pub tool_policy: ToolPolicy,
    pub model_policy: ModelPolicy,
    pub budget: RuntimeBudget,
    pub network_policy: NetworkPolicy,
    pub checkpoint_policy: CheckpointPolicy,
    pub verification_policy: VerificationPolicy,
}
```

### 8.6 生命周期接口

```rust
#[async_trait]
pub trait CapabilityRuntime: Send + Sync {
    fn manifest(&self) -> &RuntimeManifest;

    async fn health(&self) -> Result<RuntimeHealth>;

    async fn prepare(
        &self,
        spec: RuntimeLaunchSpec,
    ) -> Result<PreparedRuntime>;

    async fn start(
        &self,
        prepared: PreparedRuntime,
        events: Arc<dyn RuntimeEventSink>,
    ) -> Result<RuntimeHandle>;

    async fn send(
        &self,
        handle: &RuntimeHandle,
        message: RuntimeMessage,
    ) -> Result<MessageReceipt>;

    async fn snapshot(
        &self,
        handle: &RuntimeHandle,
    ) -> Result<RuntimeSnapshot>;

    async fn checkpoint(
        &self,
        handle: &RuntimeHandle,
    ) -> Result<CheckpointRef>;

    async fn cancel(
        &self,
        handle: &RuntimeHandle,
    ) -> Result<()>;

    async fn settle(
        &self,
        handle: RuntimeHandle,
    ) -> Result<RuntimeReceipt>;
}
```

### 8.7 标准消息

```rust
pub enum RuntimeMessage {
    Steer(String),
    FollowUp(String),
    ProvideContext(ContextBundle),
    VerificationFailure(VerificationFailure),
    UserAnswer(String),
}
```

Pi RPC 的 steer/follow-up、Codex steering 和其他 Agent mailbox 都映射成这一层语义。

### 8.8 标准事件

```rust
pub enum RuntimeEvent {
    Started(RuntimeStarted),
    AssistantDelta(TextDelta),

    ToolRequested(ToolRequestEvent),
    ToolStarted(ToolStartedEvent),
    ToolProgress(ToolProgressEvent),
    ToolCompleted(ToolCompletionEvent),

    FileChanged(FileChangeEvent),
    CommandStarted(CommandStartedEvent),
    CommandOutput(CommandOutputEvent),
    Diagnostic(DiagnosticEvent),
    TestResult(TestResultEvent),

    CheckpointCreated(CheckpointEvent),
    WaitingForInput(InputRequestEvent),
    ContextCompacted(CompactionEvent),
    Retrying(RetryEvent),
    Settled(RuntimeReceipt),
}
```

Agora、TUI、日志和 Verifier 消费标准事件，不消费 Pi 原始 JSON 或 Codex 专用事件。

### 8.9 标准 RuntimeReceipt

```rust
pub struct RuntimeReceipt {
    pub status: CompletionStatus,
    pub final_message: String,

    pub usage: RuntimeUsage,
    pub evidence: Vec<EvidenceRef>,
    pub artifacts: Vec<ArtifactRef>,

    pub workspace_delta: Option<WorkspaceDelta>,
    pub commands: Vec<CommandReceipt>,
    pub tests: Vec<TestReceipt>,
    pub diagnostics: Vec<Diagnostic>,

    pub checkpoint: Option<CheckpointRef>,
    pub verification: VerificationReceipt,
}
```

Executive 不应解析最后一段自然语言来猜任务成功，而应读取 Receipt。

### 8.10 Runtime Broker

用户不需要知道 `pi-rpc` 或 `pi-coder`：

```rust
pub enum RuntimeSelector {
    Auto,
    Named(RuntimeAlias),
    RequiredCapabilities(BTreeSet<RuntimeCapability>),
}
```

例如：

```rust
RuntimeSelectionRequest {
    selector: RuntimeSelector::Named("pi".into()),
    work_order,
}
```

Broker 负责：

- alias 解析。
- capability 匹配。
- health 检查。
- workspace mode 匹配。
- 用户/组织 policy。
- 成本和模型 policy。
- fallback 顺序。
- admission 和 concurrency。

身份应分离：

```text
用户别名：pi
稳定 Runtime ID：pi/coding
实现版本：pi-rpc@具体版本
运行实例：RuntimeInstanceId(UUID)
```

---

## 9. Pi 集成现状与推荐方案

### 9.1 Pi 默认关闭

默认和 production example 都设置：

```toml
[pi_runtime]
enabled = false
```

启用要求 executable、version、SHA-256、固定 argv、namespace sandbox、worktree 和路径 policy。这种 fail-closed 方向正确，但缺完整部署模板和健康检查。

### 9.2 `pi-coder`：安全与验证较强，交互不足

优点：

- 独立 worktree。
- namespace isolation。
- executable/version/hash/argv 固定。
- 网络关闭。
- 收集 workspace 变化。
- 接入 Goal verification。

不足：

- 主要服务 Goal AttemptCoordinator。
- 输入绑定 `PiAttemptRequest/CodingJobSpec`。
- 不适合普通对话式 coding turn。
- steering 和 follow-up 不自然。

### 9.3 `pi-rpc`：交互较强，治理与验证不足

优点：

- resident Pi process。
- JSONL stdin/stdout。
- prompt、steer、follow-up、abort。
- 能投影 Pi tool events。

Pi 官方 RPC 就是为 IDE、UI 和宿主嵌入设计的：[Pi RPC Mode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)。

当前缺口：

- 直接操作当前 workspace，不使用隔离 worktree。
- stderr 丢弃。
- `AgentResult.artifacts` 为空。
- 不产生 authoritative diff。
- 不创建 checkpoint。
- 不执行 Aletheon Coding Verifier。
- 不注入完整 Agent Profile Prompt。
- 不完整映射 allowed tools、tool calls 和 token budgets。

### 9.4 推荐：合并两条路径

目标 `runtime-pi`：

- 使用 Pi RPC resident session。
- 每个 coding job 使用 Aletheon 管理的 worktree/checkpoint。
- 支持 steer/follow-up/abort。
- Runtime Broker 映射模型、thinking、tools、Prompt 和预算。
- 捕获 stderr、tool events、diff 和 artifacts。
- Pi 结束后进入统一 Verifier。
- 验证失败时把结构化错误送回同一 Pi session。
- 最终输出标准 RuntimeReceipt。

### 9.5 为什么不是完全重写 Pi

Pi 已经提供小而一致的工具面、RPC、steering、Session tree、自动压缩、项目上下文和扩展能力。官方当前内置工具包括 `read`、`bash`、`edit`、`write`、`grep`、`find`、`ls`：[Pi README](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)。

Pi 的 Session 使用 JSONL tree，压缩后保留完整历史并累计文件操作：[Pi Compaction](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/compaction.md)。

Aletheon 没必要立即重写所有成熟机制；更重要的是拥有稳定 Runtime API、治理、证据和验证。

---

## 10. Primary Runtime 与 subagent 的正确分工

### 10.1 不应完全依赖弱主 Agent 主动调用 Pi

如果把 Pi 仅作为可选 subagent：

- 当前主 Agent 看不到 Agent control tools。
- 主 Cognit 可能不委派。
- 可能先消耗大量 token 自己尝试。
- delegation prompt 容易丢上下文和验收条件。
- Pi 结束后主 Cognit 可能错误总结结果。

### 10.2 推荐确定性路由

```text
普通对话、身份、记忆
  → Native Cognit

只读代码分析
  → Native Cognit tools 或 Pi readonly profile

代码修改、构建、测试
  → Pi Coding Runtime 作为 primary executor

独立模块探索、review、diagnostics
  → bounded subagents
```

### 10.3 Pi 在 Kernel 中仍然可以是 child Agent

“primary executor”和“child Agent process”不冲突：

- Kernel 视角：Pi 是受 root Agent 管理的 child process。
- 用户体验：当前 coding turn 直接由 Pi 执行。
- Executive 视角：Pi 是由 Broker 选中的 Capability Runtime。
- Cognit 视角：Pi 接收标准 WorkOrder，而不是自由文本猜测任务。

### 10.4 subagent 适用场景

- 并行搜索互不相关模块。
- 独立 reviewer。
- 单独测试/diagnostics worker。
- 多 worktree 无冲突修改。
- 长任务阶段性 delegation。

subagent 不应成为每个简单编码任务的必经层。

---

## 11. 工具治理：Aletheon-owned 与 Runtime-native

### 11.1 Aletheon-owned tools

由 Kernel/Platform/Corpus 执行并统一治理：

- memory
- Agora
- Agent lifecycle
- checkpoint
- approval
- device
- browser
- system services
- credentials/network grants

### 11.2 Runtime-native tools

由 Pi、Codex、Hermes 自己执行：

- read/find/grep
- edit/write
- bash
- build/test

不必强行把 Pi 内部每个工具都重新注册为 Corpus Tool。更合理的流程：

```text
Aletheon 发送 WorkOrder
  ↓
Pi 使用自己的 read/edit/bash
  ↓
Pi Adapter 把调用和结果投影为标准 RuntimeEvent
  ↓
Aletheon 收集 WorkspaceDelta、Evidence 和 RuntimeReceipt
  ↓
Verifier 独立验证
```

### 11.3 ToolGovernance 能力等级

```rust
pub enum ToolGovernance {
    Intercepted, // 每次工具调用都可由 Aletheon 审批/执行
    Mediated,    // 部分敏感调用被宿主代理
    Observed,    // Runtime 内部执行，Aletheon接收事件并依赖 Sandbox
    Opaque,      // 只能看到最终结果，风险最高
}
```

Broker 根据组织 policy 决定某个 Runtime 是否可用于特定 workspace。

---

## 12. 两类 Platform 的边界与独立计划

此前把操作系统兼容与硬件控制统称为 Platform，容易把两种完全不同的生命周期、权限和安全要求耦合在一起。现拆为两条独立计划：

1. **Host Platform**：解决 Aletheon 在 Linux、Windows、macOS 上如何运行，统一进程、文件、网络、PTY、Sandbox、服务、凭据、用户会话和桌面交互。详见 [Host Platform 多操作系统生产化计划](aletheon-host-platform-plan.md)。
2. **Hardware Control Platform**：解决设备、传感器、执行器、串口、CAN、GPIO、ROS 2 和机器人的发现、授权、遥测与安全控制。详见 [Hardware Control Platform 生产化计划](aletheon-hardware-control-platform-plan.md)。

边界：

```text
Host Platform
  ├── Linux / Windows / macOS
  ├── process / fs / network / PTY / service / sandbox
  └── keyboard / mouse / window / clipboard / host media

Hardware Control Platform
  ├── Device Registry / Provider / Broker / Control Lease
  ├── sensor / actuator / serial / CAN / GPIO / ROS 2
  └── Robot Edge Runtime / Safety Supervisor / telemetry
```

两者只通过 Kernel Capability、Device Object 和受治理的 Host 原语连接：Hardware Provider 可以使用 Host Platform 打开串口或 socket，但 Executive/Cognit 不应看到 OS 句柄或总线细节。

当前代码也验证了拆分的必要性：Corpus 中的 `PlatformAdapter` 实际只覆盖服务、主机信息和提权，且主要为 Linux/Android；Fabric `BodyRuntime` 的字符串 Action 又不足以表达硬件身份、租约、时钟、deadline、遥测和安全状态。

实施顺序可以并行但门槛不同：Host 先完成 Linux Coding Agent 生产闭环，再扩 Windows/macOS；Hardware 先完成类型系统与模拟器，再进入 ROS 2 仿真、只读总线和真实执行器。硬件控制不需要等待三大桌面 OS 全部完成，但真实机器人生产上线必须依赖稳定的 Linux Host backend。

---

## 13. 分阶段实施路线

### PR 1：修复 Agent 基础闭环

内容：

- 修复 unlimited iteration 语义。
- 建立完整 `ResolvedTurnProfile`。
- 主 Prompt、模型、预算、工具和 verifier 同时生效。
- 注册稳定 Agent control definitions 后再编译 Profile。
- 增加高层 delegate tools。

验收：

- `code-agent` 不再被压成一次迭代。
- 主模型能看到授权后的委派入口。
- 未授权工具不可见且不可执行。
- Profile model/prompt/budget 有 E2E 测试。

### PR 2：Workspace Tools V2

内容：

- 统一 `read/ls/find/grep/edit/write/bash` 语义。
- 修复 cwd、相对路径和 symlink confinement。
- 搜索使用全局 limit、cursor 和结构化结果。
- 编辑支持 expected hash。
- 长输出进入 Artifact Store。
- 旧工具名保留兼容 alias。

验收：

- daemon cwd 与 workspace cwd 不同时仍正确。
- 大仓库搜索严格遵守总上限。
- stale edit 不覆盖用户修改。
- workspace escape 被拒绝。

### PR 3：Runtime API 与 Broker

内容：

- 创建 `runtime-api` 和 `runtime-broker`。
- 引入 Manifest、WorkOrder、LaunchSpec、Event、Receipt。
- 删除 Goal 对 `PiAttemptRequest` 的依赖。
- 删除 AgentControlService 的 Pi 字符串判断。
- Native Cognit 和 Pi 都实现 contract tests。

验收：

- Executive/Goal 不 import Pi 具体类型。
- 可通过 alias 或 capability 选择 Runtime。
- Runtime 不健康时 Broker 能拒绝或 fallback。
- Native/Pi 产生同一标准事件与 Receipt。

### PR 4：Production Pi Adapter

内容：

- 合并 pi-coder 和 pi-rpc 优点。
- resident RPC + isolated worktree。
- steer/follow-up/abort。
- 模型、Prompt、tools、budget 映射。
- stderr、diff、events 和 artifacts。
- production config 和 health probe。

验收：

- Pi 能完成真实文件修改和测试。
- 用户执行中可 steering。
- 失败后同一 Session 继续修复。
- workspace 外写入被拒绝。
- 结果包含 authoritative WorkspaceDelta。

### PR 5：Verifier 与 Receipt

内容：

- CodingCompletionVerifier。
- 结构化 command/test/diagnostic receipts。
- 自动选择最窄相关测试。
- 验证失败发送 RuntimeMessage::VerificationFailure。
- 区分 verified/unverified/blocked/budget exhausted。

验收：

- 没有证据不能返回 Verified Success。
- 测试失败会继续修复而不是结束。
- 用户能看到命令、测试、diff 和最终 verdict。

### PR 6：Trajectory、压缩和恢复

内容：

- 持久保存完整 tool call/result pairs。
- token-based compaction，取消固定 6 条历史。
- 累积文件操作、决定、约束和进度。
- session branching/checkpoint。
- daemon 重启后恢复支持 resumability 的 Runtime。

### PR 7：生产编码评测

至少 30 个任务：

- 搜索并解释代码。
- 单文件 Bug。
- 跨模块修改。
- 添加测试。
- 编译失败恢复。
- 测试失败恢复。
- 大输出处理。
- 用户 steering。
- subagent reviewer。
- daemon 重启恢复。
- dirty worktree。
- `AGENTS.md` 遵循。
- protected path 拒绝。

指标：

- task success rate
- test pass rate
- verifier pass rate
- incorrect completion rate
- regression rate
- retry recovery rate
- context overflow rate
- tool calls/tokens/latency/cost

---

## 14. 测试与 Contract Gate

### 14.1 工具 Contract Tests

每个 workspace tool 必须覆盖：

- daemon cwd 与 workspace cwd 不同。
- 相对路径。
- 绝对路径。
- symlink escape。
- protected paths。
- hidden/ignored files。
- Unicode 文件名和内容。
- 大输出和 cursor。
- cancellation/timeout。
- dirty workspace。
- 多 Agent 并发修改。

### 14.2 Runtime Contract Tests

所有 Runtime adapters 必须通过同一套测试：

- Manifest 完整且稳定。
- health 可观测。
- start 只发一次 Started。
- tool event 有序且可关联。
- steering/follow-up 语义一致。
- cancellation 终止整个进程树。
- budget 被执行。
- terminal event 唯一。
- Receipt 可验证。
- artifacts 可读取。
- workspace policy 不可绕过。

### 14.3 生产 E2E Gate

至少建立一个 canary repository，发布前自动执行：

```text
用户请求
→ Broker 选择 Runtime
→ Runtime 搜索并修改
→ 运行测试
→ Verifier 判定
→ 生成 Receipt
→ daemon 重启后读取完整历史
```

现有 daemon/SQLite/lifecycle 测试不能代替这类任务成功测试。

---

## 15. 优先级与非目标

### P0：立即修复

1. `max_iterations` unlimited 语义。
2. 完整主 Agent Profile。
3. Agent control tools 可达性。
4. `file_search` cwd。
5. 搜索全局 limit。
6. 主生产 Verifier。
7. 编码任务可确定性进入 Pi Runtime。

### P1：生产可用

1. Workspace Tools V2。
2. Runtime API + Broker。
3. Production Pi Adapter。
4. Git、test、diagnostics、artifact tools。
5. 项目指令发现。
6. 完整 trajectory 和 token-based compaction。

### P2：平台扩展

1. Codex/Grok Build/Hermes adapters。
2. Linux Host Platform 生产化，并启动 Windows/macOS Core backend。
3. LSP 和跨语言代码智能。
4. Runtime fallback 与成本路由。
5. 生产编码评测门禁。

### 独立硬件轨道

1. Hardware API、Device Registry、Broker 与 deterministic simulator。
2. 只读设备发现和遥测。
3. ROS 2 仿真、Serial/CAN 测试夹具。
4. 租约、安全状态机和 HIL 通过后再接真实执行器。

### 暂缓

- Android、Remote Host 和多厂商机器人广度扩展。
- 在模拟器、租约与 HIL 门槛完成前开放真实执行器写入。
- 更多未接入任务成功率的意识机制。
- 功能重复的新搜索工具。
- 在通用 Runtime API 完成前继续加入具体 Runtime 特判。
- 立即重写 Pi 已经成熟的 Session、steering 和 compaction。

---

## 16. 关键源码证据索引

| 主题 | 文件 |
| --- | --- |
| 默认 iteration | `config/default.toml` |
| Profile limit 解析 | `crates/executive/src/impl/daemon/bootstrap/runtime.rs` |
| 主 Profile snapshot | `crates/executive/src/service/turn_runtime_ports.rs` |
| 主工具过滤 | `crates/executive/src/impl/daemon/bootstrap/turn_runtime.rs` |
| 主 Turn model policy | `crates/executive/src/service/daemon_turn/execute.rs` |
| 默认 Prompt | `crates/cognit/src/config/mod.rs` |
| code-agent Profile | `agents/code-agent.md` |
| ReActLoop | `crates/cognit/src/harness/linear/step.rs` |
| 工具输出 8 KB | `crates/cognit/src/harness/linear/tool_output.rs` |
| 并行 batch | `crates/cognit/src/harness/linear/batching.rs` |
| 6 条历史 | `crates/executive/src/service/daemon_turn/helpers.rs` |
| ToolRegistry | `crates/corpus/src/tools/tools/registry.rs` |
| file_search | `crates/corpus/src/tools/tools/file_search.rs` |
| grep | `crates/corpus/src/tools/tools/grep.rs` |
| glob | `crates/corpus/src/tools/tools/glob.rs` |
| code_graph | `crates/corpus/src/tools/tools/code_graph.rs` |
| file_read/write | `crates/corpus/src/tools/tools/file_read.rs`, `file_write.rs` |
| apply_patch | `crates/corpus/src/tools/tools/apply_patch.rs` |
| bash_exec | `crates/corpus/src/tools/tools/bash_exec.rs` |
| Sandbox runner | `crates/corpus/src/security/runner.rs` |
| Agent tools | `crates/corpus/src/tools/tools/agent_control.rs`, `agent_tool.rs` |
| Agent contracts | `crates/fabric/src/types/agent_control.rs` |
| Runtime registry | `crates/executive/src/service/agent_control/execution.rs` |
| Pi one-shot | `crates/executive/src/impl/runtime/pi.rs` |
| Pi RPC | `crates/executive/src/impl/runtime/pi_rpc.rs` |
| Pi protocol | `crates/executive/src/impl/runtime/pi_protocol.rs` |
| Pi bootstrap | `crates/executive/src/impl/daemon/bootstrap/request.rs` |
| Pi Goal 特判 | `crates/executive/src/impl/goal/attempt_coordinator.rs` |

---

## 17. 最终建议

Aletheon 的目标不应是复制 Pi、Codex、Grok Build 或 Hermes，而应成为这些能力运行时的 Agent OS：

```text
用户只表达任务和可选 Runtime 偏好
  ↓
Executive/Broker 选择健康、合规的 Runtime
  ↓
Cognit 形成标准 WorkOrder 和验收条件
  ↓
Pi/Codex/Grok/Hermes 执行专业任务
  ↓
Kernel/Host Platform 管理 workspace、权限、进程和 checkpoint
  ↓
Agora/Mnemosyne 保存状态和经验
  ↓
Verifier 根据 Receipt 和独立证据判定成功
```

最近的正确顺序是：

1. 修复 Agent 迭代、Profile、工具可达性和 verifier。
2. 修复并统一 Workspace Tools。
3. 抽象通用 Runtime API 与 Broker，删除 Pi 特判。
4. 把 Pi RPC 做成默认生产 Coding Runtime。
5. 建立真实编码任务评测门禁。
6. 再扩展 Codex、Grok Build、Hermes 与 Windows/macOS Host backend；Hardware 轨道按模拟器到 HIL 的独立门槛推进。

这样既能较快获得真正写代码的能力，也能确保 Aletheon 长期拥有 Runtime 选择权和架构所有权。

---

## 18. 验证限制

本文基于最新版 `dev` 源码静态审计、关键调用链检索和 Pi 官方资料对照。

当前执行环境没有安装 `cargo`，因此 Rust 动态测试未能启动。实施上述 PR 后仍必须在完整 Rust 环境运行：

- Executive/Profile tests
- Cognit ReActLoop tests
- Corpus tool contract tests
- Runtime contract tests
- Pi RPC/worktree tests
- Sandbox/path confinement tests
- 真实代码任务 E2E
