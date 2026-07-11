# ReAct Loop and ContentBlock Protocol

> Migrated from `docs/design/core/cognitive-engine.md` (ReAct loop and ContentBlock sections only) — code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `executive`
**Code location:** `cognit/src/harness/linear/step.rs`
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| ReAct loop | Implemented | `cognit/src/harness/linear/step.rs` | Core tool loop works end-to-end |
| ContentBlock types | Implemented | `fabric/src/types/message.rs` | Text, ToolUse, ToolResult, Image |
| Context compaction | Implemented | `mnemosyne/src/impl/compressor/` | AdvancedCompressor with token-budget tail protection, iterative summary, tool output pre-pruning |
| Streaming | Implemented | `cognit/src/impl/inference/provider.rs` | `LlmStream` trait with SSE chunk streaming |
| LoopDetector integration | Implemented | `corpus/src/security/security/loop_detector.rs` | Wired to engine via `pre_check()`/`post_check()` |

---

## 1. ReAct Reasoning Loop

**ReAct loop** — Uses Anthropic SDK's Think-Act-Observe tool loop pattern to drive agent reasoning and decision-making.
- Code location: `cognit/src/harness/linear/step.rs`

```
+-------------------------------------------------------------+
|                    Cognitive Engine                            |
|                                                               |
|  +---------------------------------------------------------+ |
|  |              Reasoning Loop (Think-Act-Observe)          | |
|  |                                                         | |
|  |  +----------+   +----------+   +----------+            | |
|  |  | THINK    |-->| PLAN     |-->| ACT      |            | |
|  |  |          |   |          |   |          |            | |
|  |  | Analyze  |   | Make     |   | Execute  |            | |
|  |  | current  |   | plan,    |   | actions, |            | |
|  |  | state &  |   | break    |   | call     |            | |
|  |  | goals    |   | steps,   |   | tools,   |            | |
|  |  |          |   | select   |   | observe  |            | |
|  |  |          |   | strategy |   | results  |            | |
|  |  +----------+   +----------+   +----------+            | |
|  |       ^                                  |              | |
|  |       |                                  |              | |
|  |       +----------------------------------+              | |
|  |                   Feedback loop                          | |
|  +---------------------------------------------------------+ |
+-------------------------------------------------------------+
```

---

## 2. Content-Block Message Protocol

**ContentBlock** — Unified content-block message format (inspired by Anthropic SDK), used for all agent communication.
- Code location: `fabric/src/types/message.rs`
- Contains Text, ToolUse, ToolResult, Image four variants
- Aligned with LLM API native format, reducing conversion overhead; `ToolResult`'s `is_error` field implements structured tool errors

```rust
enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    Image { source: ImageSource },
}

struct Message {
    role: Role,  // System / User / Assistant
    content: Vec<ContentBlock>,
}
```

---

## 3. LoopDetector Integration

The security model's `LoopDetector` (`corpus/src/security/security/loop_detector.rs`) provides pre-check and post-check hooks integrated into the ReAct loop.

**Integration points:**
1. Pre-check before tool call: risk classification + loop detection
2. Engine holds `loop_detector: LoopDetector` field alongside `policy_engine`
3. Reuses security model's RiskClassifier four-level risk classification thresholds
4. Follows fail-closed semantics: LoopDetector errors block the call and log warnings

**Risk classification thresholds:**

| Risk Level | same_call_threshold | fail_streak_threshold |
|------------|---------------------|----------------------|
| ReadOnly | 5 | 7 |
| FileModification | 3 | 5 |
| SystemChange | 2 | 3 |
| Destructive | 2 | 2 |

---

## 4. Context Compression

Implemented in `mnemosyne/src/impl/compressor/`:

```rust
async fn compact(&mut self) {
    // Compress with cheap model (e.g., Qwen3-8B local)
    let summary = self.summarizer
        .summarize(&self.recent_messages, SummarizeModel::Local)
        .await?;

    // Old messages moved to Recall Memory (SQLite)
    self.recall_db.store(&self.evicted_messages).await?;

    // Key fact extraction -> Archival Memory (vector DB)
    let facts = self.extract_key_facts(&self.evicted_messages).await;
    for fact in facts {
        self.archival_db.insert(fact).await?;
    }

    // Context replaced with summary
    self.messages = vec![ContentBlock::Text(summary)];
}
```

**Compression trigger:**
- Token count exceeds threshold (default 70% of context window)
- Can be precisely tracked by `ContextBudget` module (see [memory-system.md](../mnemosyne/memory-system.md) section 2.2)

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| ReAct loop | `cognit/src/harness/linear/step.rs` | `Engine`, `TurnConfig`, `TurnResult` |
| ContentBlock protocol | `fabric/src/types/message.rs` | `ContentBlock` (Text/ToolUse/ToolResult/Image), `Message` |
| LoopDetector integration | `corpus/src/security/security/loop_detector.rs` | `LoopDetector`, `pre_check()`, `post_check()` |
| Compressor | `mnemosyne/src/impl/compressor/` | `AdvancedCompressor`, `TailProtectionConfig`, `SummaryTemplate` |
