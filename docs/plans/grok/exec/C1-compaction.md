# C1 可执行 Spec：Compaction 引擎强化

> 对应研究文档 `../12-compaction-engine.md`。优先级 P3。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：在 Aletheon **已有的 tail-keep compaction**（`CompactorTrait` + `AdvancedCompressor` + 3-pass prune）之上，补 Grok 验证过的三项能力：(a) **guardrails**——degenerate summary 检测、失败分类、不静默丢全部上下文；(b) **tool-pair 原子性**——保证 tool_call + tool_result 永不被切分；(c) **策略选择接口**——让 conscious 仲裁在 tail-keep / full-replace / 促进记忆之间选择，而非硬编码单策略。

**非目标**：
- 不替换 `CompactorTrait`/`AdvancedCompressor`（tail-keep 已工作）。
- 本期不实现 Grok 的 inter/chunked 压缩（只加 full-replace 作为第二策略 + guardrails）。
- 不复制 Grok 的 prompt 模板（Aletheon 领域更广）。
- 不改 token 计数（Aletheon 已有）。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `CompactorTrait` | `crates/fabric/src/include/compaction.rs:37-49` | `maybe_compact(msgs, llm)->bool`、`force_compact(msgs, llm)->bool` |
| `prune_tool_outputs` | 同上 `:56-60` | 3-pass：dedup / summarize old / truncate args |
| `truncate_utf8_bytes` | 同上 `:23-32` | UTF-8 安全截断 |
| 导出 | `crates/fabric/src/lib.rs:345` | `prune_tool_outputs, truncate_utf8_bytes, CompactorTrait` |
| `HarnessConfig` | `crates/cognit/src/harness/config.rs:10-29` | `compaction_enabled`、`tail_token_budget=16_000`、`context_window_tokens=128_000` |
| 触发点 | `crates/cognit/src/harness/linear/mod.rs:90,320-330` | `compressor: Box<dyn CompactorTrait>`；loop 内 `maybe_compact` |
| 具体策略 | `mnemosyne::AdvancedCompressor` | 实现 `CompactorTrait`（tail-keep + summary） |
| `TurnEventV1::CompactionTriggered` | `crates/fabric/src/ipc/stream.rs` | 已存在 `{ used_tokens, threshold, reason }` |
| `LatestConsciousContextPort` | `crates/fabric/src/types/conscious_arbitration.rs:29` | conscious 仲裁读端口 |
| `ConsciousContextProjection` | `crates/fabric/src/types/conscious_core.rs:274` | conscious 上下文投影 |
| `ConsciousContextSlot` | `crates/executive/src/service/conscious_context_slot.rs:14` | 绑定 latest conscious context reader（38 行，端口壳） |

**核心缺口**：`CompactorTrait` 返回 `bool`（是否压缩），无失败/降级/degenerate 信号；无策略选择；prune 未文档化 tool-pair 原子性保证。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Fabric 定义强化后的 compaction 结果类型；Cognit harness 拥有触发循环；Conscious-core 提供策略仲裁（可选）；Mnemosyne 消费"dropped→promote"。
2. **scope**：压缩是 per-session/thread 的消息缓冲操作，无跨用户状态。
3. **crash 恢复**：压缩失败保留原缓冲（不破坏）；已提交的 summary 走现有 session 持久化。
4. **fail 模式**：degenerate summary / LLM 失败 → **不压缩、保留原上下文**、记事件（fail-safe，非 fail-closed——宁可上下文超长也不丢内容）。
5. **上限**：summary 最小种子字符数；tail_token_budget 已有；full-replace summary 长度上限。
6. **兼容**：新增 `maybe_compact_v2` 返回富结果；旧 `maybe_compact` 保留（默认适配）。
7. **进 event spine**：复用 `CompactionTriggered`，新增 `CompactionOutcome` 事件（strategy、kept/dropped、degenerate?）。
8. **许可证**：重新实现 guardrail 语义，不复制 Grok `xai-grok-compaction` 源码。

## 4. 类型定义

### 4.1 强化结果类型 — `crates/fabric/src/include/compaction.rs`（追加）

