# Mock 策略 (Mock Strategy)

> Mock LLM、eBPF、沙箱等外部依赖，实现快速可靠的集成测试。

**关联模块:** [测试策略](test-strategy.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Mock Strategy | ⬜ Planned | — | Conceptual design only |

---

## 1. Mock 设计原则

1. **接口即契约** — Mock 实现 Trait，而非具体类型。只要实现 `LlmProvider`、`SandboxBackend`、`PerceptionSource` 等 trait，即可替换。
2. **行为可编程** — Mock 可预置行为序列：正常响应 → 特定错误 → 超时 → 异常。
3. **调用可观测** — Mock 记录所有调用，测试结束后可断言调用次数、顺序和参数。
4. **轻量** — Mock 零外部依赖，纯内存实现，<10μs 延迟。

---

## 2. Mock LLM

### 2.1 MockLlmProvider

```rust
struct MockLlmProvider {
    responses: VecDeque<LlmResponse>,
    recorded_calls: Vec<LlmRequest>,
}

impl MockLlmProvider {
    /// 预设响应列表，按调用顺序依次返回
    fn with_responses(responses: Vec<LlmResponse>) -> Self { ... }

    /// 脚本书写模式：消息列表 → 响应 的映射
    fn with_scripted_conversation(script: Vec<(Vec<ContentBlock>, LlmResponse)>) -> Self { ... }

    /// 获取所有调用记录，用于断言
    fn recorded_calls(&self) -> &[LlmRequest] { ... }
}
```

### 2.2 应用场景

| 场景 | Mock 设置 | 测试内容 |
|------|----------|----------|
| 正常推理 | 预设一个 tool_use 响应 | 引擎正确调用工具 |
| 连续工具调用 | 预设 2 tool_use + 1 text 响应 | 引擎在 2 次工具调用后返回最终响应 |
| LLM 返回空 | 预设空响应 | 引擎处理空响应的逻辑 |
| LLM 超时 | 返回 `LlmError::Timeout` | 降级链触发 |
| 流式响应 | `complete_stream` 返回预设 deltas | TUI 正确增量渲染 |

---

## 3. Mock 感知源

### 3.1 MockPerceptionSource

```rust
struct MockPerceptionSource {
    events: VecDeque<PerceptionEvent>,
}

impl MockPerceptionSource {
    /// 预设事件列表，按订阅顺序依次推送
    fn with_events(events: Vec<PerceptionEvent>) -> Self { ... }

    /// 测试中动态注入事件
    fn emit(&mut self, event: PerceptionEvent) { ... }
}
```

### 3.2 应用场景

| 场景 | Mock 设置 | 测试内容 |
|------|----------|----------|
| CPU 过载事件 | 推送 `PerceptionEvent::System(SysEvent::CpuOverload)` | 引擎收到后自动介入 |
| 日志错误 | 推送 journald ERROR 事件序列 | EventAggregator 正确去重 |
| 事件风暴 | 1s 内推送 10000 个事件 | EventFloodProtector 洪水防护 |
| 背压测试 | 引擎处理慢，持续推送事件 | BackpressureSignal 正确触发 |

---

## 4. Mock 沙箱

### 4.1 MockSandbox

```rust
struct MockSandbox {
    allowed_commands: HashSet<String>,
    execution_log: Vec<CommandExecution>,
}

impl MockSandbox {
    /// 只允许预设命令，其余返回 PermissionDenied
    fn with_allowed_commands(cmds: Vec<&str>) -> Self { ... }

    /// 断言某命令已被执行过
    fn assert_command_executed(&self, cmd: &str) { ... }

    /// 断言命令执行次数
    fn assert_command_executed_times(&self, cmd: &str, times: u32) { ... }
}
```

### 4.2 应用场景

| 场景 | Mock 设置 | 测试内容 |
|------|----------|----------|
| 命令执行 | `mock.allowed_commands = ["ls"]` | bash_exec 正确调用沙箱 |
| 命令阻断 | `mock.allowed_commands = []` | 未授权命令被拦截 |
| 工具并行 | 同时派发 3 个 readonly 命令 | 并发执行正确保序 |
| 沙箱逃逸 | mock 返回模拟逃逸结果 | 安全层正确检测 |

---

## 5. Mock 其他组件

### 5.1 MockMemoryStore

```rust
struct MockMemoryStore {
    core_blocks: HashMap<String, String>,
    recall_entries: Vec<String>,
}

impl MockMemoryStore {
    fn with_core_block(label: &str, content: &str) -> Self { ... }
    fn assert_block_updated(&self, label: &str, expected: &str) { ... }
}
```

### 5.2 MockProvider (LLM Provider 用于推理路由)

```rust
struct MockProvider {
    name: String,
    latency: Duration,
    max_tokens: u32,
}
```

---

## 6. Mock 组合测试

对于多模块集成测试，组合多个 Mock：

```rust
fn setup_mock_env() -> (Engine, MockLlmProvider, MockPerceptionSource, MockSandbox) {
    let llm = MockLlmProvider::with_responses(vec![
        mock_tool_use("bash_exec", r#"{"cmd":"ls"}"#),
        mock_text_response("done!"),
    ]);
    let perception = MockPerceptionSource::with_events(vec![mock_cpu_event()]);
    let sandbox = MockSandbox::with_allowed_commands(vec!["ls"]);
    let engine = Engine::new(llm, sandbox, ...);
    (engine, llm, perception, sandbox)
}

#[tokio::test]
async fn test_perception_triggers_system_diagnosis() {
    let (engine, _llm, perception, _sandbox) = setup_mock_env();

    // 注入 CPU 过载事件
    perception.emit(mock_cpu_event());

    // 触发一次推理
    let response = engine.run_turn("诊断系统状态").await.unwrap();

    // 验证引擎调用了系统诊断工具
    assert_tool_was_called(response, "system_status");
}
```

---

## 7. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Rust 模式 | `#[async_trait]` + generic Mock via trait impl |
| Claude Code | 测试中 injest 感知事件的模式 |
| OpenCode | MockLlmProvider + scripted conversation 模式 |
| Hermes Agent | 沙箱 mock + 命令执行日志断言 |
| Codex | 组合 mock 测试环境 (`setup_mock_env` pattern) |

---

## Implementation Summary

> Mock 策略为概念设计，尚未系统化实现。

| Component | Status | Notes |
|-----------|--------|-------|
| MockLlmProvider | 未实现 | 设计完成，等待实现 |
| MockPerceptionSource | 未实现 | — |
| MockSandbox | 未实现 | — |
| MockMemoryStore | 未实现 | — |
| 组合 Mock 测试框架 | 未实现 | — |
