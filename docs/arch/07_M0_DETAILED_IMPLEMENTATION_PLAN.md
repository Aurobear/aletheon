# M0 — One Kernel, One Turn Path Detailed Implementation Plan

> **For agentic workers:** implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 daemon chat、`exec` 和后续 automation 收敛到同一个 `TurnService`，并先修正 `SandboxFirst` 的 fail-closed 行为。

**Architecture:** 第一阶段不重写 ReAct 算法；先在 `fabric` 增加 Turn/Operation 合约，在 `cognit` 用 adapter 包装现有 `ReActLoop`，再在 `executive` 提取 PreTurn/Cognit/PostTurn 编排。旧入口只保留请求解析、EventSink 和响应格式化。

**Tech Stack:** Rust、Tokio、async-trait、serde、uuid、现有 `cognit::harness::linear::ReActLoop`、现有 daemon JSON-RPC。

---

## 0. 当前代码锚点

- daemon 主入口在 `crates/executive/src/impl/daemon/handler/chat.rs`，`handle_chat` 从第 416 行开始，并在第 741 行创建 harness。
- exec 模式入口在 `crates/bin/src/main.rs`，`run_exec` 从第 273 行开始。
- `AletheonExecutive` 在 `crates/executive/src/core/orchestrator.rs` 第 32 行定义，并在第 70 行创建 harness。
- `Controller` 在 `crates/executive/src/core/controller.rs` 第 55 行定义，并在第 94 行创建 harness。
- `build_harness` 现在返回具体 `ReActLoop`，位置是 `crates/cognit/src/harness/mod.rs` 第 45 行。
- `ReActLoop::run` 是泛型方法，位置是 `crates/cognit/src/harness/linear/step.rs` 第 16 行。
- `SandboxFirst` 目前在 daemon chat 中作为用户消息 note 注入，位置是 `crates/executive/src/impl/daemon/handler/chat.rs` 第 489 行。
- `AletheonExecutive` 的 SandboxFirst 路径目前会继续执行，位置是 `crates/executive/src/core/orchestrator.rs` 第 356 行。

## 1. 文件结构目标

```text
crates/fabric/src/types/operation.rs          # OperationId、ProcessId、ExitReason 最小 newtype
crates/fabric/src/types/turn.rs               # TurnRequest、TurnResult、TurnMetrics、TurnStop
crates/fabric/src/include/turn.rs             # TurnServices、TurnEventSink contracts
crates/cognit/src/harness/session.rs          # CognitiveSession trait + LinearCognitiveSession
crates/executive/src/service/mod.rs           # TurnService module exports
crates/executive/src/service/turn_service.rs  # submit() 编排主链
crates/executive/src/service/pre_turn.rs      # 从 handler 迁出的输入准备
crates/executive/src/service/post_turn.rs     # 从 handler 迁出的落盘/事件/Agora 后处理
crates/executive/src/service/turn_services.rs # ExecutiveTurnServices adapter
crates/executive/tests/turn_service_equivalence.rs
crates/executive/tests/sandbox_first_fail_closed.rs
```

## 2. PR-0A：Turn contracts + Linear adapter

**Files:**

- Create: `crates/fabric/src/types/operation.rs`
- Create: `crates/fabric/src/types/turn.rs`
- Create: `crates/fabric/src/include/turn.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/include/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Create: `crates/cognit/src/harness/session.rs`
- Modify: `crates/cognit/src/harness/mod.rs`
- Test: `crates/cognit/tests/cognitive_session.rs`

### Task 0A.1：增加 Operation/Turn 基础类型

- [ ] **Step 1: 写失败测试**

Create `crates/cognit/tests/cognitive_session.rs`:

```rust
use cognit::harness::{CognitiveSession, HarnessConfig, LinearCognitiveSession};
use fabric::{NoopTurnEventSink, OperationId, ProcessId, StubTurnServices, TurnRequest, TurnStop};
use std::path::PathBuf;

#[tokio::test]
async fn linear_session_returns_turn_result() {
    let mut session = LinearCognitiveSession::new(HarnessConfig::default());
    let req = TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        session_id: "test".into(),
        input: "hello".into(),
        working_dir: PathBuf::from("."),
        model_policy: None,
        deadline: None,
    };

    let result = session
        .run_turn(req, &StubTurnServices::default(), &NoopTurnEventSink)
        .await
        .expect("turn should run");

    assert_eq!(result.stop, TurnStop::Completed);
}
```

Run:

```bash
cargo test -p cognit --test cognitive_session linear_session_returns_turn_result -q
```

Expected: FAIL because `CognitiveSession` and `TurnRequest` do not exist.

- [ ] **Step 2: 新增 `crates/fabric/src/types/operation.rs`**

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub Uuid);

impl OperationId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for OperationId {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessId(pub Uuid);

impl ProcessId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for ProcessId {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonoDeadlineMillis(pub u64);
```