```rust
/// 压缩策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompactionStrategy {
    /// 保留头 + 近尾，裁中间（现有 AdvancedCompressor 行为）。
    TailKeep,
    /// 全量替换：整段总结为 summary 前缀 + 近尾。
    FullReplace,
    /// 不压缩，把可丢弃段促进到 Mnemosyne 后移除。
    PromoteToMemory,
}

/// 压缩尝试的富结果。替代裸 bool。
#[derive(Debug, Clone)]
pub struct CompactionOutcome {
    pub strategy: CompactionStrategy,
    /// 是否实际改动了 messages。
    pub applied: bool,
    pub tokens_before: usize,
    pub tokens_after: usize,
    /// 被移出主缓冲、可促进记忆的消息（PromoteToMemory 用）。
    pub evicted: Vec<crate::message::Message>,
    pub failure: Option<CompactionFailure>,
}

/// 压缩失败/降级原因。出现时 messages 保持未改动（fail-safe）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionFailure {
    /// LLM 返回退化 summary（过短/空/重复）。
    DegenerateSummary { reason: String },
    /// 会话太短，无意义可总结。
    TooShortToSummarize,
    /// summarization LLM 调用失败。
    SamplerError { detail: String },
}

/// degenerate 检测（对齐 Grok is_degenerate_summary 语义）。
pub const MIN_SUMMARY_SEED_CHARS: usize = 200;

pub fn is_degenerate_summary(summary: &str) -> bool {
    let trimmed = summary.trim();
    trimmed.is_empty()
        || trimmed.chars().count() < 40
        // 全是重复的单一 token/行
        || is_mostly_repetition(trimmed)
}

fn is_mostly_repetition(s: &str) -> bool {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 3 { return false; }
    let unique: std::collections::HashSet<&str> = lines.iter().copied().collect();
    unique.len() * 3 < lines.len() // 唯一行 < 1/3 → 判为重复
}
```

### 4.2 tool-pair 原子性保证 — 同文件

```rust
/// 计算 tail-keep 的安全切点：保证不切断 tool_call/tool_result 对。
/// 返回可安全丢弃的 [0, cut) 区间；cut 落在一个完整对的边界上。
pub fn safe_tail_cut(messages: &[crate::message::Message], keep_from: usize) -> usize {
    let mut cut = keep_from.min(messages.len());
    // 若 cut 落在一个 tool_result 前而其 tool_call 在 cut 之后，或反之，回退 cut
    // 直到 [cut..] 内所有 tool_result 都能在 [cut..] 内找到配对 tool_call。
    while cut > 0 && splits_tool_pair(messages, cut) {
        cut -= 1;
    }
    cut
}

fn splits_tool_pair(messages: &[crate::message::Message], cut: usize) -> bool {
    // 实现：扫描 [cut..]，若存在 tool_result 其对应 tool_call_id 的 tool_call 在 [0..cut)，
    // 则该切点切断了对。见任务 T5。
    let _ = (messages, cut);
    unimplemented!()
}
```

### 4.3 富接口（附加方法） — `CompactorTrait`

