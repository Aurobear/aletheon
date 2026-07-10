> Merged from docs/design/security/loop-detector.md + docs/design/security/security-model.md §4.1-4.3, §4.13 — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# 循环检测器 (LoopDetector)

> 工具调用链的看门人——检测循环模式、输出异常、风险等级，防止 Agent 死循环。

**模块编号:** 05-子模块
**父模块:** [安全模型](security-model.md)
**关联模块:** [WritableRoot 路径隔离](writable-root.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| LoopDetector | ✅ Implemented | `security/loop_detector.rs` | Stagnation + fail-streak detection |
| RiskClassifier | ✅ Implemented | `security/risk_classifier.rs` | 4-level risk classification with thresholds |
| CircuitBreaker | ✅ Implemented | `security/circuit_breaker.rs` | Per-turn consecutive block → interrupt |
| MultiAgentLoopDetector | ⬜ Planned | — | Per-agent state isolation not started |

---

## 目录

- [1. 已识别缺陷](#1-已识别缺陷)
  - [1.1 P0: 工具调用循环检测/Guardrail](#11-p0-工具调用循环检测guardrail)
  - [1.2 P1: LoopDetector 全局追踪不区分 Agent](#12-p1-loopdetector-全局追踪不区分-agent)
- [2. 改进设计](#2-改进设计)
  - [2.1 LoopDetector 概览](#21-loopdetector-概览)
  - [2.2 循环模式检测](#22-循环模式检测)
  - [2.3 完整 LoopDetector 实现](#23-完整-loopdetector-实现)
  - [2.4 Per-Agent LoopDetector 状态隔离](#24-per-agent-loopdetector-状态隔离)

---

## 1. 已识别缺陷

### 1.1 P0: 工具调用循环检测/Guardrail

**问题：** Agent 可能陷入工具调用死循环——反复调用同一失败工具、两个工具之间无限互相触发、或在 token 消耗不断增加的情况下没有任何实质进展。当前安全模型只检查单次调用的权限级别，没有对调用序列做模式分析。

**典型场景：**

| 场景 | 表现 | 后果 |
|------|------|------|
| 同工具重复失败 | `bash("make")` 连续失败 10 次，每次错误相同 | token 浪费，Agent 无法自行跳出 |
| 双工具互锁 | tool_a 调用 tool_b，tool_b 调用 tool_a，无限循环 | CPU 和 token 双重浪费 |
| 无进展循环 | 连续调用 20 个工具，但系统状态未发生任何变化 | 表面上在"工作"，实际毫无产出 |

**需要的防护：**
1. **相同工具+参数连续失败 N 次** — 阻断并报告
2. **连续 M 次失败** — 升级到人工介入
3. **token 消耗无变化** — 提示无进展，建议人工检查

### 1.2 P1: LoopDetector 全局追踪不区分 Agent

**问题：** LoopDetector 的检测状态（滑动窗口、失败计数、熔断器）使用全局状态，在多 Agent 场景下产生误判和漏检。

| 缺陷 | 全局追踪的问题 | 后果 |
|------|---------------|------|
| Same-Call Detection 不区分 Agent | `fs_agent` 和 `code_agent` 都调用 `file_read("/config/app.toml")`，全局窗口出现两次相同调用 | 合法调用被误判为循环 |
| Fail-Streak 全局累计 | `fs_agent` 连续 3 次失败后，`code_agent` 的无关操作也被 CircuitBreaker 阻断 | 一个代理的安全问题影响其他代理正常执行 |
| Risk-Classifier 阈值全局统一 | `code_agent` 频繁读文件，ReadOnly 阈值 5 可能过低；`deploy_agent` 系统变更应更严格 | 无法为不同类型代理设置差异化策略 |
| CircuitBreaker 状态全局共享 | 子代理 A 被 Block 2 次 + 子代理 B 被 Block 1 次 = 全局计数 3 → InterruptTurn | 正在正常工作的父代理被错误中断 |

**影响：** 3+ 并发子代理场景下误报率显著上升。真正的循环被多样化调用"稀释"而漏检。故障隔离失败——违反多代理系统应有的隔离原则。

**参考来源：** Codex Guardian `GuardianRejectionCircuitBreaker`（per-turn scoped，需扩展为 per-agent）。

---

## 2. 改进设计

### 2.1 LoopDetector 概览

`LoopDetector` 作为工具调用链的看门人，独立于策略引擎运行。它按 `turn_id` 维护滑动窗口，记录每个推理轮次内的工具调用历史，实时检测循环模式、输出异常和风险等级。检测范围包含五个子系统：

- **SameCall / FailStreak / Stagnation** — 三种调用模式检测（算法层面）
- **CircuitBreaker** — 连续阻断达到阈值时中断整个推理轮次，防止卡死
- **OutputGuardrail** — 工具执行后验证输出是否符合预期，失败则注入反馈并允许有限重试
- **RiskClassifier** — 按工具类型和参数将调用分类为不同风险等级，动态调整检测阈值
- **Metrics** — 每次判定都发射遥测指标，支持阈值调优和运行时监控

```
工具调用请求 ──▶ ┌──────────────────────────────────────────────────┐
                 │  LoopDetector (per-turn scoped)                  │
                 │                                                  │
                 │  ┌──────────────┐                                │
                 │  │ RiskClassifier│── 动态阈值 ──┐                │
                 │  └──────────────┘               │                │
                 │                                  ▼                │
                 │  ┌─────────────┐  ┌────────────┐  ┌──────────┐ │
                 │  │ SameCall    │  │ FailStreak │  │ Stagnation│ │
                 │  │ Detector    │  │ Detector   │  │ Detector  │ │
                 │  └──────┬──────┘  └─────┬──────┘  └─────┬─────┘ │
                 │         │               │               │        │
                 │  ┌──────┴───────────────┴───────────────┴──────┐ │
                 │  │ Verdict: Allow / Warn / Block / Escalate     │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │                       │                          │
                 │  ┌────────────────────▼────────────────────────┐ │
                 │  │ CircuitBreaker (per-turn)                   │ │
                 │  │ consecutive_blocks ≥ 3 → InterruptTurn      │ │
                 │  │ recent_blocks(window=50) ≥ 10 → InterruptTurn│ │
                 │  └────────────────────┬────────────────────────┘ │
                 │                       │                          │
                 │  (post-execution)     ▼                          │
                 │  ┌────────────────────────────────────────────┐ │
                 │  │ OutputGuardrail                           │ │
                 │  │ validate(output) → pass / fail+retry       │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │                       │                          │
                 │                       ▼                          │
                 │  Allow / Warn / Block / Escalate / InterruptTurn │
                 │  + Metrics emission                              │
                 └──────────────────────────────────────────────────┘
```

### 2.2 循环模式检测

**核心模式：**

| 模式 | 检测条件 | 动作 |
|------|----------|------|
| **相同调用重复** | 同一工具名 + 参数哈希，连续出现 N 次 | `Block` — 阻断，返回错误提示 |
| **连续失败** | 工具返回 `is_error: true`，连续 M 次 | `Escalate` — 升级到人工确认 |
| **无进展** | 最近 K 次调用后，token 消耗变化 < 阈值 且 无成功工具结果 | `Warn` — 告警，建议重新规划 |

**风险分级（动态阈值）：**

不同风险等级的工具调用使用不同的检测阈值，高风险操作触发更严格的防护：

| 风险等级 | 工具示例 | same_call_threshold | fail_streak_threshold | 首次失败动作 |
|----------|----------|---------------------|-----------------------|-------------|
| **ReadOnly** (L0) | read_file, grep, ls | 5 | 7 | Allow |
| **FileModification** (L1) | write_file, bash("make") | 3 | 5 | Allow |
| **SystemChange** (L2) | systemctl, pacman, iptables | 2 | 3 | Warn |
| **Destructive** (L2+) | rm, mkfs, dd | 2 | 2 | Warn + 立即记录审计 |

**熔断器（CircuitBreaker）模式：**

单次调用的 Block 不足以解决"Agent 卡死"问题——Agent 可能尝试变体参数绕过 Block，但仍在同一任务上空转。熔断器在推理轮次级别追踪阻断累积：

| 触发条件 | 阈值 | 动作 |
|----------|------|------|
| 连续阻断次数（同一 turn） | 3 次 | `InterruptTurn` — 中断整个推理轮次 |
| 滑动窗口内阻断次数（窗口=50） | 10 次 | `InterruptTurn` — 中断整个推理轮次 |

`InterruptTurn` 的语义：停止当前 turn 的所有后续工具调用，向用户报告阻断原因和建议，等待用户显式指示后才开始新 turn。

**输出验证（OutputGuardrail）模式：**

工具执行完成后，验证输出是否合理（受 CrewAI Guardrail 启发）：

| 验证规则 | 适用范围 | 失败动作 |
|----------|----------|----------|
| 非空输出检查 | 所有工具 | 注入错误上下文，允许 agent 重试（上限 2 次） |
| 退出码检查 | bash/shell 工具 | 非零退出码视为失败 |
| JSON schema 验证 | 返回结构化数据的工具 | 输出不符合 schema 时重试 |
| 语义一致性检查 | L1+ 工具（可选） | 输出与输入意图明显矛盾时告警 |

验证失败时，将失败原因注入 Agent 上下文并允许有限重试（max_retries=2），而非直接阻断。这给 Agent 一个修正输出的机会，与 CrewAI 的 guardrail-on-failure 模式一致。

### 2.3 完整 LoopDetector 实现

**LoopDetector** — 检测工具调用循环模式（同工具重复、失败连续、无进展停滞），配合 RiskClassifier（风险分类）和 CircuitBreaker（熔断器）工作。
- 代码位置: `security/loop_detector.rs`
- 配合 RiskClassifier（`security/risk_classifier.rs`）和 CircuitBreaker（`security/circuit_breaker.rs`）工作

**RiskCategory** — 四级风险分类：ReadOnly / FileModification / SystemChange / Destructive，每级有不同的 same_call_threshold 和 fail_streak_threshold。

**RiskClassifier** — 根据工具名和参数判定风险等级，支持用户可配置的规则列表（glob 模式匹配工具名，可选参数匹配）。内置默认规则覆盖 rm/mkfs/dd（Destructive）、systemctl/pacman/iptables（SystemChange）、read_file/ls/cat/grep/find（ReadOnly）。

**LoopDetectorConfig** — 配置项：window_size(50), default_same_call_threshold(3), default_fail_streak_threshold(5), stagnation_token_delta(100), stagnation_call_threshold(10), output_guardrail_max_retries(2), circuit_breaker_max_consecutive(3), circuit_breaker_max_recent(10), circuit_breaker_window_size(50)。

**ToolCallRecord** — 调用记录：tool_name, args_hash, is_error, token_cost, timestamp, state_changed, turn_id。

**LoopVerdict** — 检测结果：Allow / Warn / Block / Escalate / InterruptTurn。

**OutputGuardrail** — 输出验证管理器，包含 NonEmptyOutputValidator 和 ExitCodeValidator，失败时注入错误上下文并允许重试（max_retries=2）。

**LoopCircuitBreaker** — 按 turn_id 分组的熔断状态，连续 Block 达阈值或滑动窗口内 Block 累积时触发 InterruptTurn。

**LoopDetectorMetrics** — 遥测指标：total_checks, allows, warnings, blocks, escalations, circuit_breaker_trips, output_guardrail_failures/retries, detector_errors。

**核心接口：**
- `pre_check(tool_name, args, turn_id)` — 调用前模式匹配检测
- `post_check(tool_name, args, is_error, token_cost, turn_id)` — 调用后记录结果
- `record_and_check()` — 合一接口，Fail-closed 语义
- `validate_output(tool_name, output)` — 输出验证
- `end_turn(turn_id)` — turn 结束清理

**ToolRunnerWithGuard** — 与工具运行器集成，执行流程：策略引擎权限检查 → 循环检测 pre-check → 执行工具（含输出验证重试） → 循环检测 post-check → 审计记录。

### 2.4 Per-Agent LoopDetector 状态隔离

> ⬜ **Planned** — 保持完整设计。

**MultiAgentLoopDetector** — 将 LoopDetector 的全局状态拆分为 per-agent 状态，解决全局追踪误判问题。

- 每个 agent 拥有独立的 AgentLoopState（call_window, fail_streak, circuit_breaker, threshold_override）
- 父代理可为每个子代理配置差异化阈值
- AggregateLoopState 提供所有子代理的安全健康摘要（Healthy / Degraded / Critical）
- Fail-closed 保持：单个 agent 的 LoopDetector 异常 → 阻断该 agent，不影响其他 agent

---

*源文档: [安全模型](security-model.md) §3.1, §3.6, §4.1-4.3, §4.13*


---

## Appendix: Additional Design Details (from security-model.md)

### 4.1 LoopDetector 概览

`LoopDetector` 作为工具调用链的看门人，独立于策略引擎运行。它按 `turn_id` 维护滑动窗口，记录每个推理轮次内的工具调用历史，实时检测循环模式、输出异常和风险等级。检测范围包含五个子系统：

- **SameCall / FailStreak / Stagnation** — 三种调用模式检测
- **CircuitBreaker** — 连续阻断达到阈值时中断整个推理轮次
- **OutputGuardrail** — 工具执行后验证输出是否符合预期
- **RiskClassifier** — 按工具类型和参数将调用分类为不同风险等级
- **Metrics** — 每次判定都发射遥测指标

```
工具调用请求 ──▶ ┌──────────────────────────────────────────────────┐
                 │  LoopDetector (per-turn scoped)                  │
                 │  ┌──────────────┐                                │
                 │  │ RiskClassifier│── 动态阈值 ──┐                │
                 │  └──────────────┘               │                │
                 │                                  ▼                │
                 │  ┌─────────────┐  ┌────────────┐  ┌──────────┐ │
                 │  │ SameCall    │  │ FailStreak │  │ Stagnation│ │
                 │  │ Detector    │  │ Detector   │  │ Detector  │ │
                 │  └──────┬──────┘  └─────┬──────┘  └─────┬─────┘ │
                 │         │               │               │        │
                 │  ┌──────┴───────────────┴───────────────┴──────┐ │
                 │  │ Verdict: Allow / Warn / Block / Escalate     │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  ┌────────────────────▼────────────────────────┐ │
                 │  │ CircuitBreaker (per-turn)                   │ │
                 │  │ consecutive_blocks ≥ 3 → InterruptTurn      │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  ┌────────────────────────────────────────────┐ │
                 │  │ OutputGuardrail                           │ │
                 │  │ validate(output) → pass / fail+retry       │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  Allow / Warn / Block / Escalate / InterruptTurn │
                 │  + Metrics emission                              │
                 └──────────────────────────────────────────────────┘
```

### 4.2 循环模式检测

**核心模式：**

| 模式 | 检测条件 | 动作 |
|------|----------|------|
| **相同调用重复** | 同一工具名 + 参数哈希，连续出现 N 次 | `Block` |
| **连续失败** | 工具返回 `is_error: true`，连续 M 次 | `Escalate` |
| **无进展** | 最近 K 次调用后，token 消耗变化 < 阈值 且 无成功结果 | `Warn` |

**风险分级（动态阈值）：**

| 风险等级 | 工具示例 | same_call_threshold | fail_streak_threshold | 首次失败动作 |
|----------|----------|---------------------|-----------------------|-------------|
| **ReadOnly** (L0) | read_file, grep, ls | 5 | 7 | Allow |
| **FileModification** (L1) | write_file, bash("make") | 3 | 5 | Allow |
| **SystemChange** (L2) | systemctl, pacman, iptables | 2 | 3 | Warn |
| **Destructive** (L2+) | rm, mkfs, dd | 2 | 2 | Warn + 立即记录审计 |

**熔断器（CircuitBreaker）模式：**

| 触发条件 | 阈值 | 动作 |
|----------|------|------|
| 连续阻断次数（同一 turn） | 3 次 | `InterruptTurn` |
| 滑动窗口内阻断次数（窗口=50） | 10 次 | `InterruptTurn` |

**输出验证（OutputGuardrail）模式：**

| 验证规则 | 适用范围 | 失败动作 |
|----------|----------|----------|
| 非空输出检查 | 所有工具 | 注入错误上下文，允许重试（上限 2 次） |
| 退出码检查 | bash/shell 工具 | 非零退出码视为失败 |
| JSON schema 验证 | 结构化数据工具 | 输出不符合 schema 时重试 |

### 4.3 完整 LoopDetector 实现

> 完整实现见 [loop-detector.md](loop-detector.md)。以下为关键组件摘要。

**核心组件：**
- **RiskCategory** — 四级风险分类：ReadOnly / FileModification / SystemChange / Destructive
- **RiskClassifier** — 根据工具名和参数判定风险等级，支持用户可配置规则
- **LoopDetectorConfig** — 滑动窗口(50)、阈值、熔断器参数等配置
- **ToolCallRecord** — 调用记录（tool_name, args_hash, is_error, token_cost, turn_id）
- **LoopVerdict** — Allow / Warn / Block / Escalate / InterruptTurn
- **OutputGuardrail** — 输出验证（非空、退出码），失败注入上下文+重试(max 2)
- **LoopCircuitBreaker** — 连续 Block 达阈值(3)或滑动窗口累积(10/50)时 InterruptTurn
- **LoopDetectorMetrics** — 遥测指标

**核心接口：**
- `pre_check(tool_name, args, turn_id)` — 调用前模式匹配
- `post_check(tool_name, args, is_error, token_cost, turn_id)` — 调用后记录
- `record_and_check()` — 合一接口，Fail-closed
- `validate_output(tool_name, output)` — 输出验证

**ToolRunnerWithGuard** — 集成循环检测的工具运行器，执行流程：权限检查 → pre-check → 执行(含重试) → post-check → 审计

**RiskClassifier 内置默认规则（部分）：**

| 工具模式 | 风险等级 |
|----------|----------|
| `rm`, `mkfs*`, `dd`, `shutdown`, `reboot` | Destructive |
| `systemctl`, `pacman`, `iptables`, `mount`, `useradd` | SystemChange |
| `read_file`, `ls`, `cat`, `grep`, `find` | ReadOnly |
| 其他工具 | FileModification（默认） |

### 4.13 Per-Agent LoopDetector 状态隔离

将 LoopDetector 的全局状态拆分为 per-agent 状态：

```rust
struct MultiAgentLoopDetector {
    agent_detectors: HashMap<String, AgentLoopState>,
    aggregate_view: AggregateLoopState,
    global_config: LoopDetectorConfig,
}

struct AgentLoopState {
    agent_id: String,
    call_window: VecDeque<ToolCallRecord>,
    fail_streak: u32,
    circuit_breaker: CircuitBreakerState,
    threshold_override: Option<RiskThresholds>,
}
```

**Per-Agent 阈值配置** — 父代理可为每个子代理配置差异化阈值。**聚合报告** — 父代理可查看所有子代理的安全健康摘要。Fail-closed 保持：单个 agent 的 LoopDetector 异常只阻断该 agent。

---
