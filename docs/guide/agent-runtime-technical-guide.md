# Aletheon 技术教程：从项目入门到 Runtime 源码

> 一份可以当**项目介绍**、也能当 **Runtime 源码教程**的文档。
> **上篇（第 0 章）**：Aletheon 是什么、16 个 crate、怎么构建运行、能力状态。
> **下篇（第 1–21 章）**：从模型调用到一次完整 Turn 的技术拆解，每章末尾映射到真实源码。
> 面向第一次系统学习 Agent、但具备 Linux/Rust/系统开发经验的开发者；示例以 Rust 风格为主。

---

## 0. 项目介绍：Aletheon 是什么

### 0.1 一句话

> **An Agent that is not merely executed, but continuously exists.**

Aletheon 是**持久自进化的 Agent Runtime**：它是常驻的 daemon / 系统服务，而不是一次性 App。
它不训练 LLM，而是用 LLM 驱动一个闭环——感知 → 认知 → 执行 → 记忆 → 自我进化——并把 Agent
深度接入 Linux 内核与系统服务（systemd、Unix socket、`/proc`、journald、沙箱）。

- 生产平台：Linux（Arch Linux 为主）；Android / Embedded 是设计目标，未实现。
- 入口：`aletheon` 一个二进制，三种模式：`aletheon daemon`（常驻服务）、`aletheon exec`（单次
  CLI 执行）、`aletheon`（TUI）。
- 目标：**Agent Runtime**——承载 Agent 的会话、工具治理、权限、预算、记忆、子 Agent、事件、
  恢复与验收的系统软件；不是再训练一个 LLM。

### 0.2 crate 地图

| Crate | 角色 | 核心内容 |
|---|---|---|
| `contracts` | ABI | 共享类型与 Trait：message/tool/LLM/admission/embodiment；IPC（Unix socket） |
| `dasein` | Self | 自我：identity、boundary、care、narrative、attention |
| `cognit` | Brain | 推理、规划、反思、provider 路由；ReActLoop / RobotHarness |
| `corpus` | Body | 工具注册与执行、沙箱、感知、MCP、硬件驱动 |
| `agora` | Workspace | 共享认知工作区：blackboard、attention、task graph、scratchpad、trace |
| `interact` | Interface | 可复用 CLI 与 TUI |
| `mnemosyne` | Memory | episodic/semantic/procedural/self 记忆、检索、沉淀 |
| `metacog` | Meta | 自我进化脚手架 |
| `gateway` | Channels | 通道无关的 intent/effect 分发与传输 |
| `kernel` | Kernel services | Admission / Permit / Lease、clock/timer、operation 基础 |
| `platform` | Host platform | Linux 宿主能力契约与适配（Android/macOS/Windows 未实现） |
| `runtime` | Agent runtime | 外部 runtime manifest、Turn/Session/Agent 生命周期与确定性选择 |
| `application` | App services | daemon 生命周期、admin service、turn engine 组合 |
| `hardware` | Embodiment | 硬件 permit/receipt、确定性 simulator、gRPC provider |
| `execd` | Isolated executor | 提权 / 隔离的文件系统执行 daemon |
| `aletheon` | Assembly | 统一可执行入口，无领域逻辑 |

Adapter crates（`crates/adapters/`，各自归属其背书的 port）：

| Crate | 角色 | 核心内容 |
|---|---|---|
| `adapters-sqlite` | Persistence | SQLite 会话事件溯源存储与投影 |
| `adapters-agent-profile` | Agent profiles | 文件系统/Markdown 的 Agent profile 持久化 |
| `adapters-agent-backend` | Agent backends | Runtime 执行权威下的外部 agent backend 适配 |
| `adapters-inference` | Inference | HTTP provider、machine-core RPC 传输、背压 |
| `adapters-gbrain` | Memory | GBrain 补充记忆 MCP 适配与受治理召回 |
| `adapters-google` | Sync | Google/Gmail 同步存储、gmail 入站与分类 |

顶层依赖关系：

```text
aletheon  ---> interact, gateway, corpus, runtime, kernel, application
interact  ---> gateway
corpus    ---> platform, kernel, application
agora/cognit/dasein/metacog/mnemosyne ---> kernel, contracts
gateway/hardware/kernel ---> contracts
execd     ---> platform
```

### 0.3 构建与运行

仓库规则：**不要直接用裸 `cargo`**（会耗尽机器内存）。统一走 `just` 或 `scripts/cargo-agent.sh`：

```bash
just build              # test + lint
just check              # fmt + test + lint + doc
just acceptance         # architecture-status.toml 验收（architecture-check）
bash scripts/cargo-agent.sh +stable check --workspace
```

运行：

```bash
# 常驻服务（systemd 安装态）：
aletheon daemon --config /etc/aletheon/config.toml
# 单次执行一个 turn：
aletheon exec ...
# TUI：
aletheon
```

### 0.4 能力状态

以 `architecture-status.toml` 为权威账本（`just acceptance` 校验）。概况：

- **稳定（有代码 + 测试）**：DaemonHost（Unix socket JSON-RPC）、SystemdHost、ReActLoop 推理
  （唯一生产推理引擎）、TUI、多会话、健康检查、Bash/File/Grep 工具、Provider 抽象
  （Anthropic / OpenAI 兼容）、SQLite 会话持久化、Hook 系统、Bubblewrap 沙箱、多 Agent 协作。