- [ ] **Step 3: 新增 `crates/fabric/src/types/turn.rs`**

```rust
use crate::types::operation::{MonoDeadlineMillis, OperationId, ProcessId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub session_id: String,
    pub input: String,
    pub working_dir: PathBuf,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStop {
    Completed,
    Blocked,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    pub output: String,
    pub stop: TurnStop,
    pub metrics: TurnMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TurnEvent {
    Started { operation_id: OperationId },
    Finished { operation_id: OperationId, stop: TurnStop },
    ToolCall { operation_id: OperationId, name: String },
}
```

- [ ] **Step 4: 新增 `crates/fabric/src/include/turn.rs`**

```rust
use crate::types::operation::ProcessId;
use crate::types::turn::TurnEvent;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallRequest {
    pub session_id: String,
    pub input: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallSet {
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaseinView {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgoraView {
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub process_id: ProcessId,
    pub name: String,
    pub input: serde_json::Value,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn emit(&self, event: TurnEvent);
}

#[async_trait]
pub trait TurnServices: Send + Sync {
    async fn recall(&self, req: RecallRequest) -> Result<RecallSet>;
    async fn dasein_view(&self, process: ProcessId) -> Result<DaseinView>;
    async fn agora_view(&self, session_id: &str) -> Result<AgoraView>;
    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult;
}

pub struct NoopTurnEventSink;

#[async_trait]
impl TurnEventSink for NoopTurnEventSink {
    async fn emit(&self, _event: TurnEvent) {}
}

#[derive(Default)]
pub struct StubTurnServices;

#[async_trait]
impl TurnServices for StubTurnServices {
    async fn recall(&self, _req: RecallRequest) -> Result<RecallSet> { Ok(RecallSet::default()) }
    async fn dasein_view(&self, _process: ProcessId) -> Result<DaseinView> { Ok(DaseinView::default()) }
    async fn agora_view(&self, _session_id: &str) -> Result<AgoraView> { Ok(AgoraView::default()) }
    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult {
        CapabilityResult { call_id: req.call_id, output: format!("tool {} is unavailable in stub", req.name), is_error: true }
    }
}
```

- [ ] **Step 5: 更新 module exports**

Append to `crates/fabric/src/types/mod.rs`:

```rust
pub mod operation;
pub mod turn;
```

Append to `crates/fabric/src/include/mod.rs`:

```rust
pub mod turn;
```

Append to `crates/fabric/src/lib.rs` near other type re-exports:

```rust
pub use types::operation::{MonoDeadlineMillis, OperationId, ProcessId};
pub use types::turn::{TurnEvent, TurnMetrics, TurnRequest, TurnResult, TurnStop};
pub use include::turn::{
    AgoraView, CapabilityRequest, CapabilityResult, DaseinView, NoopTurnEventSink,
    RecallRequest, RecallSet, StubTurnServices, TurnEventSink, TurnServices,
};
```

- [ ] **Step 6: 运行验证**

```bash
cargo test -p fabric turn -q
cargo check -p fabric --all-targets
```

Expected: PASS.

### Task 0A.2：增加 CognitiveSession adapter

- [ ] **Step 1: 新增 `crates/cognit/src/harness/session.rs`**

```rust
use crate::harness::config::HarnessConfig;
use crate::harness::linear::{CompactorTrait, ReActLoop};
use anyhow::Result;
use async_trait::async_trait;
use fabric::{TurnMetrics as FabricTurnMetrics, TurnRequest, TurnResult, TurnStop};
use fabric::{TurnEvent, TurnEventSink, TurnServices};
use std::pin::Pin;

struct NoopCompressor;

impl CompactorTrait for NoopCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<fabric::Message>,
        _llm: &'a dyn crate::r#impl::llm::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<fabric::Message>,
        _llm: &'a dyn crate::r#impl::llm::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

#[async_trait]
pub trait CognitiveSession: Send {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult>;
}

pub struct LinearCognitiveSession {
    #[allow(dead_code)]
    inner: ReActLoop,
}

impl LinearCognitiveSession {
    pub fn new(config: HarnessConfig) -> Self {
        Self { inner: ReActLoop::new(config, Box::new(NoopCompressor)) }
    }
}

#[async_trait]
impl CognitiveSession for LinearCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        _services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult> {
        events.emit(TurnEvent::Started { operation_id: request.operation_id }).await;
        let result = TurnResult {
            output: request.input,
            stop: TurnStop::Completed,
            metrics: FabricTurnMetrics { completed_normally: true, ..Default::default() },
        };
        events.emit(TurnEvent::Finished { operation_id: request.operation_id, stop: result.stop.clone() }).await;
        Ok(result)
    }
}
```

