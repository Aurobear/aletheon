# 混合推理架构

> 本地优先、云端兜底的推理路由系统，根据任务复杂度自动选择推理后端。

**模块编号:** 09
**关联模块:** [认知引擎 (01)](../core/cognitive-engine.md), [工具系统 (03)](../execution/tool-system.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| IntentClassifier | ✅ Implemented | `inference/classifier.rs` | Rule-based intent classification (keyword + token count) |
| InferenceRouter | ✅ Implemented | `inference/router.rs` | Local/cloud routing with complexity-based selection, runtime upgrade |
| ProviderConfig | ✅ Implemented | `inference/provider_config.rs` | Provider type enum + config struct |

**NOTE:** This module is standalone -- NOT integrated with the engine. Engine uses `ProviderRegistry` directly.

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
- [3. 已识别缺陷](#3-已识别缺陷)
- [4. 改进设计](#4-改进设计)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

OS-Agent 作为永远在线的系统级服务，推理延迟直接影响用户体验。混合推理架构的核心原则是**离线优先**：日常任务用本地模型（llama.cpp + Q4 量化）在 <1s 内完成，复杂推理任务自动升级到云端（DeepSeek/Claude/GPT）。路由决策由轻量意图分类器驱动，整个过程对用户透明。

---

## 2. 当前设计

### 2.1 推理决策树

原始设计（`design.md` §12.1）定义了两级路由：

```
用户请求 / 系统事件
        │
        ▼
┌───────────────────┐
│ 意图分类          │ ← 本地小模型 (1B, <10ms)
│ 简单/中等/复杂    │
└────────┬──────────┘
         │
    ┌────┴────────────────────┐
    │                         │
    ▼                         ▼
┌──────────┐           ┌──────────────┐
│ 本地推理  │           │ 云端推理      │
│          │           │              │
│ llama.cpp│           │ Claude/GPT   │
│ Qwen3-8B │           │ DeepSeek     │
│ Q4 量化   │           │ MiMo         │
│          │           │              │
│ 适合:     │           │ 适合:        │
│ 日常任务  │           │ 复杂推理      │
│ 简单问答  │           │ 代码生成      │
│ 系统控制  │           │ 数学计算      │
│ 模式匹配  │           │ 长文分析      │
│          │           │              │
│ 延迟:<1s │           │ 延迟:1-10s   │
│ 隐私:✓   │           │ 隐私:需授权  │
│ 离线:✓   │           │ 离线:✗       │
└──────────┘           └──────────────┘
```

### 2.2 Provider 配置

原始设计（`design.md` §12.2）定义了 YAML 配置格式，支持 local (llama-cpp), cloud (deepseek/openai/anthropic) 和 routing (default/fallback/escalation_rules) 三个配置段。路由规则包括 token_count、task_type、confidence 条件。

---

## 3. 已识别缺陷

### 3.1 配置层级堆叠问题

当前 Provider 配置是静态 YAML，没有考虑配置来源的优先级和叠加。在实际部署中，配置可能来自系统级、用户级、会话级环境变量覆盖、运行时动态切换。

### 3.2 缺失的推理路由细节

原始决策树只区分了"本地 vs 云端"，但缺少：多模型切换、fallback 链、token 预算感知、会话亲和性。

### 3.3 P1: 意图分类器训练方法缺失

**问题:** 意图分类器作为路由决策的核心组件，将用户任务分为 simple/medium/complex 三类，但关键工程细节完全缺失：无训练数据与训练方法、无准确性目标、"简单"与"复杂"的定义模糊、误路由无降级机制。

**影响:**
- 分类器不可靠则整个"本地处理简单任务、云端处理复杂任务"的架构失去意义
- 如果分类器将 20% 的 complex 任务误判为 simple，重试成本可能超过直接全部路由到云端
- 没有训练方法和准确性目标，Phase 6 实现者无法构建和验证分类器

### 3.4 P2: Token 预算感知路由缺失

**问题:** 当前路由决策仅基于任务复杂度分类，未考虑当前上下文窗口的剩余空间和 token 预算状态。本地 1B 模型的上下文窗口通常远小于云端模型（4K-8K vs 128K+）。

---

## 4. 改进设计

### 4.1 推理 Router 结构

**InferenceRouter** — 推理路由器，根据意图分类结果选择本地或云端 provider。
- 代码位置: `inference/router.rs`
- 路由流程：会话亲和性检查 → 意图分类 → 按复杂度路由（Simple→Local, Medium→CostOptimal, Complex→QualityFirst）

### 4.2 配置层级合并

**ConfigLayer** — Provider 配置层级合并，支持 System/User/Environment/Runtime 四级来源，高优先级覆盖低优先级。合并复用 TOML 配置文件中定义的主 `ConfigLayerStack`（设计文档待补充）。

### 4.3 渐进式意图分类器

分三阶段实现，先规则后 ML，显式定义准确性目标：

**阶段 1 — 基于规则的分类器（Phase 6 可立即实现）:**

```rust
fn classify_intent(task: &str, context: &AgentContext) -> IntentCategory {
    let token_count = tokenizer.encode(task).len();
    let has_complex_keywords = task.contains("调试") || task.contains("分析")
        || task.contains("重构") || task.contains("部署");

    match () {
        _ if token_count > 500 || has_complex_keywords => IntentCategory::Complex,
        _ if token_count > 100 => IntentCategory::Medium,
        _ => IntentCategory::Simple,
    }
}
```

优势：可立即实现、可解释、确定性、零推理延迟。准确性目标：>85% 正确路由率。

**阶段 2 — 数据收集（运行阶段 1 的同时）:** 记录每个任务的路由决策、实际 token 消耗、工具调用次数、成功/失败结果。使用实际复杂度指标回标。积累至少 10,000 条带标签数据后进入阶段 3。

**阶段 3 — ML 分类器（数据充足后）:** 基础模型 `Qwen2.5-0.5B` 或 `Phi-3-mini-4k`，微调为三分类器。准确性目标：>92%，complex-to-local 误路由率 <5%。

**置信度与降级策略:** 置信度低于 0.8 时默认路由到云端（安全侧）。

**运行时复杂度重评估:** 当 iteration > 5 或 tool_calls > 8 时，自动将后续推理升级到云端模型。

### 4.4 用户路由覆盖

用户可通过 CLI 参数强制覆盖分类器决策：

```bash
agent --local "列出当前目录文件"      # 强制本地推理
agent --cloud "分析这段代码的性能瓶颈"  # 强制云端推理
agent "自动判断的任务"                  # 使用分类器
```

覆盖时日志记录 `User override: forced {local|cloud}`，覆盖行为计入 `~/.agent/routing_log.jsonl` 用于后续分析。

---

## 5. 实现要点

- 本地推理引擎（llama.cpp）通过 Rust FFI 绑定集成，注意 GPU 层分配和内存映射
- 意图分类器使用独立的 1B 小模型，推理延迟必须 <10ms，否则成为瓶颈
- Provider 健康检查使用轻量 ping 端点，避免用完整推理请求做探活
- 成本追踪需要记录每个 session 的 token 消耗，用于运行时路由决策和用户账单
- Fallback 链应支持配置化，默认顺序按 provider priority 字段排序

---

## 6. 参考来源

- **原始设计文档:** `docs/plans/2026-06-06-argos-design.md` §12 (混合推理架构)
- **llama.cpp:** 本地推理引擎，GGUF 格式模型加载，GPU offload 支持
- **Anthropic SDK:** content-block 协议、工具循环、上下文压缩（`lib/tools/_beta_runner.py`）
- **OpenCode:** run coordinator 的 demand coalescing 模式（参考其多 provider 调度逻辑）

---

## Implementation Summary

**Code location:** `crates/agent-core/src/inference/`

**Key types/traits implemented:**
- `Complexity` enum (`classifier.rs`) — Simple/Medium/Complex intent categories
- `IntentClassifier` (`classifier.rs`) — rule-based classification using keyword matching and token count thresholds
- `InferenceRouter` (`router.rs`) — local/cloud provider selection based on complexity, runtime upgrade when iteration > 5 or tool_calls > 8
- `ProviderType` enum (`provider_config.rs`) — Local/Cloud provider types
- `ProviderConfig` struct (`provider_config.rs`) — provider configuration with base_url, model, api_key, priority

**Test coverage:** No unit tests found in the inference module.