- **实验**：ContainerHost、io_uring IPC、自进化 loop 示例（需显式 opt-in）、本地 Ollama
  （出厂配置禁用）。
- **仅设计**：FUSE、D-Bus、Android、嵌入式、向量库、macOS/Windows。
- 硬件侧（`hardware` 确定性 simulator、`RobotHarness`、gRPC provider）当前为
  `experimental_wired`，**不是生产主链**。机器人执行链的实现说明已并入当前代码与本指南，真实
  bridge/stance 能力仍由对应 provider 配置决定。

### 0.5 文档地图与阅读约定

| 文档 | 内容 |
|---|---|
| `docs/guide/concepts.md` | 概念层：三体架构（SelfField / Cognit / Corpus） |
| **本文档** | 项目介绍 + Runtime 源码级教程 |
| `docs/design/architecture-overview.md` | 完整系统架构 |
| `docs/design/README.md` | 按 crate 的设计与状态矩阵 |
| `docs/testing/runtime-correctness.md` | 运行时正确性测试视角 |
| `architecture-status.toml` | 权威验收账本 |

> **读代码约定**：下文每章末尾的「Aletheon 对应代码」路径在编写时已逐一核对存在；代码会演进，
> 路径是起点而非终点。

---

## 1. 先建立正确的技术模型

### 1.1 LLM、Agent 和 Agent Runtime

LLM 是一个输入 token 序列、输出 token 序列的模型。它本身不会读文件、执行命令、保存
记忆，也不会在两次请求之间持续运行。

```text
tokens_in -> neural network -> tokens_out
```

Agent 是一个使用 LLM 选择下一步动作的闭环程序：

```text
goal -> build context -> call model -> parse action
     -> execute action -> collect observation -> build next context
     -> ... -> verify result
```

Agent Runtime 是承载一个或多个 Agent 的系统软件，还要解决：

- 会话和状态；
- 工具注册与执行；
- 权限、沙箱和审批；
- token、时间和费用预算；
- 超时、取消和重试；
- 记忆检索；
- 子 Agent；
- 事件、日志、恢复和验收。

Aletheon 的目标是 Agent Runtime，不是再训练一个 LLM。

### 1.2 一次 Agent 运行的最小数据流

```text
User input
    |
    v
Vec<Message> + Vec<ToolDefinition>
    |
    v
LLM HTTP request
    |
    +--> assistant text ----------> final candidate
    |
    +--> tool call(name, JSON) ---> host tool executor
                                      |
                                      v
                                  tool result
                                      |
                                      v
                              append to messages
                                      |
                                      +--> next LLM request
```

模型永远只提出 `tool call`。Host 决定这个调用是否存在、是否合法、是否被授权以及如何执行。

---

## 2. 模型请求到底是什么

### 2.1 Message 数据结构

最简单的消息类型：

```rust
#[derive(Clone, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    Image { source: ImageSource },
}
```

为什么不直接使用一个 `String`？因为模型响应可能同时包含文本、推理、多个工具调用和图片。
工具结果还必须通过 `tool_use_id` 与原调用配对。

### 2.2 一个真实的工具请求

OpenAI-compatible Chat Completions 请求通常近似：

```json
{
  "model": "example-model",
  "messages": [
    {"role": "system", "content": "You are a coding agent."},
    {"role": "user", "content": "读取 Cargo.toml"}
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "file_read",
        "description": "Read a UTF-8 file",
        "parameters": {
          "type": "object",
          "properties": {
            "path": {"type": "string"}
          },
          "required": ["path"],
          "additionalProperties": false
        }
      }
    }
  ]
}
```

模型可能返回：

```json
{
  "role": "assistant",
  "content": null,
  "tool_calls": [
    {
      "id": "call_01",
      "type": "function",
      "function": {
        "name": "file_read",
        "arguments": "{\"path\":\"Cargo.toml\"}"
      }
    }
  ]
}
```

注意 `arguments` 经常是 JSON 字符串，需要再次解析。解析成功也只完成了语法检查。

Host 执行后追加：

```json
{
  "role": "tool",
  "tool_call_id": "call_01",
  "content": "[workspace]\nmembers = [...]"
}
```

然后把完整消息序列再次发给模型。

### 2.3 为什么不同 Provider 需要 Adapter

不同提供商对以下内容的格式不同：

- system message；
- tool definition；
- tool result role；
- streaming event；
- usage 和 cache token；
- reasoning content；
- stop reason；
-错误状态和 retry hint。

因此核心运行时应定义 provider-neutral trait：

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse>;

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream>;

    fn runtime_facts(&self) -> ModelRuntimeFacts;
}
```

具体 OpenAI/Anthropic/Ollama 格式只存在于 adapter 内。

### 2.4 Aletheon 对应代码

- 公共类型：`crates/contracts/src/types/llm_types.rs`；
- 消息块：`crates/contracts/src/types/message.rs`；
- OpenAI adapter：`crates/cognit/src/adapters/inference/openai_provider.rs`；
- Anthropic adapter：`crates/cognit/src/adapters/inference/anthropic.rs`；
- Ollama adapter：`crates/cognit/src/adapters/inference/ollama.rs`；
- provider factory：`crates/cognit/src/composition/inference_factory.rs`。

阅读这些文件时，重点追踪一条 `ContentBlock::ToolUse` 如何变成每家 API 的 JSON。

---

## 3. Streaming：模型为什么能逐字输出

### 3.1 SSE 数据流

多数云模型使用 HTTP Server-Sent Events。服务端不是一次返回完整 JSON，而是返回多个 event：

```text
data: {"delta":{"content":"你"}}

