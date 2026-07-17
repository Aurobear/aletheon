# Context Compaction Engine

## 1. Why This Matters for Aletheon Right Now

Aletheon's current branch has active work on context management:

- `conscious_context_slot.rs` (new, unstaged)
- Recent commits: "measure bounded field dynamics", "define conscious arbitration contracts", "support governed capability batch order"

These suggest you're building a system that decides what context enters the model's attention window. Grok's compaction engine is a production-hardened reference for exactly this problem — it addresses the same question ("when the context window is full, what do we keep and what do we drop?") with multiple strategies, transport-agnostic design, and proven failure modes.

## 2. Grok's Compaction Engine Architecture

### 2.1 Three Compaction Styles

The crate `xai-grok-compaction` (at `crates/common/xai-grok-compaction/`) defines three compaction approaches, each in its own module:

| Style | Module | Trigger | Method |
|---|---|---|---|
| **Code Compaction** | `code_compaction/` | Context window nearly full | Full-replace: summarize whole session, restart with summary as prefix |
| **Intra Compaction** | `intra_compaction/` | Per-step, during a turn | Tail-keep: trim middle turns, keep head + recent tail |
| **Inter Compaction** | `inter_compaction/` | Between turns | Chunked: compact older turns into summary chunks |

Source: `/home/aurobear/Bear-ws/grok-build/crates/common/xai-grok-compaction/src/lib.rs:1-50`

### 2.2 Code Compaction (Full-Replace)

This is the most relevant style for Aletheon's conscious context. When the session's token usage hits a threshold (`DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT`), the engine:

1. **Samples a summary**: Calls an LLM with a specialized self-summarization prompt (`SELF_SUMMARIZATION_PROMPT`) that asks the model to summarize the session's work, decisions, and state.
2. **Validates the summary**: Checks for degenerate output (too short, empty, repetition) via `is_degenerate_summary()`.
3. **Classifies failures**: `FailureKind` covers HTTP status errors, stream event errors, and content-length errors via `classify_http_status()` and `classify_stream_event_error()`.
4. **Assembles compacted history**: `assemble_compacted_history()` reconstructs the turn list with the summary as prefix + recent turns preserved.
5. **Produces outcome**: `FullReplaceAttemptOutcome` records success/failure/drop with metrics.

Key design decisions visible in the source:

- **File references are preserved**: `CompactionFileRef` tracks which files are mentioned, so the model retains awareness of the working set even after older turns are summarized away.
- **Minimum summary seed**: `MIN_SUMMARY_SEED_CHARS` — if the session is too short to summarize meaningfully, compaction is skipped rather than producing a garbage summary.
- **User query wrapping**: `wrap_user_query()` ensures the original user intent is preserved in the compacted context.

Source: `/home/aurobear/Bear-ws/grok-build/crates/common/xai-grok-compaction/src/code_compaction/`

### 2.3 Intra Compaction (Tail-Keep, Per-Step)

Used by the Grok chat product. Instead of full-replace, it keeps the head of the conversation and a configurable tail, trimming the middle. Key component: `select_turns_to_compact()` which produces a `SplitPlan` — which turns stay, which are compacted.

Safety rule: tool-call pairs (tool_use + tool_result) are never split — they're either both kept or both compacted. This prevents the model from seeing unpaired tool calls.

Source: `/home/aurobear/Bear-ws/grok-build/crates/common/xai-grok-compaction/src/select.rs`, `intra_compaction/`

### 2.4 Inter Compaction (Chunked, Between-Turn)

Runs when the model is idle between turns. Compacts older history into summary chunks that replace the original turns. The `history/` module provides filtering, validation, and user-query preservation across chunk boundaries.

Source: `/home/aurobear/Bear-ws/grok-build/crates/common/xai-grok-compaction/src/inter_compaction/`, `history/`

### 2.5 Transport-Agnostic Trait Seams

The compaction engine is decoupled from any specific conversation type or product host through trait boundaries:

```text
CompactionItem          — single turn + reconstruction
CompactionRole          — user / assistant / system
CompactionItemBuilder   — reconstructs a turn from compacted form
ItemTokenCounter        — trusted token counting (host-supplied)
CompactionSampler       — the LLM call that produces the summary
IntraCompactionObserver — metrics (host-supplied)
InterCompactionObserver — metrics (host-supplied)
CompactionStreamProc    — state commit for intra-compaction
```

Source: `/home/aurobear/Bear-ws/grok-build/crates/common/xai-grok-compaction/src/lib.rs:12-21`, `item.rs`, `token.rs`, `sampler.rs`

This is the key architectural insight: **the compaction policy, prompt, and selection logic is shared; only the host-specific transport, persistence, and metrics are injected**. Aletheon could adopt the same pattern, keeping Conscious-core as the policy owner while Cognit/Executive provide the host seams.

## 3. Relevance to Aletheon's Conscious Context

### 3.1 Mapping to Aletheon Concepts

