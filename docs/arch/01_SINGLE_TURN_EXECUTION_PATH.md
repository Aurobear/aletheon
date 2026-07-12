# Phase 0：唯一 Turn Execution Path

## 1. 问题

当前生产或半生产路径包含多套循环：

```text
daemon handler → TurnService/DaemonTurnOrchestrator
AletheonExecutive → TurnService (formerly ReActLoop path)
bin exec → manual LLM/tool loop
Controller → (deleted; was parked ReActLoop)
```

结果是安全、Memory、Agora、Hook 和事件语义不一致。

## 2. 本阶段完成后的调用链

```text
Chat/Exec/Automation Adapter
→ TurnService::submit
→ PreTurnPipeline
→ CognitiveSession::run_turn
→ CapabilityInvoker
→ PostTurnPipeline
→ TurnResult
```

## 3. 新增接口

位置：`crates/fabric/src/include/turn.rs`。

```rust
pub struct TurnRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub session_id: String,
    pub input: String,
    pub working_dir: PathBuf,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadline>,
}

pub struct TurnResult {
    pub output: String,
    pub stop: TurnStop,
    pub metrics: TurnMetrics,
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn emit(&self, event: TurnEvent);
}

#[async_trait]
pub trait TurnServices: Send + Sync {
    async fn recall(&self, req: RecallRequest) -> Result<RecallSet>;
    async fn dasein_view(&self, process: ProcessId) -> Result<DaseinView>;
    async fn agora_view(&self, space: SpaceId) -> Result<AgoraView>;
    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult;
}
```

位置：`crates/cognit/src/harness/session.rs`。

```rust
#[async_trait]
pub trait CognitiveSession: Send {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult>;
}

pub trait HarnessFactory: Send + Sync {
    fn create(&self, profile: &CognitProfile) -> Box<dyn CognitiveSession>;
}
```

第一阶段 `LinearCognitiveSession` 内部继续复用 `ReActLoop`（即现在 `TurnService` 下的认知管线），不要重写 ReAct 算法。

## 4. Executive 新结构

新增：

```text
crates/executive/src/service/mod.rs
crates/executive/src/service/turn_service.rs
crates/executive/src/service/pre_turn.rs
crates/executive/src/service/post_turn.rs
crates/executive/src/service/turn_services.rs
```

`TurnService` 负责组合，不实现认知算法：

```rust
pub struct TurnService {
    factory: Arc<dyn HarnessFactory>,
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
}
```

## 5. 从 handle_chat 迁出的内容

迁入 `PreTurnPipeline`：

- keyword skill；
- fact recall；
- CoreMemory view；
- Dasein view；
- PreTurn hooks；
- bounded history；
- model policy 选择。

迁入 `PostTurnPipeline`：

- PostTurn hooks；
- session append；
- auto memory；
- reflection；
- evolution trigger；
- Agora proposal/commit；
- usage settlement。

Handler 最终只保留：

```rust
let req = parse_turn_request(request)?;
let mut events = SocketTurnEventSink::new(self.notify_tx.clone());
let result = self.turn_service.submit(req, &mut events).await;
format_json_rpc(result)
```

## 6. 迁移步骤

### PR-0A：只加类型和 Adapter

- 增加 `TurnRequest/TurnResult/TurnServices`；
- `LinearCognitiveSession` 包装现有认知管线（原 `ReActLoop`）；
- 旧调用方不变。

### PR-0B：建立 TurnService

- 将现有 Handler 逻辑复制并按 Pre/Cognit/Post 组织；
- 使用 characterization tests 确保行为未变。

### PR-0C：切换 daemon

- `handle_chat` 调用 `TurnService`；
- 删除 Handler 内创建认知管线（原 `ReActLoop`）的代码。

### PR-0D：切换 exec

- 删除 `bin/main.rs::run_exec` 手写循环；
- exec 使用同一 `TurnService`，仅 EventSink 和 ApprovalSink 不同。

### PR-0E：清理重复入口

- `AletheonExecutive::process/process_react` 标记 deprecated 后删除；
- `Controller` 已删除（原计划是成为 `TurnService` 门面或删除）；
- 保留唯一生产 Harness 创建点。

## 7. 测试

新增：

```text
crates/executive/tests/turn_service_equivalence.rs
crates/executive/tests/turn_pipeline_order.rs
crates/cognit/tests/cognitive_session.rs
```

必须验证：

1. daemon 与 exec 对同一个 ScriptedLlm 得到相同 Tool 顺序和最终输出；
2. PreTurn Block 时模型调用次数为 0；
3. PostTurn 只在成功完成后写入 assistant memory；
4. cancel 会终止模型流与工具执行；
5. ToolUse/ToolResult 配对结构保持 Anthropic API 合法；
6. 同一个用户输入不会在 history 中重复两次。

执行：

```bash
cargo test -p cognit cognitive_session
cargo test -p executive turn_service
cargo test -p executive turn_pipeline
cargo check --workspace --all-targets
```

## 8. 完成标准

- `rg 'ReActLoop::new|build_harness' crates/executive crates/bin` 只剩 composition/factory 允许的位置（M0 验收后 `ReActLoop::new` 已从 production path 移除，harness factory 为唯一创建点）；
- `handle_chat` 不直接引用认知管线（原 `ReActLoop`）、`AdvancedCompressor` 或 Provider；
- `bin` 不直接执行 Tool；
- daemon 与 exec 共用安全、Memory、Hook、Agora 和 metrics 语义。