Note: This is intentionally a compile seam only. PR-0B replaces echo behavior with the real `ReActLoop::run` wiring after characterization tests are in place.

- [ ] **Step 2: 更新 `crates/cognit/src/harness/mod.rs`**

Add:

```rust
pub mod session;
pub use session::{CognitiveSession, LinearCognitiveSession};
```

- [ ] **Step 3: 运行测试**

```bash
cargo test -p cognit --test cognitive_session linear_session_returns_turn_result -q
cargo check -p cognit --all-targets
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/fabric/src/types/operation.rs crates/fabric/src/types/turn.rs crates/fabric/src/include/turn.rs crates/fabric/src/types/mod.rs crates/fabric/src/include/mod.rs crates/fabric/src/lib.rs crates/cognit/src/harness/session.rs crates/cognit/src/harness/mod.rs crates/cognit/tests/cognitive_session.rs
git commit -m "arch: introduce turn contracts and cognitive session adapter"
```

## 3. PR-0B：TurnService scaffold + characterization tests

**Files:**

- Create: `crates/executive/src/service/mod.rs`
- Create: `crates/executive/src/service/turn_service.rs`
- Create: `crates/executive/src/service/pre_turn.rs`
- Create: `crates/executive/src/service/post_turn.rs`
- Create: `crates/executive/src/service/turn_services.rs`
- Modify: `crates/executive/src/lib.rs`
- Test: `crates/executive/tests/turn_service_equivalence.rs`

### Task 0B.1：建立 TurnService 最小可测主链

- [ ] **Step 1: 写测试**

Create `crates/executive/tests/turn_service_equivalence.rs`:

```rust
use executive::service::{PreTurnPipeline, PostTurnPipeline, TurnService};
use fabric::{NoopTurnEventSink, OperationId, ProcessId, StubTurnServices, TurnRequest, TurnStop};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn turn_service_submits_one_turn() {
    let service = TurnService::new(
        Arc::new(StubTurnServices::default()),
        PreTurnPipeline::default(),
        PostTurnPipeline::default(),
    );

    let result = service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
                process_id: ProcessId::new(),
                session_id: "s1".into(),
                input: "hello".into(),
                working_dir: PathBuf::from("."),
                model_policy: None,
                deadline: None,
            },
            &NoopTurnEventSink,
        )
        .await
        .expect("turn service should submit");

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(result.output, "hello");
}
```

Run:

```bash
cargo test -p executive --test turn_service_equivalence turn_service_submits_one_turn -q
```

Expected: FAIL because `executive::service` does not exist.

- [ ] **Step 2: 新增 `crates/executive/src/service/pre_turn.rs`**

```rust
use anyhow::Result;
use fabric::{RecallRequest, TurnRequest, TurnServices};

#[derive(Debug, Clone, Default)]
pub struct PreTurnPipeline;

impl PreTurnPipeline {
    pub async fn run(&self, mut request: TurnRequest, services: &dyn TurnServices) -> Result<TurnRequest> {
        let recall = services
            .recall(RecallRequest { session_id: request.session_id.clone(), input: request.input.clone() })
            .await?;
        if !recall.snippets.is_empty() {
            request.input = format!("<memory>\n{}\n</memory>\n\n{}", recall.snippets.join("\n"), request.input);
        }
        Ok(request)
    }
}
```

- [ ] **Step 3: 新增 `crates/executive/src/service/post_turn.rs`**

```rust
use anyhow::Result;
use fabric::{TurnResult, TurnStop};

#[derive(Debug, Clone, Default)]
pub struct PostTurnPipeline;

impl PostTurnPipeline {
    pub async fn run(&self, result: TurnResult) -> Result<TurnResult> {
        match result.stop {
            TurnStop::Completed | TurnStop::Blocked | TurnStop::Cancelled | TurnStop::Failed => Ok(result),
        }
    }
}
```

- [ ] **Step 4: 新增 `crates/executive/src/service/turn_service.rs`**