data: {"delta":{"content":"好"}}

data: {"finish_reason":"stop"}

data: [DONE]
```

工具参数也会分片：

```text
ToolUseStart(id="call_1", name="file_read")
ToolUseDelta(id="call_1", delta="{\"pa")
ToolUseDelta(id="call_1", delta="th\":\"Cargo.toml\"}")
ToolUseComplete(id="call_1", input={"path":"Cargo.toml"})
```

### 3.2 Streaming 状态机

不能对每个网络 chunk 直接 `serde_json::from_slice`，因为：

- 一个 UTF-8 字符可能跨 chunk；
-一个 SSE event 可能跨 chunk；
-一个 chunk 可能包含多个 event；
-工具 JSON 本身分片。

最小状态：

```rust
struct StreamState {
    utf8_buffer: Vec<u8>,
    event_buffer: String,
    tool_calls: HashMap<usize, PartialToolCall>,
    usage: Usage,
    stop_reason: Option<StopReason>,
}
```

解析流程：

```text
network bytes
 -> incremental UTF-8 decoder
 -> split complete SSE frames
 -> parse provider event
 -> update partial tool state
 -> emit provider-neutral StreamChunk
```

### 3.3 流结束不等于正常结束

必须区分：

-收到明确 `Done`；
-连接正常 EOF 但没有 terminal event；
- idle timeout；
-取消；
-半个工具 JSON 后断流；
-已经执行工具但返回流中断。

最后一种涉及副作用恢复，不能简单重新跑整轮。

### 3.4 Aletheon 对应代码

- `crates/cognit/src/adapters/inference/utf8_stream.rs`；
-各 provider 的 `complete_stream`；
- `crates/cognit/src/harness/linear/step.rs`；
- `crates/cognit/src/harness/event_sink.rs`；
- `crates/contracts/src/types/llm_types.rs`（`StreamChunk`）。

当前 non-streaming `run()` 通过 adapter 复用 streaming loop，这是正确方向：一个 loop，两种消费方式。

---

## 4. 实现第一个 ReAct Agent

### 4.1 Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub metadata: ToolResultMeta,
}
```

