# 模块间接口 (Inter-Module Interfaces)

> 模块间通信契约定义。

**关联模块:** 所有模块
**最后更新:** 2026-06-06

---

## Implementation Status

> These are interface contracts defining module boundaries. Not all are fully implemented.
> Status reflects whether the interface is exercised in practice.

| Interface | Status | Notes |
|-----------|--------|-------|
| CognitiveEngine <-> ToolSystem | ✅ Implemented | Engine calls tools via ToolRegistry |
| PerceptionEngine -> CognitiveEngine | ✅ Implemented | PerceptionBridge → injection_tx → engine.drain_perceptions() wired before each turn |
| CognitiveEngine <-> MemorySystem | ✅ Implemented | Core memory reads/writes during loop |
| Security -> ToolSystem | ✅ Implemented | Policy checks before tool execution |
| Orchestration -> ToolSystem | ✅ Implemented | DelegateTool as tool call |

---

## 1. 认知引擎 ↔ 工具系统

```
认知引擎调用工具:
  LlmResponse.tool_calls → ToolRegistry.execute() → ToolResult → messages.push()

工具结果反馈:
  ToolResult → 检查 is_error → 决定重试/跳过/终止
```

## 2. 感知引擎 → 认知引擎

```
PerceptionEvent → EventAggregator → 过滤/去重/聚合
  → 高优先级: 直接注入认知引擎消息队列
  → 低优先级: 写入 Core Memory 的 system_state block
  → 事件统计: 更新 observability metrics
```

## 3. 安全引擎 → 工具系统

```
Tool.execute() 调用前:
  → SecurityEngine.check_permission(tool, input) → Allow/Deny/Confirm
  → LoopDetector.record_call(tool) → 是否触发循环检测
  → WritableRoot.check_path(input) → 路径是否允许

Tool.execute() 调用后:
  → AuditLog.record(tool, input, result)
```

## 4. 编排引擎 → 子 Agent

```
Orchestrator.create_sub_agent(config)
  → AgentRegistry.register(agent_info)
  → 为子 Agent 创建独立 Channel
  → 子 Agent 运行 ReAct 循环
  → 结果通过 Channel 返回父 Agent
```

## 5. 主动行为引擎 → 编排引擎

> ⬜ **Design aspiration only** — ProactiveGoal, GoalQueue, IdleScheduler have NO code.
> This is the most significant architectural gap between OS-Agent's design vision and implementation.

```
ProactiveGoal → GoalQueue.push(goal)
  → IdleScheduler 决定何时执行
  → Orchestrator.execute(goal) → 使用 SingleAgent 策略
```

## 6. 自学习循环 → 记忆系统

> 🔶 **Code exists but not wired** — `learning/` module (outcome, pattern, rule, 312 lines) is standalone;
> not integrated into engine or handler.

```
ToolResult + UserFeedback → OutcomeRecorder.record()
  → 存入 Recall Memory (SQLite)
  → PatternExtractor 定期分析
  → LearnRule → 写入 Core Memory (learned_rules block)
```

---

## Implementation Summary

| Interface | Code Location | Notes |
|-----------|---------------|-------|
| Engine -> ToolRegistry | `engine.rs`, `tool/mod.rs` | Engine calls tools via `ToolRegistry::execute()` |
| Security -> ToolRunner | `security/policy.rs`, `security/runner.rs` | Policy + LoopDetector checks before execution |
| DelegateTool | `orchestration/` | Delegation as tool call |
| Perception -> Engine | `perception/bridge.rs`, `engine.rs:120-167`, `agentd/src/main.rs:146-171` | PerceptionBridge wired via injection_tx → engine.set_perception_rx() → drain_perceptions() |
