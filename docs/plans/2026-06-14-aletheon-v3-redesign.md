# Aletheon v3 重设计方案

日期: 2026-06-14
状态: 已批准

---

## 1. 背景

### 1.1 问题

v2 实现存在以下问题：
- **命名重合**: BodyRuntime（身体执行层）和 Runtime（编排层）名字混淆
- **实现深度不足**: Body 层只有 2 个 BRIDGE 文件，Runtime 层有 2 个 STUB 文件
- **argos-core 未融合**: argos-core 的 94 个文件覆盖了几乎所有 Aletheon 层，但未被迁移

### 1.2 目标

- 重命名 BodyRuntime → Body，Runtime 保持编排层
- 将 argos-core 的所有模块分阶段融合到 aletheon-* crate
- 保持 argos 代码可用直到验证完毕

---

## 2. 命名体系

| 层 | Crate | 职责 |
|---|-------|------|
| **Body** | `aletheon-body` | 身体执行层：工具、沙箱、驱动、MCP |
| **Runtime** | `aletheon-runtime` | 编排层：Agent 生命周期、ReAct 循环、行为路径、会话 |
| **BrainCore** | `aletheon-brain-core` | 认知层：LLM、推理、学习、提供者选择 |
| **SelfField** | `aletheon-self-field` | 主体场：身份、边界、关切、叙事、冲突、注意力、连续性、变异 |
| **Memory** | `aletheon-memory` | 记忆层：4 个 SQLite 后端（保持现状） |
| **EventBus** | `aletheon-event-bus` | 事件总线（保持现状） |
| **ABI** | `aletheon-abi` | 接口定义（保持现状） |

---

## 3. 分阶段融合

### Phase 1: Body

从以下 argos crate 迁移到 `aletheon-body`：
- `argos-tools` (23 files): bash_exec, file_read, file_write, executor, registry, output/, search/
- `argos-sandbox` (12 files): bubblewrap, container, executor, policy
- `argos-driver` (19 files): display, input, OCR, a11y, proc
- `argos-core/mcp`: MCP client, auth, transport, wrapper

```
aletheon-body/
├── src/
│   ├── lib.rs
│   ├── tools/          ← argos-tools
│   ├── sandbox/        ← argos-sandbox
│   ├── driver/         ← argos-driver
│   ├── mcp/            ← argos-core/mcp
│   └── conversions.rs  ← 类型桥接
```

### Phase 2: Runtime

从以下 argos-core 模块迁移到 `aletheon-runtime`：
- `engine` (ReAct loop): run_turn, run_turn_streaming
- `orchestration` (多 Agent): Agent trait, registry, selector, delegate, digraph
- `automation` (调度): cron, webhook, delivery
- `session` (会话): journal, store, observability
- `plugin` (插件): loader, manager, manifest

```
aletheon-runtime/
├── src/
│   ├── lib.rs
│   ├── aletheon_runtime.rs   ← 顶层编排器
│   ├── react_loop.rs         ← 从 engine 迁移
│   ├── agent_runtime.rs      ← Agent 生命周期
│   ├── behavior_paths.rs     ← 行为路径
│   ├── session/              ← argos-core/session
│   ├── orchestration/        ← argos-core/orchestration
│   ├── automation/           ← argos-core/automation
│   └── plugin/               ← argos-core/plugin
```

### Phase 3: BrainCore

从以下 argos-core 模块迁移到 `aletheon-brain-core`：
- `llm` (LLM 提供者): LlmProvider trait, Anthropic, OpenAI
- `inference` (推理路由): InferenceRouter, IntentClassifier
- `learning` (学习): OutcomeRecorder, PatternExtractor, RuleStore
- `provider_registry` (提供者注册): model resolution, API keys

```
aletheon-brain-core/
├── src/
│   ├── lib.rs
│   ├── brain_core.rs
│   ├── reasoner.rs
│   ├── planner.rs
│   ├── reflector.rs
│   ├── critic.rs
│   ├── learner.rs
│   ├── world_model.rs
│   ├── llm/                  ← argos-core/llm
│   ├── inference/            ← argos-core/inference
│   ├── learning/             ← argos-core/learning
│   └── provider_registry.rs  ← argos-core/provider_registry
```

### Phase 4: SelfField

从以下 argos 模块迁移到 `aletheon-self-field`：
- `argos-core/hook` (Hook 系统): HookDispatcher, PreLLMCall/PreToolUse
- `argos-core/resilience` (韧性): guardian, safe_mode, watchdog
- `argos-security` (安全策略): policy, loop_detector, circuit_breaker, rate_limiting, self_protection
- `argos-perception` (感知): screen events, sensory input, FUSE

```
aletheon-self-field/
├── src/
│   ├── lib.rs
│   ├── self_field.rs
│   ├── identity.rs
│   ├── boundary.rs
│   ├── care.rs
│   ├── narrative.rs
│   ├── conflict.rs
│   ├── attention.rs
│   ├── continuity.rs
│   ├── mutation.rs
│   ├── hook/                 ← argos-core/hook
│   ├── resilience/           ← argos-core/resilience
│   ├── security/             ← argos-security
│   └── perception/           ← argos-perception
```

---

## 4. 依赖关系

```
aletheon-abi          ← 所有层依赖
aletheon-event-bus    ← 所有层依赖
aletheon-memory       ← 独立
aletheon-self-field   ← 依赖 abi, event-bus, memory
aletheon-brain-core   ← 依赖 abi, event-bus, memory
aletheon-body         ← 依赖 abi, event-bus
aletheon-runtime      ← 依赖 abi, event-bus, self-field, brain-core, body, memory
```

---

## 5. 渐进迁移策略

### 5.1 每个 Phase 的步骤

1. 在 aletheon-* crate 中创建新模块
2. 从 argos-* crate 复制代码并适配（修改 imports, 类型转换）
3. 更新 aletheon-* 的 Cargo.toml 依赖
4. 运行测试验证
5. argos-* 代码保留到全部验证完毕

### 5.2 最终验证

- 所有 aletheon-* crate 测试通过
- argos-core 的功能被完整覆盖
- 可以安全删除 argos-core 中已迁移的模块

---

## 6. 测试策略

- 每个 Phase 迁移后运行 `cargo test --workspace`
- 保持 821+ 测试通过
- 新增模块需要对应测试

---

## 7. v2 → v3 变更

| 变更 | v2 | v3 |
|------|----|----|
| Body 层命名 | BodyRuntime | Body |
| Body 层实现 | 2 BRIDGE 文件 | 完整 tools/sandbox/driver/mcp |
| Runtime 层实现 | 2 STUB 文件 | 完整 engine/orchestration/automation/session |
| BrainCore 实现 | 4 REAL + 2 STUB | 完整 llm/inference/learning/provider_registry |
| SelfField 实现 | 9 REAL | 完整 + hook/resilience/security/perception |
| argos-core | 独立存在 | 分阶段融合到 aletheon-* |