```rust
use crate::service::{PostTurnPipeline, PreTurnPipeline};
use anyhow::Result;
use cognit::harness::{CognitiveSession, HarnessConfig, LinearCognitiveSession};
use fabric::{TurnEventSink, TurnRequest, TurnResult, TurnServices};
use std::sync::Arc;

pub struct TurnService {
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
}

impl TurnService {
    pub fn new(
        services: Arc<dyn TurnServices>,
        pre_turn: PreTurnPipeline,
        post_turn: PostTurnPipeline,
    ) -> Self {
        Self { services, pre_turn, post_turn }
    }

    pub async fn submit(
        &self,
        request: TurnRequest,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult> {
        let request = self.pre_turn.run(request, self.services.as_ref()).await?;
        let mut session = LinearCognitiveSession::new(HarnessConfig::default());
        let result = session.run_turn(request, self.services.as_ref(), events).await?;
        self.post_turn.run(result).await
    }
}
```

- [ ] **Step 5: 新增 module exports**

Create `crates/executive/src/service/mod.rs`:

```rust
pub mod post_turn;
pub mod pre_turn;
pub mod turn_service;
pub mod turn_services;

pub use post_turn::PostTurnPipeline;
pub use pre_turn::PreTurnPipeline;
pub use turn_service::TurnService;
```

Create `crates/executive/src/service/turn_services.rs`:

```rust
pub use fabric::{StubTurnServices, TurnServices};
```

Add to `crates/executive/src/lib.rs`:

```rust
pub mod service;
```

- [ ] **Step 6: 运行测试**

```bash
cargo test -p executive --test turn_service_equivalence turn_service_submits_one_turn -q
cargo check -p executive --all-targets
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/executive/src/service crates/executive/src/lib.rs crates/executive/tests/turn_service_equivalence.rs
git commit -m "refactor: scaffold turn service pipeline"
```

## 4. PR-0C：切换 daemon chat 到 TurnService

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/chat.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs`
- Test: `crates/executive/tests/turn_service_equivalence.rs`

### Task 0C.1：先做 shadow mode，不删除旧逻辑

- [ ] **Step 1: 增加 feature/config gate**

在 handler state 增加可选 `turn_service: Option<Arc<TurnService>>`，默认 `None`，避免一次性切换生产路径。

- [ ] **Step 2: 在 `handle_chat` 开头构造 TurnRequest**

Insert near `handle_chat` request parsing:

```rust
let turn_request = fabric::TurnRequest {
    operation_id: fabric::OperationId::new(),
    process_id: fabric::ProcessId::new(),
    session_id: session_id.clone(),
    input: message.clone(),
    working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    model_policy: None,
    deadline: None,
};
```

- [ ] **Step 3: 仅当 gate 开启时走 TurnService**

```rust
if let Some(turn_service) = &self.turn_service {
    let events = fabric::NoopTurnEventSink;
    let result = turn_service.submit(turn_request, &events).await?;
    return Ok(json!({
        "role": "assistant",
        "content": result.output,
        "stop": format!("{:?}", result.stop),
    }));
}
```

- [ ] **Step 4: 验证旧路径仍工作**

```bash
cargo test -p executive --test turn_service_equivalence -q
cargo check -p executive --all-targets
```

Expected: PASS.

### Task 0C.2：正式切换后删除 handler 内 harness 创建

- [ ] **Step 1: 删除 `chat.rs` 中直接 `build_harness` 创建点**

目标验证命令：

```bash
rg "build_harness|ReActLoop::new" crates/executive/src/impl/daemon/handler/chat.rs
```

Expected: no matches.

- [ ] **Step 2: 运行 daemon/chat 相关测试**

```bash
cargo test -p executive turn_service -q
cargo check --workspace --all-targets
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/executive/src/impl/daemon/handler/chat.rs crates/executive/src/impl/daemon/handler/init.rs crates/executive/src/impl/daemon/handler/mod.rs crates/executive/tests/turn_service_equivalence.rs
git commit -m "refactor: route daemon chat through turn service"
```

## 5. PR-0D：切换 exec 到 TurnService

**Files:**

- Modify: `crates/bin/src/main.rs`
- Test: `crates/bin/tests/integration/main.rs`

### Task 0D.1：替换 `run_exec` 手写 loop

- [ ] **Step 1: 写集成测试或保留现有 exec 测试**

Run current baseline:

```bash
cargo test -p aletheon-bin --test integration -- --nocapture
```

Expected: record current failures before editing; do not expand scope beyond exec path.

- [ ] **Step 2: 在 `run_exec` 中构造 `TurnRequest` 并调用 `TurnService`**

Replacement shape:

```rust
let turn_service = executive::service::TurnService::new(
    std::sync::Arc::new(fabric::StubTurnServices::default()),
    executive::service::PreTurnPipeline::default(),
    executive::service::PostTurnPipeline::default(),
);
let result = turn_service
    .submit(
        fabric::TurnRequest {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            session_id: "exec".to_string(),
            input: prompt.to_string(),
            working_dir: std::env::current_dir()?,
            model_policy: None,
            deadline: None,
        },
        &fabric::NoopTurnEventSink,
    )
    .await?;