| Grok Concept | Potential Aletheon Mapping |
|---|---|
| `CompactionItem` | A turn/segment in `ConsciousContextSlot` |
| `CompactionSampler` | Conscious-core's bounded field measurement |
| `ItemTokenCounter` | Already exists — Aletheon tracks tokens per turn |
| `FullReplaceSummary` | Conscious arbitration contract output |
| `SplitPlan` (which turns stay/drop) | Bounded field dynamics — which context enters the attention window |
| `CompactionFileRef` | Working-set awareness in Agora / memory projection |

### 3.2 Design Patterns Worth Adopting

**Pattern 1: Summary with Guardrails**

Grok's full-replace doesn't just summarize — it validates:
- Is the summary degenerate? → retry or abort compaction
- Did the LLM call fail? → classify failure, don't silently drop context
- Is the session too short to summarize? → skip, don't produce garbage

Aletheon's conscious arbitration contracts should include equivalent guardrails.

**Pattern 2: Tool-Pair Atomicity**

Intra compaction's rule that tool_use + tool_result must stay together is a correctness invariant. Aletheon's context slot trimming should preserve equivalent atomicity for CapabilityCall + CapabilityResult pairs.

**Pattern 3: Host-Independent Policy**

The compaction policy (when to compact, what strategy to use, thresholds) is separated from the transport (how to call the LLM, how to persist state). This is exactly the boundary between Conscious-core (policy/arbitration) and Cognit (execution/transport).

**Pattern 4: Observer Pattern for Metrics**

`IntraCompactionObserver` and `InterCompactionObserver` are host-injected traits that receive structured events (attempt started, summary generated, compaction applied, failure). Aletheon's event spine already supports this pattern.

### 3.3 What NOT to Adopt

- **The specific prompt templates**: Grok's self-summarization prompt is tuned for coding agents. Aletheon's conscious context has broader domain scope (governance, memory, multi-agent).
- **The full-replace strategy as the only option**: Aletheon should support multiple arbitration strategies (summarize, trim, delegate-to-subagent, promote-to-memory, request-user-guidance).
- **Grok's token counting**: Aletheon already has its own token accounting.

## 4. Suggested Integration with Conscious Context Slot

The new `conscious_context_slot.rs` (unstaged) is the natural place for this. A candidate architecture:

```text
ConsciousContextSlot
  |
  +-- measure(): ConsciousBoundedFieldDynamics
  |     Uses token accounting + configured thresholds to detect
  |     when compaction is needed.
  |
  +-- arbitrate(): ConsciousArbitrationContract
  |     Selects strategy: full-replace / tail-keep / chunked /
  |     promote-to-memory / delegate / request-guidance.
  |     This is where the three Grok styles become explicit options.
  |
  +-- execute(): ArbitrationOutcome
        Invokes the chosen strategy. Full-replace calls the model
        for summarization. Tail-keep trims middle turns. Chunked
        processes older segments. Each produces a bounded result.
```

## 5. Key Design Decisions to Resolve

Before implementing, Aletheon needs to decide:

1. **Single strategy or multi-strategy?** Grok supports three. Aletheon's first version could start with one (full-replace, since it's the simplest to reason about) and add tail-keep/chunked later.

2. **Model or rule-based summarization?** Grok uses an LLM for summarization. This is flexible but has cost, latency, and failure modes. Aletheon could start with rule-based trimming (keep last N turns, drop middle) and add LLM summarization as a conscious arbitration option.

3. **When to compact — threshold or proactive?** Grok triggers on threshold (context window % full). Aletheon could also trigger proactively when the conscious field measurement detects diminishing return from additional context.

4. **What crosses the compaction boundary?** Grok preserves file references. Aletheon needs to preserve memory citations, authority grants, and agent relationships across compaction boundaries.

5. **Compaction events in the event spine?** Grok fires `PreCompact`/`PostCompact` hooks. Aletheon's conscious arbitration should produce canonical events for audit and recovery.

## 6. Acceptance Direction

1. Session approaching context limit triggers arbitration automatically (not requiring user intervention).
2. Compaction never produces a degenerate/empty summary that silently drops all context.
3. CapabilityCall + CapabilityResult pairs are never split by tail-keep trimming.
4. Memory citations and authority grants survive compaction.
5. Compaction decisions (what was kept, what was dropped, why) are recorded as canonical events.
6. Failed compaction (LLM error, token miscount) leaves the session in a recoverable state — never silently corrupts.

## 7. Relationship to Other G-documents

- **G5 (Lifecycle Extensions)**: Compaction should expose pre/post hooks following the contributor model.
- **G6 (Subagent Settlement)**: Child Agent context should compact independently of parent; cross-agent memory promotion is a separate operation.
- **G9 (Memory Search)**: Compacted context that is "dropped" may be a candidate for Mnemosyne promotion rather than deletion.
- **G4 (Checkpoint/Rewind)**: Compaction boundaries are natural checkpoint points. Rewinding past a compaction should restore the pre-compaction state.