```rust
// 追加到 CompactorTrait，带默认实现桥接旧 maybe_compact：
fn maybe_compact_v2<'a>(
    &'a mut self,
    messages: &'a mut Vec<Message>,
    llm: &'a dyn LlmProvider,
    strategy: CompactionStrategy,
) -> Pin<Box<dyn Future<Output = anyhow::Result<CompactionOutcome>> + Send + 'a>>;
// 默认实现：忽略 strategy，调 maybe_compact，包装为 TailKeep 的 CompactionOutcome。
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 修改 | `crates/fabric/src/include/compaction.rs` | 追加结果类型、degenerate 检测、safe_tail_cut、`maybe_compact_v2` 默认方法 |
| 修改 | `crates/fabric/src/lib.rs:345` | 导出新类型 |
| 修改 | `mnemosyne::AdvancedCompressor` | 实现 `maybe_compact_v2`：用 safe_tail_cut + degenerate 检测 + 富结果 |
| 修改 | `crates/cognit/src/harness/linear/mod.rs:90+` | 循环调 v2；degenerate/failure → 保留上下文 + 发事件；evicted → 交 promote 回调 |
| 新增 | conscious 策略选择挂钩（可选，flag 后） | 从 `LatestConsciousContextPort` 读仲裁，选 strategy；默认 TailKeep |
| 新增 | evicted→Mnemosyne promote 桥 | PromoteToMemory 策略用 |
| 修改 | feature flag | `grok_hardening.compaction_v2` 默认关（关时走旧 maybe_compact） |

## 6. 任务分解（TDD）

**阶段 A：guardrails（纯函数，最高价值先做）**
- T1. 追加 `CompactionStrategy`/`CompactionOutcome`/`CompactionFailure`。`cargo check -p fabric`。
- T2. `is_degenerate_summary`：单测——空/过短/重复行 → true；正常 summary → false。
- T3. `MIN_SUMMARY_SEED_CHARS` 用于 TooShortToSummarize 判定。单测。

**阶段 B：tool-pair 原子性**
- T4. 用现有 `Message`/`ContentBlock` 结构确认 tool_call_id 关联方式（读 message.rs）。
- T5. 实现 `splits_tool_pair` + `safe_tail_cut`。单测：构造 [user, tool_call(id=A), tool_result(id=A), assistant]，cut 落在 tool_call 与 tool_result 之间 → safe_tail_cut 回退到不切断。
- T6. 属性测试：任意消息序列 + 任意 keep_from → `safe_tail_cut` 结果处 [cut..] 内无孤儿 tool_result。

**阶段 C：富接口 + AdvancedCompressor**
- T7. `maybe_compact_v2` 默认方法（桥接旧）。`cargo check -p fabric`。
- T8. `AdvancedCompressor::maybe_compact_v2`：TailKeep 用 safe_tail_cut；总结后跑 degenerate 检测，退化则 `failure=DegenerateSummary` 且 **messages 不变**。单测。
- T9. FullReplace 分支：整段总结为前缀 + 近尾；同样 degenerate 检测。单测。
- T10. 单测：LLM sampler 失败 → `failure=SamplerError` 且 messages 不变（fail-safe）。

**阶段 D：harness 集成（flag 后）**
- T11. `linear/mod.rs` 循环：flag 开走 v2。degenerate/failure → 保留上下文、发 `CompactionOutcome` 事件、记指标。flag 关走旧 `maybe_compact`（回归等价）。
- T12. `evicted` 非空 → 调 promote 回调（本期回调可为 no-op + 计数，Mnemosyne 实接后续）。

**阶段 E：conscious 策略选择（可选，flag 后）**
- T13. 从 `LatestConsciousContextPort` 读仲裁信号选 strategy；无信号默认 TailKeep。单测（mock port）。

**阶段 F：收尾**
- T14. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭**：harness 走旧 `maybe_compact`，`AdvancedCompressor` 旧行为不变。
- **v2 默认方法**：未实现 v2 的 compactor 自动桥接到旧行为。
- **fail-safe 优先**：压缩失败宁可保留超长上下文也不丢内容（与 tool streaming 的 fail-closed 相反——这里数据完整性 > 长度约束）。
- **evicted 促进**：本期只产出 evicted + 计数；Mnemosyne 实际 promotion 是 G7/独立工作。

## 8. 测试计划（映射研究文档 ../12 §6 验收方向）

| 验收方向 | 测试 |
|---|---|
| 接近上限自动触发（无需用户干预） | 复用现有 `loop_compacts_when_over_budget`（linear/mod.rs:497） |
| 不产生退化 summary 静默丢全部上下文 | T8, T10（degenerate/failure → messages 不变） |
| tool_call+tool_result 永不被切分 | T5, T6 |
| 失败留在可恢复状态 | T10, T11 |
| 压缩决策记为 canonical 事件 | T11（CompactionOutcome 事件） |
| memory 引用/authority 跨压缩存活 | T5（tail 保留）+ evicted 只含可丢弃段 |

## 9. 可观测性

- 事件：复用 `TurnEventV1::CompactionTriggered`；新增 `CompactionOutcome`（strategy、tokens_before/after、evicted 数、degenerate?）。
- 指标：`compaction_degenerate_total`、`compaction_sampler_error_total`、`compaction_evicted_messages_total`。

## 10. 许可证

重新实现 guardrail 与 tool-pair 语义，不复制 Grok `xai-grok-compaction` 源码。degenerate 阈值为独立选值。无 NOTICE 变更。