println!("{}", result.output);
```

- [ ] **Step 3: 删除 exec 内直接 Tool/LLM loop**

Verify:

```bash
rg "complete\(|ToolUse|execute_tool|build_harness" crates/bin/src/main.rs
```

Expected: no production loop matches in `run_exec`.

- [ ] **Step 4: 运行验证**

```bash
cargo check -p aletheon-bin --all-targets
cargo check --workspace --all-targets
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/bin/src/main.rs crates/bin/tests/integration/main.rs
git commit -m "refactor: route exec through turn service"
```

## 6. PR-0E：清理重复入口 + SandboxFirst fail closed

**Files:**

- Modify: `crates/executive/src/core/orchestrator.rs`
- Modify: `crates/executive/src/impl/daemon/handler/chat.rs`
- Modify: `crates/executive/src/core/verdict_handler.rs`
- Create: `crates/executive/tests/sandbox_first_fail_closed.rs`

### Task 0E.1：SandboxFirst 不再继续真实执行

- [ ] **Step 1: 写失败测试**

Create `crates/executive/tests/sandbox_first_fail_closed.rs`:

```rust
use executive::{AletheonExecutive, ExecutiveConfig};
use fabric::self_field::Verdict;

#[tokio::test]
async fn sandbox_first_without_sandbox_is_blocked() {
    let mut runtime = AletheonExecutive::new(ExecutiveConfig::default());
    let result = runtime
        .handle_verdict_for_test(Verdict::SandboxFirst { reason: "risk".into() })
        .await;

    assert!(format!("{result:?}").contains("SandboxRequiredUnavailable"));
}
```

If `handle_verdict_for_test` does not exist, add a narrow test-only helper behind `#[cfg(test)]` or convert the existing verdict-handler tests to assert the same behavior.

Run:

```bash
cargo test -p executive --test sandbox_first_fail_closed sandbox_first_without_sandbox_is_blocked -q
```

Expected: FAIL because current behavior proceeds.

- [ ] **Step 2: 修改 `orchestrator.rs` SandboxFirst 分支**

Replace the branch that logs `Proceeding without sandbox` with a structured error:

```rust
return Err(anyhow::anyhow!(
    "SandboxRequiredUnavailable: SandboxFirst requested ({reason}) but no sandbox execution path is configured"
));
```

- [ ] **Step 3: 删除 daemon prompt-only SandboxFirst note**

Remove the block that prepends `<selffield-note>SandboxFirst...` to user input. Replace with early blocked response until Admission is available:

```rust
if let Ok(Verdict::SandboxFirst { ref reason }) = verdict {
    return Ok(json!({
        "role": "assistant",
        "content": format!("Blocked: sandbox execution is required before this action can continue. Reason: {reason}"),
        "error": "SandboxRequiredUnavailable"
    }));
}
```

- [ ] **Step 4: 验证 SandboxFirst 搜索结果**

```bash
rg "Proceeding without sandbox|selffield-note>SandboxFirst|sandbox review" crates/executive/src
```

Expected: no production path matches.

- [ ] **Step 5: 运行测试**

```bash
cargo test -p executive verdict_handler -q
cargo test -p executive sandbox_first_fail_closed -q
cargo check --workspace --all-targets
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/core/orchestrator.rs crates/executive/src/impl/daemon/handler/chat.rs crates/executive/src/core/verdict_handler.rs crates/executive/tests/sandbox_first_fail_closed.rs
git commit -m "security: make sandbox-first fail closed"
```

## 7. M0 Gate 验收

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p fabric turn
cargo test -p cognit cognitive_session
cargo test -p executive turn_service
cargo test -p executive sandbox_first_fail_closed
rg "ReActLoop::new|build_harness" crates/executive crates/bin
rg "Proceeding without sandbox|selffield-note>SandboxFirst|sandbox review" crates/executive/src
```

Expected:

```text
cargo fmt/check/test: PASS
ReActLoop/build_harness: only factory/composition/test locations remain
SandboxFirst unsafe downgrade strings: no production matches
```

M0 is complete only when:

- daemon chat and exec both submit `TurnRequest` to `TurnService`；
- handler 不再编排 Cognit 细节；
- `bin` 不再手写模型/工具循环；
- `SandboxFirst` 缺少沙箱时 fail closed；
- 后续 Process/Operation 能以 `TurnService` 为唯一承载入口继续演进。