### 4.2 Tool Registry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = self.tools.values().map(to_definition).collect::<Vec<_>>();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }
}
```

为什么排序？工具 schema 是模型请求前缀的一部分。`HashMap` 随机顺序会降低 prompt cache 命中，
也让请求 digest 不稳定。

### 4.3 最小循环

```rust
pub async fn run_agent(
    provider: &dyn LlmProvider,
    registry: &ToolRegistry,
    mut messages: Vec<Message>,
    max_iterations: usize,
) -> anyhow::Result<String> {
    let definitions = registry.definitions();

    for _ in 0..max_iterations {
        let response = provider.complete(&messages, &definitions).await?;
        messages.push(Message::assistant(response.content.clone()));

        let calls = response.tool_calls();
        if calls.is_empty() {
            return response.final_text()
                .ok_or_else(|| anyhow::anyhow!("model stopped without final text"));
        }

        for call in calls {
            let result = match registry.resolve(&call.name) {
                Some(tool) => tool.execute(call.input, &ToolContext::default()).await,
                None => ToolResult::error(format!("unknown tool: {}", call.name)),
            };
            messages.push(Message::tool_result(call.id, result));
        }
    }

    anyhow::bail!("iteration budget exhausted")
}
```

这段代码只是教学实现，还缺少权限、预算、取消、压缩、幂等、并发和验证。

### 4.4 多工具调用

模型一次可以返回多个 ToolUse。能否并行取决于工具语义：

```rust
enum ConcurrencyClass {
    ReadOnly,
    IndependentSideEffect,
    ExclusiveSideEffect,
}
```

两个文件读取可以并行；两个修改同一工作区的 patch 不能盲目并行；机器人动作通常需要设备
exclusive lease。

### 4.5 工具错误如何返回

工具错误通常应作为 `ToolResult { is_error: true }` 返回模型，让模型有机会修正参数。基础设施
崩溃、权限拒绝和 deadline 到期则可能直接终止 Turn，不能全部包装成普通文本继续尝试。

需要错误分类：

```rust
enum ToolErrorClass {
    InvalidInput,
    PermissionDenied,
    Transient,
    Conflict,
    Timeout,
    Cancelled,
    Unsafe,
    Internal,
}
```

### 4.6 Aletheon 对应代码

- `crates/corpus/src/tools/tools/`；
- `crates/corpus/src/tools/tools/registry.rs`；
- `crates/cognit/src/harness/linear/`；
- `crates/cognit/src/harness/linear/tool_exec.rs`；
- `crates/contracts/src/types/tool.rs`。

---

## 5. 从 Tool 升级到 Capability

直接 `tool.execute()` 只适合教学。生产 Runtime 需要统一治理层。

### 5.1 CapabilityRequest

```rust
pub struct CapabilityRequest {
    pub call: CapabilityCall,
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

pub struct CapabilityAuthority {
    pub principal: PrincipalId,
    pub action: String,
    pub requested_scope: CapabilityScope,
    pub risk: RiskLevel,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
}
```

模型输出只有 `name + input`。Executive 根据当前 principal、workspace、Agent profile 和政策补齐
authority，模型不能自己声称“我有权限”。

### 5.2 Admit -> Execute -> Settle

```rust
let permit = admission.admit(request.to_admission()).await?;

let result = tokio::select! {
    value = executor.execute_with_permit(&request, &permit) => value,
    _ = request.control.cancel.cancelled() => {
        admission.revoke(permit.id, OperationCancelled).await?;
        return Err(Cancelled);
    }
};

admission.settle(permit.id, result.usage.clone()).await?;
```

Permit 是单次、有限 scope、有限时间的执行权。Settlement 使用实际资源量结算预算并释放 lease。

### 5.3 为什么要 Lease

Lease 解决对独占资源的有限期占有：

```text
workspace write lease
GPU lease
robot control lease
camera stream lease
```

如果 Agent 崩溃，lease 到期后资源自动回收。真实机器人还必须在 lease 过期时进入安全状态。

### 5.4 Aletheon 对应代码

- `crates/kernel/src/capability/mod.rs`；
- `crates/kernel/src/admission/`；
- `crates/kernel/src/capability/invoker.rs`；
- `crates/contracts/src/types/admission.rs`；
- `crates/kernel/src/capability/governed.rs`。

---

## 6. Context Window 与 Token Budget

### 6.1 为什么上下文会不断增长

每轮工具循环都会追加：

```text
assistant tool call
+ tool result
+ next assistant response
```

如果第 `i` 轮上下文长度是 `c_i`，总计费输入可能接近：

```text
total_input = c_1 + c_2 + ... + c_n
```

因此一次拥有 100k active context 的 10 轮任务，累计输入可能远高于 100k。

### 6.2 Context budget

```rust
pub struct ContextBudget {
    pub max_context: usize,
    pub output_reserve: usize,
    pub tool_result_reserve: usize,
    pub safety_margin: usize,
}

impl ContextBudget {
    pub fn input_limit(&self) -> usize {
        self.max_context
            - self.output_reserve
            - self.tool_result_reserve
            - self.safety_margin
    }
}
```

预算分配示例：

```text
system/profile       8%
tool definitions    12%
task contract        5%
retrieved memory    15%
working state       15%
recent history      35%
reserve             10%
```

比例不是固定规则，但必须保证安全指令、当前任务和未结算 tool call/result 不被截断。

### 6.3 Compaction

三种常见策略：

1. Sliding window：保留最近 N 条，简单但可能丢失早期约束；
2. Head + tail：保留开头系统/目标和最近交互；
3. Summary + tail：旧历史压缩为摘要，保留近期原文。

更可靠的结构化压缩：

```rust
struct CompactionRecord {
    summary: String,
    unresolved_tasks: Vec<String>,
    verified_facts: Vec<FactRef>,
    failed_attempts: Vec<AttemptRef>,
    artifacts: Vec<ArtifactRef>,
    source_range: EventRange,
    digest: String,
}
```

摘要不能替换原事件权威，只是下一次模型上下文的投影。

### 6.4 Prompt Cache

Prompt cache 依赖相同前缀：

```text
[stable system][stable tools][stable old messages][new dynamic suffix]
```

导致缓存失效的常见原因：

- system prompt 每轮加入时间戳；
-工具顺序来自 `HashMap`；
- JSON 字段顺序不稳定；
-动态记忆插入 system 前部；
-每轮使用不同模型或 provider route。

### 6.5 Aletheon 对应代码

- `crates/cognit/src/harness/linear/message_compose.rs`；
- `crates/cognit/src/harness/linear/` 的 compaction；
- `crates/cognit/src/harness/session.rs`（`ProjectionRecordingLlm`）；
- `crates/contracts/src/types/model_projection.rs`；
- `docs/testing/runtime-correctness.md`。

---

## 7. Model Router、Retry 与 Backpressure

### 7.1 Model routing

不同任务可选择不同模型：

```rust
struct ModelRequirements {
    tool_calls: bool,
    vision: bool,
    min_context_tokens: usize,
    max_latency_ms: Option<u64>,
    cost_tier: CostTier,
}
```

Router 根据任务要求、模型能力、健康状态、预算和当前限流选择 effective route。最终模型事实
必须来自 Host，不允许模型自己报告身份。

### 7.2 Retry 分类

可重试：

- 429 rate limit；
- 502/503；
-连接超时；
- provider 建议 `Retry-After`。

通常不可直接重试：

-无效 API key；
-请求 schema 错误；
-上下文超限；
-模型不支持工具；
-安全拒绝。

指数退避：

```text
delay_n = min(max_delay, base * 2^n) + jitter
```

必须优先服从 provider 的 `Retry-After`。

### 7.3 Backpressure

单 Session 做 retry 不足以保护机器。多个 Session、Goal 和 Sub-agent 可能同时攻击同一 provider。
需要 machine/provider 级：

- semaphore；
- token/request rate bucket；
- cooldown；
- circuit breaker；
-排队 deadline；
-健康快照。

### 7.4 Aletheon 对应代码

- `crates/cognit/src/adapters/inference/scheduler.rs`；
- `crates/cognit/src/adapters/inference/backpressure.rs`；
- `crates/cognit/src/composition/provider_registry.rs`；
- machine core RPC 的 provider permit/health 路径；
- `ModelRuntimeFacts`。

---

## 8. Agent State 与持久化

### 8.1 为什么不能只保存聊天记录

聊天记录能告诉你模型说了什么，却不能可靠表达：

- Tool 是否真正执行；
-副作用是否结算；
-哪个 attempt 失败；
-当前权限和 lease；
-任务是否已验证；
-重启后应该恢复还是重跑。

需要独立的领域状态。

### 8.2 基本对象

```text
Session   用户交互连续性
Turn      一次用户输入到终态
Goal      可验收目标
Task      可调度工作单元
Attempt   Task 的一次实际尝试
Action    一次模型或能力动作
Operation 受治理执行生命周期
```

### 8.3 SQLite schema 示例

```sql
CREATE TABLE attempts (
  attempt_id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at_ms INTEGER NOT NULL,
  settled_at_ms INTEGER,
  result_json TEXT,
  receipt_id TEXT
);

CREATE UNIQUE INDEX attempts_task_generation
ON attempts(task_id, request_digest);
```

### 8.4 状态转换

```text
Pending -> Running -> Verifying -> Settled
                  \-> Failed
                  \-> Cancelling -> Cancelled
```

终态通常不可重新打开。Retry 创建新 Attempt，不覆盖旧失败。

```rust
fn transition(from: Status, to: Status) -> Result<(), TransitionError> {
    match (from, to) {
        (Pending, Running)
        | (Running, Verifying)
        | (Verifying, Settled)
        | (Running, Failed)
        | (Running, Cancelling)
        | (Cancelling, Cancelled) => Ok(()),
        _ => Err(InvalidTransition { from, to }),
    }
}
```

### 8.5 Event sourcing 与 Projection

```text
authoritative event log
  -> session projection
  -> UI projection
  -> metrics projection
  -> recovery projection
```

Projection 可以重建，不能反过来覆盖权威事件。写 event 与更新关键状态应在同一事务或使用
outbox，避免状态已变但 event 丢失。

### 8.6 Aletheon 对应代码

- `crates/aletheon/src/daemon/turn_engine.rs`；
- `crates/aletheon/src/composition/turn_coordinator.rs` 与 `crates/aletheon/src/host/session/test_composition.rs`；
- `crates/runtime/src/event_projection/`；
- `crates/adapters/sqlite/src/runtime_agent/mod.rs`；
- `crates/kernel/src/runtime.rs`。

---

## 9. Idempotency 与副作用恢复

### 9.1 超时后的三种可能

Client 调用外部工具后超时：

```text
1. 请求没有到达 provider
2. provider 正在执行
3. provider 已完成，但响应丢失
```

如果直接 retry，第三种情况会重复副作用。

### 9.2 Idempotency key

```text
idempotency_key = hash(
  principal,
  operation,
  action,
  canonical_input,
  generation
)
```

Provider 处理规则：

```text
new key                        -> execute and persist receipt
same key + same request digest -> return stored receipt
same key + different digest    -> conflict
```

### 9.3 WAL/Outbox

可靠副作用常用顺序：

```text
persist intent
 -> execute external effect
 -> persist external receipt
 -> settle local operation
 -> publish terminal event
```

重启时扫描未结算 intent，通过 provider status/idempotency receipt 恢复，不能盲目重新执行。

### 9.4 Aletheon 对应代码

- Agent run request hash；
- Coding attempt settlement recovery 测试；
- Gateway inbox/outbox correlation；
- Memory/GBrain cursor 和 receipt；
- Hardware command sequence、permit 和 lease。

---

## 10. Memory 技术

### 10.1 Memory 不是聊天记录

需要区分：

| 类型 | 示例 |
|---|---|
| Working | 当前计划、未解决问题 |
| Episodic | 2026-08-03 某次机器人仿真失败 |
| Semantic | v62 某配置下关节误差规律 |
| Procedural | 部署和排障步骤 |
| Identity | Agent 的目标、边界和连续性 |

### 10.2 MemoryRecord

```rust
struct MemoryRecord {
    id: MemoryRecordId,
    kind: MemoryKind,
    scope: MemoryScope,
    content: String,
    source_event_ids: Vec<EventId>,
    authority: MemoryAuthority,
    status: MemoryStatus,
    created_at: DateTime,
    valid_from: Option<DateTime>,
    valid_until: Option<DateTime>,
    embedding: Option<Vec<f32>>,
}
```

### 10.3 Embedding 检索

Embedding 把文本映射到向量。常用 cosine similarity：

```text
cos(q, d) = (q · d) / (||q|| ||d||)
```

向量相似表示语义接近，不表示事实正确或仍然有效。

### 10.4 BM25

关键词检索常用 BM25，简化形式：

```text
score(D,Q) = sum IDF(q_i) *
             f(q_i,D)*(k1+1) /
             (f(q_i,D)+k1*(1-b+b*|D|/avgdl))
```

它擅长精确术语、错误码、路径和型号，向量检索擅长语义近似。工程系统通常做 hybrid search。

### 10.5 Hybrid ranking

```text
final = w_vec * vector_score
      + w_bm25 * lexical_score
      + w_recency * recency_score
      + w_authority * authority_score
      + w_evidence * evidence_score
      - w_stale * staleness_penalty
```

然后应用硬过滤：principal/session/agent scope、tombstone、validity、max bytes。

### 10.6 Memory 写入管线

```text
event
 -> candidate extraction
 -> scope classification
 -> provenance validation
 -> novelty/dedup
 -> candidate store
 -> consolidation
 -> promotion/supersession
```

不能把模型每句话直接保存为事实。

### 10.7 Aletheon 对应代码

- `crates/mnemosyne/src/service.rs`；
- `crates/mnemosyne/src/recall/`；
- `crates/mnemosyne/src/consolidation/`；
- `crates/mnemosyne/src/knowledge_graph/`；
- Executive 的 GBrain adapter/worker；
- `docs/design/mnemosyne/memory-system.md`。

---

## 11. Agora：共享工作内存

Memory 解决跨任务经验；Agora 解决当前任务的协作状态。

### 11.1 Workspace 内容

```text
Blackboard   当前共享事实/候选
Attention    当前关注对象
TaskGraph    任务和依赖
Trace        认知轨迹
Scratchpad   临时推导
Artifacts    已验证认知产物
Claims       哪个 process 正在写哪个对象
```

### 11.2 乐观并发控制

```rust
struct Proposal {
    base_version: u64,
    operation: AgoraOperation,
}

fn commit(workspace: &mut Workspace, proposal: Proposal) -> Result<()> {
    ensure!(proposal.base_version == workspace.version, VersionConflict);
    validate(&proposal.operation)?;
    apply(&mut workspace.state, proposal.operation)?;
    workspace.version += 1;
    Ok(())
}
```

两个 Agent 基于 version 10 同时修改：第一个提交为 11，第二个必须冲突并重新读取，不能覆盖。

### 11.3 Aletheon 对应代码

- `crates/agora/src/workspace/mod.rs`；
- `crates/agora/src/task_graph/`；
- `crates/agora/src/blackboard/`；
- `crates/agora/src/attention/`；
- `crates/agora/src/trace/`。

---

## 12. Planning 技术

### 12.1 ReAct 与 Plan-and-Execute

ReAct：

```text
observe -> decide one action -> execute -> observe
```

适合未知环境，但容易短视。

Plan-and-Execute：

```text
goal -> generate task graph -> execute ready tasks -> verify -> repair plan
```

适合步骤明确、成本高的任务。

### 12.2 结构化 Plan

```rust
struct Plan {
    id: PlanId,
    goal: GoalId,
    version: u64,
    tasks: Vec<PlanTask>,
}

struct PlanTask {
    id: TaskId,
    objective: String,
    dependencies: Vec<TaskId>,
    allowed_capabilities: Vec<CapabilityId>,
    preconditions: Vec<Predicate>,
    expected_outcome: ExpectedOutcome,
    budget: TaskBudget,
}
```

### 12.3 Ready queue

```rust
fn ready_tasks(plan: &Plan, states: &HashMap<TaskId, Status>) -> Vec<TaskId> {
    plan.tasks.iter()
        .filter(|t| states[&t.id] == Status::Pending)
        .filter(|t| t.dependencies.iter().all(|d| states[d] == Status::Done))
        .map(|t| t.id)
        .collect()
}
```

### 12.4 Retry 与 Replan

Retry：目标、策略和 expected outcome 不变，只创建新 Attempt。

Replan：现有策略无效，生成新 plan version，保留旧版本和失败证据。

### 12.5 搜索式规划

Tree of Thoughts、MCTS 或 beam search 都需要：

- state representation；
- action generator；
- transition/rollout；
- evaluator；
- branching/compute budget。

如果只有“让模型给 5 个方案再选一个”，却没有稳定 state 和 evaluator，它只是多次采样，不是
可靠规划器。

### 12.6 Aletheon 对应代码

- Cognit planner/reasoning；
- Agora TaskGraph；
- Executive Goal/Attempt；
- RobotHarness 的 Plan/Retry/Replan；
- `docs/design/executive/orchestration.md`。

---

## 13. Verification 技术

模型的 final answer、provider 的 `end_turn`、ToolResult 的 `success` 都不等于 Goal 成功。

### 13.1 ExpectedOutcome

```rust
enum Predicate {
    Equals { path: String, value: Value },
    Range { path: String, min: Option<f64>, max: Option<f64> },
    Change { path: String, min_delta: Option<f64>, max_delta: Option<f64> },
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
}

struct ExpectedOutcome {
    predicate: Predicate,
    freshness_ms: u64,
    stable_window_ms: u64,
    timeout_ms: u64,
}
```

### 13.2 验证循环

```rust
let deadline = now() + expected.timeout;
let mut matched_since = None;

while now() < deadline {
    let observation = world.observe_after(last_sequence).await?;
    ensure_fresh(&observation, expected.freshness_ms)?;

    if evaluate(&expected.predicate, &observation.payload) {
        matched_since.get_or_insert(now());
        if now() - matched_since.unwrap() >= expected.stable_window {
            return VerificationDecision::Matched;
        }
    } else {
        matched_since = None;
    }
}
```

### 13.3 Verification 分类

```text
Matched
RetryableMismatch
ReplannableMismatch
Unsafe
Unknown
```

Unknown 不能当成功。机器人 observation 丢失、代码测试没有运行、provider 状态不明都属于
Unknown 或 failure。

### 13.4 Aletheon 对应代码

- `crates/contracts/src/types/expected_outcome.rs`；
- `crates/contracts/src/types/outcome_verification.rs`；
- `crates/cognit/src/ports/verifier.rs`；
- Executive evaluation store；
- Cognit RobotHarness verifier port。

---

## 14. Sub-agent 技术

### 14.1 Spawn 不是函数调用

Sub-agent 更像进程：

```text
Parent
 -> create task contract
 -> allocate budget/workspace/capabilities
 -> spawn runtime
 -> receive progress/events
 -> wait terminal report
 -> verify artifacts
 -> settle child resources
```

### 14.2 Task contract

```rust
struct SubagentTask {
    task_id: TaskId,
    parent_goal: GoalId,
    objective: String,
    inputs: Vec<ArtifactRef>,
    workspace_scope: WorkspaceScope,
    allowed_capabilities: Vec<CapabilityId>,
    expected_artifacts: Vec<ArtifactSpec>,
    acceptance: Vec<AcceptanceCriterion>,
    budget: Budget,
    deadline: Deadline,
}
```

### 14.3 Mailbox

```rust
struct Envelope<T> {
    message_id: MessageId,
    correlation_id: CorrelationId,
    sender: ProcessId,
    recipient: ProcessId,
    sequence: u64,
    deadline: Option<Deadline>,
    payload: T,
}
```

需要处理重复投递、乱序、过期、ack 和 mailbox 容量。

### 14.4 Workspace 隔离

代码 Agent 最安全的是独立 Git worktree：

```text
parent repository
  -> child worktree A
  -> child worktree B
  -> each produces patch/diff
  -> parent verifies and applies selected result
```

这比两个 Agent 直接修改同一目录可靠。

### 14.5 Parent verification

Child 报告：

```json
{"status":"success","tests":"passed"}
```

不能直接信任。Parent 必须检查 diff、artifact digest 和测试 receipt。子 Agent 只提供候选结果。

### 14.6 Aletheon 对应代码

- AgentControl；
- mailbox/recovery repository；
- Runtime manifest/selector；
- Pi RPC runtime；
- Corpus subagent worktree；
- Agent settlement receipt。

---

## 15. Agent 安全技术

### 15.1 Prompt injection

工具读取的网页、README、issue 或日志都可能包含“忽略之前指令”。这些内容是 data，不是
instruction。

技术防线：

```text
context classification
+ minimum tool exposure
+ capability admission
+ sandbox
+ approval
+ output validation
```

### 15.2 Confused deputy

一个低权限来源让高权限 Agent 替它执行动作。解决方法是 CapabilityRequest 保留真实 principal、
connection、workspace 和 scope，不能只写 `agent=admin`。

### 15.3 Sandbox

Linux 上常见组合：

- namespaces/bubblewrap；
- seccomp；
- Landlock；
- cgroups；
-只读 bind mount；
-网络策略；
-临时工作区。

Sandbox 基础设施不可用时，高风险动作应 fail closed，不能静默改为普通 process 执行。

### 15.4 Output limit

工具返回几百 MB 日志会导致内存和 context 爆炸。ToolResult 应：

-限制字节数；
-保存完整 artifact；
-向模型返回 head/tail/summary；
-标记 truncated；
-提供 digest 和引用。

### 15.5 Loop detector

可以按动作 fingerprint 检测重复：

```text
fingerprint = hash(tool_name + canonical_arguments + relevant_state_version)
```

连续多次相同 fingerprint 且世界状态没有变化时，阻止或要求 replan。

---

## 16. Evaluation 技术

### 16.1 不同层次

```text
Unit test        schema、状态转换、解析器
Contract test    provider/bridge 协议
Trajectory eval  Agent 是否正确使用工具
Outcome eval     外部目标是否真实达成
System eval      重启、并发、限流、持久化
Production eval  安装态真实依赖和 provenance
```

### 16.2 指标

必须分开记录：

- task success rate；
- verification pass rate；
- inference rounds；
- provider retries；
- tool calls/errors；
- input/output/cache tokens；
- active context；
- wall time；
- cost；
- unsafe action blocks；
-恢复成功率。

### 16.3 LLM-as-judge

适合评估解释质量、相关性、表达；不适合独自验证文件、测试、权限、数值和机器人状态。

### 16.4 Aletheon 对应文档

- `docs/testing/runtime-correctness.md`；
- `docs/testing/production-scenarios.md`；
- `docs/design/testing/test-strategy.md`。

---

## 17. Aletheon 的一次 Turn 如何运行

```text
1. Interact/Gateway 接收输入
2. Executive 创建 TurnEngineRequest/Context
3. TurnEngine 进入唯一 TurnService/CognitiveSession 路径
4. Cognit 获取 seed messages、model、tool definitions
5. ReActLoop 调用 LlmProvider
6. 模型产生 ToolUse
7. Cognit 通过 TurnServices::invoke 请求 Capability
8. Kernel Admission 发放 Permit/Lease
9. Corpus/Hardware/Runtime 执行
10. CapabilityResult 和 receipt 持久化
11. ToolResult 回到 ReActLoop
12. 模型继续或返回 final candidate
13. Executive 验证并 settlement
14. Event/session projection 更新
```

建议按此顺序阅读：

```text
crates/aletheon/src/daemon/turn_engine.rs
crates/aletheon/tests/support/turn_service.rs
crates/cognit/src/harness/session.rs
crates/cognit/src/harness/linear/step.rs
crates/cognit/src/harness/linear/tool_exec.rs
crates/kernel/src/capability/mod.rs
crates/runtime/src/event_projection/
```

阅读时自己画出对象和 ID：TurnId、OperationId、ProcessId、Tool call ID、PermitId、AttemptId。

---

## 18. 机器人与 VLA 的技术接入

### 18.1 为什么 Agent 不进入实时环

LLM 延迟通常是数百毫秒到数秒，网络还有抖动；WBC/电机控制需要稳定周期：

```text
Aletheon task loop        seconds
VLA/policy                1–20 Hz（取决于模型和系统）
MPC/WBC                   100–2000 Hz
motor drive loop          device/control period
```

云模型无法提供 hard real-time、确定性 deadline 和 fail-safe。因此 Agent 只产生语义任务或受约束
proposal，Robot Runtime 保持实时控制和本地安全。

### 18.2 Embodiment contract

```rust
#[async_trait]
trait EmbodimentProvider {
    async fn observe(&self, device: &DeviceId) -> Result<Vec<Observation>>;
    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>>;
    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn ProgressSink>,
    ) -> Result<SkillResult>;
    async fn cancel(&self, device: &DeviceId, operation: &OperationId) -> Result<CancelAck>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt>;
}
```

### 18.3 VLA 只产生 proposal

```rust
trait PolicyProviderPort {
    async fn propose(
        &self,
        goal: &str,
        device: &DeviceId,
        state: &[WorldSnapshot],
        images: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, String>;
}
```

VLA 没有 `execute` 方法。Proposal 还要经过：

```text
registered skill check
 -> device check
 -> JSON schema
 -> observation freshness
 -> confidence/provenance
 -> risk/limits
 -> Kernel admission
 -> Robot Runtime safety
```

### 18.4 ROS/MuJoCo Bridge

```text
Aletheon gRPC EmbodimentProvider
 -> Kuavo bridge
 -> ROS topic/service/action
 -> MuJoCo/controller
 -> normalized observations/progress/result
```

ROS 消息和公司专有类型留在 bridge，Fabric 只保留稳定 skill/observation contract。

### 18.5 Aletheon 对应代码

- `crates/contracts/src/types/embodiment.rs`；
- `crates/hardware/`；
- `crates/corpus/src/tools/tools/robot.rs`；
- `crates/aletheon/src/host/embodiment/service.rs`；
- `crates/cognit/src/harness/robot/`；
- `crates/cognit/src/ports/policy_provider.rs`；
- `crates/cognit/src/harness/robot/` 与 `crates/aletheon/src/composition/robot_harness.rs`。

---

## 19. 推荐的实际学习顺序

不要先通读所有 RFC。按技术链学习：

### 第一周：模型与工具循环

1. 看 `Message`、`ContentBlock`、`LlmProvider`；
2. 手工写出一组 request/response JSON；
3. 追踪 OpenAI adapter；
4. 追踪一次 ToolUse/ToolResult；
5. 理解 streaming tool JSON 如何拼接。

### 第二周：Context 与状态

1. 追踪 seed message；
2. 找出 system、memory、goal、Dasein、tools 的注入位置；
3. 观察 context projection receipt；
4. 理解 compaction；
5. 区分 active context 和 cumulative tokens。

### 第三周：执行治理

1. 从 ToolUse 追到 CapabilityRequest；
2. 看 AdmissionRequest 和 ExecutionPermit；
3. 看 cancel/revoke/settle；
4. 阅读 settlement failure recovery 测试；
5. 理解 lease 和预算。

### 第四周：Memory 与规划

1. 构造 RecallRequest；
2. 跟踪 scope/temporal state/ranking；
3. 阅读 Agora TaskGraph；
4. 区分 retry 和 replan；
5. 追踪一个 Sub-agent task/report。

### 第五周：机器人链路

1. 用 simulator 调 `robot_list_skills`；
2. 追踪 `robot_execute_skill`；
3. 检查 permit/lease；
4. 对比 SkillResult 与 VerificationReport；
5. 再开始 Kuavo MuJoCo bridge。

---

## 20. 学完后的技术判断标准

你应当能够从代码回答：

1. 一次模型 HTTP 请求的 messages/tools JSON 如何产生？
2. Streaming 下半个 UTF-8 字符和半个 tool JSON 如何处理？
3. ToolUse ID、OperationId 和 PermitId 为什么不能合并？
4. ReAct loop 在哪里终止，谁判断 Goal 成功？
5. Context 超限时哪些数据绝不能丢？
6. Prompt cache 为什么会被工具顺序破坏？
7. 429 retry 和副作用 retry 有什么本质区别？
8. 为什么 settlement failure 不能重新执行工具？
9. Embedding 相似度为什么不能证明事实正确？
10. Agora 的 version conflict 如何阻止多个 Agent 覆盖？
11. Sub-agent 为什么需要 mailbox、worktree 和 parent verification？
12. VLA proposal 如何经过验证、权限和 Robot Runtime？

能够回答并沿源码追踪这些问题，就真正掌握了开发 Agent Runtime 所需的基础技术，而不是只
记住 ReAct、Memory、Multi-agent 等名词。

---

## 21. 延伸阅读入口

- [`Aletheon 架构总览`](../design/architecture-overview.md)
- [`Core Concepts`](../guide/concepts.md) — 概念层：三体架构（项目介绍上篇）
- [`Cognit 设计`](../design/cognit/README.md)
- [`ReAct Loop`](../design/executive/react-loop.md)
- [`Corpus Tools`](../design/corpus/tools.md)
- [`Agora`](../design/agora/README.md)
- [`Mnemosyne`](../design/mnemosyne/memory-system.md)
- [`Runtime Correctness`](../testing/runtime-correctness.md)
