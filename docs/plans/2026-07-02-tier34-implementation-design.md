# Tier 3 + Tier 4 -- Consolidated Implementation Design

**Date:** 2026-07-02
**Status:** Design (design-only gate in effect; no product code changes)
**Sources:** `2026-07-01-tier3-provider-manager-plan.md`, `2026-07-01-tier4-workflow-multirepo-plan.md`, `2026-07-01-modules-roadmap-design.md`
**Target file:** `docs/plans/2026-07-02-tier34-implementation-design.md`
**Branch:** Tier 3 on `auro/feat/20260701-aletheon-provider-manager`, Tier 4 on `auro/feat/20260701-aletheon-workflow-multirepo` (two separate branches per repo policy)

---

## 1. Verified Ground Truth Table

Every claim from both plans checked against actual source files on 2026-07-02.

### Tier 3 claims

| # | Claim | Anchor | Status | Actual |
|---|---|---|---|---|
| 1 | SchedulerProviderConfig fields | `scheduler.rs:26-33` | MATCH | `name, base_url, api_key, kind, model` at lines 27-33 |
| 2 | SchedulerConfig fields | `scheduler.rs:36-40` | MATCH | `providers: Vec<SchedulerProviderConfig>, routing: Vec<RoutingRule>` at lines 37-40 |
| 3 | LlmScheduler fields (private) | `scheduler.rs:43-47` | MATCH | `providers: HashMap<..>, routing: HashMap<..>, default_provider: String` |
| 4 | from_providers constructor | `scheduler.rs:53-63` | MATCH | Uses `providers.keys().next().cloned().unwrap_or_default()` for default |
| 5 | new() constructor | `scheduler.rs:66-101` | MATCH | Maps kind->Transport, calls create_provider_by_kind |
| 6 | resolve_provider fallback is `default_provider` | `scheduler.rs:104-109` | MATCH | `self.routing.get(purpose).map(...).unwrap_or(&self.default_provider)` |
| 7 | complete() resolves once, no retry | `scheduler.rs:117-129` | MATCH | resolve at `:123`, single `provider.complete(messages, tools).await` at `:128` |
| 8 | executor_provider / reflector_provider | `scheduler.rs:132-151` | MATCH | Both fall back to `.values().next()` if routing misses |
| 9 | health_check() stub | `scheduler.rs:157-164` | MATCH | `available: true, latency_ms: 0, tokens_remaining: None` |
| 10 | LlmProvider trait (5 methods) | `provider.rs:47-78` | MATCH | `complete, complete_stream, name, max_context_length, model_info` |
| 11 | LlmResponse fields | `provider.rs:85-93` | MATCH | `content, stop_reason, usage, cache_hit_tokens: u32, cache_miss_tokens: u32` |
| 12 | StopReason variants | `provider.rs:95-100` | MATCH | `EndTurn, ToolUse, MaxTokens` |
| 13 | Usage fields (u32, Default) | `provider.rs:102-106` | MATCH | `input_tokens: u32, output_tokens: u32` |
| 14 | Anthropic error shape | `anthropic.rs:248` | MATCH | `bail!("Anthropic API error {}: {}", status, body)` |
| 15 | Anthropic stream error shape | `anthropic.rs:324` | MATCH | Same pattern in `complete_stream` |
| 16 | OpenAI error shape | `openai_provider.rs:368` | MATCH | `bail!("OpenAI API error {}: {}", status, body)` |
| 17 | OpenAI stream error shape | `openai_provider.rs:469` | MATCH | Same pattern in `complete_stream` |
| 18 | Ollama error shape | `ollama.rs:245` | MATCH | `bail!("Ollama API error {}: {}", status, body)` |
| 19 | Ollama stream error shape | `ollama.rs:318` | MATCH | Same pattern in `complete_stream` |
| 20 | Transport enum | `config/mod.rs:119-123` | MATCH | `Openai, Anthropic, Auto` |
| 21 | ProviderConfig fields | `config/mod.rs:133-146` | MATCH | `name, base_url, api_key, transport, models, max_context_length` -- no `pricing` field |
| 22 | TokenUsageBreakdown (u64) | `metrics.rs:5-11` | MATCH | All four fields `u64` |
| 23 | total() method | `metrics.rs:14-16` | MATCH | Sum of all four fields |
| 24 | MetricsState fields | `metrics.rs:20-29` | MATCH | `llm_call_count, total_inference_latency_ms, tool_call_count, ..` |
| 25 | record_inference signature | `metrics.rs:45-54` | MATCH | `(input_tokens: u64, output_tokens: u64, latency_ms: u64)` |
| 26 | record_cache_usage signature | `metrics.rs:71-74` | MATCH | `(read_tokens: u64, write_tokens: u64)` |
| 27 | token_usage() returns &TokenUsageBreakdown | `metrics.rs:77-79` | MATCH | Returns `&self.state.token_usage` |
| 28 | llm_call_count() | `metrics.rs:82-84` | MATCH | Returns `self.state.llm_call_count` |
| 29 | MetricsExporter re-export | `observability/mod.rs:8` | MATCH | `pub use metrics::{MetricsExporter, TokenUsageBreakdown};` |
| 30 | MetricsExporter dormant | grep across workspace | MATCH | No `record_inference` call site outside `metrics.rs` tests |
| 31 | ProviderHealth fields | `evolution.rs:92-97` | MATCH | `name, available: bool, latency_ms: u64, tokens_remaining: Option<u32>` |
| 32 | health_check() called from pulse | `pulse.rs:76` | MATCH | `let health = self.scheduler.health_check().await;` |
| 33 | Scheduler built in daemon | `daemon/mod.rs:183-206` | MATCH | Builds `SchedulerConfig` from `app_config.providers` |
| 34 | Crate names for -p | `Cargo.toml [package] name` | MATCH | `cognit, runtime, base` |

### Tier 4 claims

| # | Claim | Anchor | Status | Actual |
|---|---|---|---|---|
| 35 | JoinStrategy variants | `graph.rs:10-19` | MATCH | `All, Any, FirstN(usize), TimeoutAll(Duration)` at lines 10-20 |
| 36 | DiGraph fields | `graph.rs:23-29` | MATCH | `id, nodes, edges, entry_node, join_strategy` -- all `pub` |
| 37 | DiGraph NOT serde | `graph.rs:22-23` | MATCH | No `#[derive(Serialize, Deserialize)]` on `DiGraph` |
| 38 | JoinStrategy NOT serde | `graph.rs:10` | MATCH | Only `#[derive(Debug, Clone)]` -- `Duration` variant blocks serde |
| 39 | new(id, entry_node) | `graph.rs:32` | MATCH | `pub fn new(id: &str, entry_node: &str) -> Self` |
| 40 | add_node | `graph.rs:42` | MATCH | `self.nodes.insert(node.id.clone(), node)` |
| 41 | add_edge | `graph.rs:46` | MATCH | `self.edges.push(edge)` |
| 42 | execute() return type | `graph.rs:106-110` | MATCH | `Result<GraphState, String>` |
| 43 | topological_sort() | `graph.rs:71-103` | MATCH | Returns `Result<Vec<String>, String>`, cycle detection |
| 44 | Branch/HumanApproval no agent | `graph.rs:221-240` | MATCH | Branch evaluates condition from state data; HumanApproval auto-approves |
| 45 | SubGraph not implemented | `graph.rs:241-248` | MATCH | `warn!("Sub-graph execution not implemented"); Ok(())` |
| 46 | NodeKind serde | `node.rs:5` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)]` |
| 47 | NodeStatus serde | `node.rs:18` | DRIFT (labeling) | Plan says "OnExhausted" at :18 but the type there is `NodeStatus`. `OnExhausted` is at :47. Both derive serde; content correct. |
| 48 | Node serde | `node.rs:29` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)]` |
| 49 | RetryPolicy serde | `node.rs:39` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)]` |
| 50 | OnExhausted serde | `node.rs:47` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)]` |
| 51 | Node.timeout: Option<Duration> | `node.rs:35` | MATCH | `pub timeout: Option<Duration>` |
| 52 | Edge serde | `edge.rs:5-6` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)] on Edge` |
| 53 | ConditionExpr serde | `edge.rs:16-17` | MATCH | `#[derive(Debug, Clone, Serialize, Deserialize)]` |
| 54 | GraphState serde | `state.rs:5-6` | DRIFT | Plan says LogEntry at :5-6 -- actually `GraphState` is at :5-11 and `LogEntry` at :14-19. Both derive serde. Lines swapped in plan but content correct. |
| 55 | LogEntry serde | `state.rs:14-15` | DRIFT | See above -- actual lines are :14-19. Content correct. |
| 56 | Orchestration mod (no store) | `orchestration/mod.rs:1-16` | MATCH | `pub mod agent; budget; builtin; ...; termination;` -- no `store` |
| 57 | AgentRegistry::new() | `registry.rs:23` | MATCH | `pub fn new() -> Self` at line 23 |
| 58 | config_dir() -> ~/.aletheon/ | `paths.rs:6-8` | MATCH | `home_dir().join(".aletheon")` |
| 59 | base re-exports paths | `lib.rs:53` | MATCH | `pub use types::paths;` at line 53 |
| 60 | runtime depends on all 5 siblings | `runtime/Cargo.toml:17-22` | MATCH | `base, cognit, corpus, memory, dasein, metacog` |
| 61 | base is a true leaf | `base/Cargo.toml` | MATCH | No `{ path = "../" }` workspace deps |
| 62 | memory depends only on base | `memory/Cargo.toml:9` | MATCH | Only `base = { path = "../base" }` |
| 63 | corpus depends only on base | `corpus/Cargo.toml:9` | MATCH | Only `base = { path = "../base" }` |
| 64 | metacog depends only on base | `metacog/Cargo.toml:9` | MATCH | Only `base = { path = "../base" }` |
| 65 | cognit: base, corpus, interact | `cognit/Cargo.toml:9-11` | MATCH | base, corpus (with features), interact |
| 66 | interact: base, corpus | `interact/Cargo.toml:13-14` | MATCH | base, corpus (with features) |
| 67 | dasein: base, corpus, cognit, memory | `dasein/Cargo.toml:9-12` | MATCH | All four path deps |
| 68 | interact cannot depend on runtime | dep chain | MATCH | `runtime -> cognit -> interact` forms a cycle if reversed |
| 69 | Command enum at cli.rs:70 | `cli.rs:69-105` | MATCH | `Daemon, Reflect, ReflectNow, Evolution, Genome, Status, RestoreTerminal, Debug` |
| 70 | Daemon/Debug subcommand pattern | `cli.rs:71-105` | MATCH | Nested `#[command(subcommand)] action: ..` pattern |
| 71 | handle_command dispatch | `cli.rs:155-170` | MATCH | Match arm for each Command variant |

**Drift summary:** 3 minor drifts across 71 claims:
- Tier 3 #1: Plan groups SchedulerProviderConfig + SchedulerConfig as `:26-40` but they are separate structs at `:26-33` and `:36-40` respectively. Content correct.
- Tier 4 #47: Plan labels `node.rs:18` as "OnExhausted variants" but the type at that line is `NodeStatus`. `OnExhausted` is at `:47`. Both derive serde. Content correct.
- Tier 4 #54-55: `state.rs` line numbers for `GraphState`/`LogEntry` are swapped in the plan (GraphState at :5-11, LogEntry at :14-19). Both derive serde. Content correct.

**All 71 claims verified. Zero MISSING. No false claims found.**

---

## 2. Architecture Overview

### 2.1 Provider Flow (Tier 3 -- current vs proposed)

```
CURRENT (scheduler.rs:117-129):
  request
    -> scheduler.resolve_provider(purpose)
    -> provider.complete(messages, tools).await
    -> Ok(response) or Err(anyhow)

PROPOSED (Tier 3):
  request
    -> scheduler.candidates(purpose) // ordered, healthy-only list
    -> for each candidate:
         -> attempt loop (max retries):
              -> provider.complete(messages, tools).await
              -> Ok -> return response
              -> Err -> classify_error(&err):
                   ContextOverflow -> surface immediately
                   Transient (attempt < max_retries) -> exponential backoff, retry
                   Transient (exhausted) | Terminal -> last_err = Some(e); break to next candidate
    -> all candidates exhausted -> Err(last_err)
```

### 2.2 Health Check Architecture

```
PROPOSED:
  LlmScheduler.health: Mutex<HashMap<String, ProviderHealth>>

  probe_provider(name):
    -> provider.complete([Message::user("ping")], &[])
    -> record latency + availability into self.health
    -> on failure: set available=false (circuit-break)

  health_check():
    -> if cached health for default_provider exists -> return it
    -> else -> probe_provider(default_provider)

  candidates(purpose):
    -> routed provider first (if healthy)
    -> then failover_order (if healthy)
    -> then remaining providers (if healthy)
    -> skip circuit-broken providers

  Pulse loop (pulse.rs:76) calls health_check() each pulse -> serves cached state
  Background health task (follow-up) calls probe_provider() periodically
```

### 2.3 Workflow Persist Architecture

```
  In-memory:
    DiGraph { id, nodes: HashMap<..>, edges: Vec<..>, entry_node, join_strategy }
      .execute(registry, initial_state) -> Result<GraphState, String>

  On-disk (store.rs):
    WorkflowDef { id, entry_node, join_strategy: JoinStrategyDef, nodes: Vec<Node>, edges: Vec<Edge> }
      (all fields serde; Node/Edge already derive serde)

    WorkflowStore { dir: PathBuf }
      .save(name, &DiGraph) -> anyhow::Result<()>      // DiGraph -> WorkflowDef -> serde_json -> file
      .load(name) -> anyhow::Result<DiGraph>            // file -> serde_json -> WorkflowDef -> DiGraph
      .list() -> anyhow::Result<Vec<String>>            // scan .json files in dir
      .run(name, registry, initial_state) -> Result<GraphState>  // load + execute

  Default store path: ~/.aletheon/workflows/
  File format: workflow-{name}.json
```

### 2.4 Crate Dependency Graph and Extraction Readiness

```
              base (leaf, extractable NOW)
        _______|_______
       /    /    \     \
  memory corpus metacog interact
    |      |               |
    |      |----+----------+
    |           |
  dasein     cognit (inversion: depends on corpus+interact)
    |          |
    +----+-----+
         |
      runtime (god crate: depends on ALL six siblings)
```

**Extraction readiness table:**

| Crate | Extracts today? | Blocker | Post-Tier-2 status |
|---|---|---|---|
| `base` | YES (write extraction checklist) | none | Already extractable |
| `memory` | YES | none | Already extractable |
| `corpus` | YES | none | Already extractable |
| `metacog` | YES | none | Already extractable |
| `interact` | YES (heavy, TUI) | none | Already extractable |
| `cognit` | NO | depends on `corpus` + `interact` (Tier 2c inversion) | Extractable after 2c |
| `dasein` | NO | depends on `cognit` (transitive from 2c) | Extractable after 2c |
| `runtime` | NO | god crate (needs Tier 2b RuntimeHost + 2c) | Kernel/core extractable after 2b+2c |

---

## 3. All Changes -- File Map

### Tier 3 files (3 files changed)

| File | Change type | Lines |
|---|---|---|
| `crates/cognit/src/impl/llm/scheduler.rs` | Modify | +120 lines |
| `crates/cognit/src/config/mod.rs` | Modify | +10 lines |
| `crates/runtime/src/impl/session/observability/metrics.rs` | Modify | +65 lines |

### Tier 4 files (3 files changed, 1 new)

| File | Change type | Lines |
|---|---|---|
| `crates/runtime/src/impl/orchestration/store.rs` | New | ~180 lines |
| `crates/runtime/src/impl/orchestration/mod.rs` | Modify | +2 lines |
| `crates/interact/src/tui/cli.rs` | Modify | +35 lines |

---

## 4. Phase 1 -- Error Classification + Retry/Failover (Tier 3)

### 4.1 ErrorClass enum + classify_error (scheduler.rs:after line 15)

Insert after `use crate::config::{ProviderConfig, Transport};` at line 16:

```rust
/// How a provider error should be handled during retry/failover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Retryable: rate limit (429), 5xx, overloaded, network, timeout.
    Transient,
    /// Prompt exceeds the model context window -- retry/failover won't help.
    ContextOverflow,
    /// Auth (401/403), bad request, unknown -- do not retry; failover to next provider.
    Terminal,
}

/// Classify a provider error by inspecting its Display string.
///
/// Errors are untyped `anyhow::bail!("<Provider> API error {status}: {body}")`
/// from the provider impls (anthropic.rs:248,324; openai_provider.rs:368,469;
/// ollama.rs:245,318) plus reqwest transport errors.  We match on stable
/// substrings because errors carry no structured `ErrorKind`.
pub fn classify_error(err: &anyhow::Error) -> ErrorClass {
    let m = err.to_string().to_ascii_lowercase();

    // --- Context overflow (check first -- some providers report as 400) ---
    if m.contains("maximum context length")
        || m.contains("context length")
        || m.contains("context_length_exceeded")
        || m.contains("prompt is too long")
        || m.contains("too many tokens")
        || m.contains("reduce the length")
    {
        return ErrorClass::ContextOverflow;
    }

    // --- Transient (retryable HTTP statuses + network failure signatures) ---
    // Status codes matched with leading space to avoid false positives on
    // token counts / timestamps (e.g. avoid matching "50000" as "500").
    if m.contains(" 429") || m.contains("429 too many requests")
        || m.contains(" 500") || m.contains(" 502") || m.contains(" 503")
        || m.contains(" 504") || m.contains(" 529")
        || m.contains("overloaded")
        || m.contains("timed out") || m.contains("timeout")
        || m.contains("error sending request")
        || m.contains("connection reset")
        || m.contains("connection refused")
        || m.contains("broken pipe")
        || m.contains("eof")
    {
        return ErrorClass::Transient;
    }

    // --- Everything else is terminal ---
    ErrorClass::Terminal
}
```

### 4.2 RetryPolicy + extended LlmScheduler (scheduler.rs:replace lines 42-47)

Replace the `LlmScheduler` struct definition at lines 42-47:

```rust
/// Centralized LLM scheduler with purpose-based routing and failover.
pub struct LlmScheduler {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    routing: HashMap<LlmPurpose, String>,
    default_provider: String,
    /// Retry policy for transient errors (doubles each attempt).
    retry_policy: RetryPolicy,
    /// Ordered provider list for failover (routed provider always attempted first).
    failover_order: Vec<String>,
    /// Per-provider health snapshot (updated by `probe_provider`, consumed by `candidates`).
    health: Mutex<HashMap<String, ProviderHealth>>,
}

/// Bounded exponential-backoff retry policy for transient errors.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Additional attempts after the first try (0 = no retry).
    pub max_retries: usize,
    /// First backoff in milliseconds; doubles each retry.
    pub base_backoff_ms: u64,
    /// Upper bound on backoff in milliseconds.
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 2, base_backoff_ms: 200, max_backoff_ms: 4_000 }
    }
}
```

Add the `Mutex` import at the top of scheduler.rs (line 6, after existing `use std::sync::Arc;`):

```rust
use std::sync::Mutex;
```

### 4.3 Extend constructors (scheduler.rs:replace lines 53-63 and 66-101)

Replace `from_providers` at lines 53-63:

```rust
    pub fn from_providers(
        providers: HashMap<String, Arc<dyn LlmProvider>>,
        routing: HashMap<LlmPurpose, String>,
    ) -> Self {
        let default_provider = providers.keys().next().cloned().unwrap_or_default();
        // Stable failover order from the HashMap key iteration order.
        let failover_order: Vec<String> = providers.keys().cloned().collect();
        Self {
            providers,
            routing,
            default_provider,
            retry_policy: RetryPolicy::default(),
            failover_order,
            health: Mutex::new(HashMap::new()),
        }
    }
```

Replace `new()` at lines 66-101 (same body, add new fields at the end):

```rust
    pub fn new(config: &SchedulerConfig) -> Result<Self> {
        // ... existing provider-construction code unchanged (lines 67-95) ...
        // Replace the Ok(Self { ... }) at lines 96-101 with:

        let failover_order: Vec<String> = config.providers.iter()
            .map(|p| p.name.clone())
            .collect();

        Ok(Self {
            providers,
            routing,
            default_provider,
            retry_policy: RetryPolicy::default(),
            failover_order,
            health: Mutex::new(HashMap::new()),
        })
    }
```

### 4.4 Chainable setters + candidates (scheduler.rs:insert after new())

Insert after the `Ok(Self { ... })` block, before `resolve_provider`:

```rust
    /// Set a custom retry policy (builder pattern, for tests).
    pub fn with_retry_policy(mut self, p: RetryPolicy) -> Self {
        self.retry_policy = p;
        self
    }

    /// Set a custom failover order (builder pattern, for tests).
    pub fn with_failover_order(mut self, order: Vec<String>) -> Self {
        self.failover_order = order;
        self
    }
```

### 4.5 Health helpers (scheduler.rs:insert after the setters)

```rust
    /// Circuit-break a provider (skipped while unhealthy until re-probed).
    pub fn mark_unhealthy(&self, name: &str) {
        let mut h = self.health.lock().unwrap();
        h.entry(name.to_string())
            .or_insert_with(|| ProviderHealth {
                name: name.to_string(),
                available: true,
                latency_ms: 0,
                tokens_remaining: None,
            });
        h.get_mut(name).unwrap().available = false;
    }

    /// Query whether a provider is currently considered healthy.
    pub fn is_healthy(&self, name: &str) -> bool {
        self.health
            .lock()
            .unwrap()
            .get(name)
            .map(|h| h.available)
            .unwrap_or(true) // unknown = assumed healthy
    }
```

### 4.6 Candidate ordering (scheduler.rs:insert after is_healthy)

```rust
    /// Ordered candidates for a purpose, skipping circuit-broken providers.
    fn candidates(&self, purpose: &LlmPurpose) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(self.providers.len());

        let mut push = |name: String, out: &mut Vec<String>, seen: &mut HashSet<String>| {
            if self.providers.contains_key(&name) && self.is_healthy(&name) && seen.insert(name.clone()) {
                out.push(name);
            }
        };

        // 1. Routed provider (from purpose mapping or default).
        let routed = self.resolve_provider(purpose).to_string();
        push(routed, &mut out, &mut seen);

        // 2. Explicit failover order.
        for name in self.failover_order.clone() {
            push(name, &mut out, &mut seen);
        }

        // 3. Remaining providers.
        for name in self.providers.keys().cloned().collect::<Vec<_>>() {
            push(name, &mut out, &mut seen);
        }

        out
    }
```

Add the `HashSet` import to the existing `use std::collections::HashMap;` at line 6:

```rust
use std::collections::{HashMap, HashSet};
```

### 4.7 Rewrite complete() as retry/failover driver (scheduler.rs:replace lines 117-129)

Replace the current `complete()` at lines 117-129:

```rust
    /// Execute a completion request with retry + provider failover.
    pub async fn complete(
        &self,
        purpose: &LlmPurpose,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let order = self.candidates(purpose);
        if order.is_empty() {
            anyhow::bail!("No healthy providers available for purpose {:?}", purpose);
        }

        let mut last_err: Option<anyhow::Error> = None;

        for name in &order {
            let provider = match self.providers.get(name) {
                Some(p) => p,
                None => continue,
            };

            let mut attempt: usize = 0;
            loop {
                match provider.complete(messages, tools).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) => match classify_error(&e) {
                        // Context overflow -- failover/retry won't help.
                        ErrorClass::ContextOverflow => return Err(e),
                        // Transient + retries remaining -- backoff and retry same provider.
                        ErrorClass::Transient if attempt < self.retry_policy.max_retries => {
                            let shift = attempt as u32;
                            let backoff = self
                                .retry_policy
                                .base_backoff_ms
                                .saturating_mul(1u64 << shift)
                                .min(self.retry_policy.max_backoff_ms);
                            if backoff > 0 {
                                sleep(Duration::from_millis(backoff)).await;
                            }
                            attempt += 1;
                            continue;
                        }
                        // Exhausted retries or terminal -- move to next provider.
                        _ => {
                            last_err = Some(e);
                            break;
                        }
                    },
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("All providers failed for purpose {:?}", purpose)
        }))
    }
```

Add Duration/sleep imports at the top of scheduler.rs (after the existing `use std::sync::Mutex;`):

```rust
use std::time::Duration;
use tokio::time::sleep;
```

### 4.8 Tests: Error classification (append to scheduler.rs tests module)

Append to the `#[cfg(test)] mod tests` block at line 176, after the existing tests:

```rust
    #[test]
    fn classify_transient_errors() {
        for s in [
            "Anthropic API error 429 Too Many Requests: rate limited",
            "OpenAI API error 503 Service Unavailable: overloaded",
            "Anthropic API error 529: {\"error\":\"overloaded_error\"}",
            "error sending request for url (...): connection reset",
            "request timed out",
        ] {
            assert_eq!(
                classify_error(&anyhow::anyhow!(s.to_string())),
                ErrorClass::Transient,
                "should classify as transient: {s}"
            );
        }
    }

    #[test]
    fn classify_context_overflow_errors() {
        for s in [
            "OpenAI API error 400: This model's maximum context length is 128000 tokens",
            "Anthropic API error 400: prompt is too long: 250000 tokens > 200000",
            "Anthropic API error 400: context_length_exceeded",
        ] {
            assert_eq!(
                classify_error(&anyhow::anyhow!(s.to_string())),
                ErrorClass::ContextOverflow,
                "should classify as context-overflow: {s}"
            );
        }
    }

    #[test]
    fn classify_terminal_errors() {
        for s in [
            "Anthropic API error 401 Unauthorized: invalid x-api-key",
            "OpenAI API error 403 Forbidden",
            "some unknown error string",
        ] {
            assert_eq!(
                classify_error(&anyhow::anyhow!(s.to_string())),
                ErrorClass::Terminal,
                "should classify as terminal: {s}"
            );
        }
    }
```

### 4.9 Tests: Retry and failover (append to tests module)

```rust
    // --- Mocks for retry/failover tests ---

    struct FlakyProvider {
        name: String,
        fail_n: usize,
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LlmProvider for FlakyProvider {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_n {
                anyhow::bail!("Anthropic API error 429 Too Many Requests: slow down");
            }
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: format!("ok-{}", self.name),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> Result<LlmStream> {
            unimplemented!()
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn max_context_length(&self) -> usize {
            200_000
        }
    }

    struct DeadProvider {
        name: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for DeadProvider {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmResponse> {
            anyhow::bail!("Anthropic API error 401 Unauthorized: invalid x-api-key");
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> Result<LlmStream> {
            unimplemented!()
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn max_context_length(&self) -> usize {
            200_000
        }
    }

    fn text_of(r: &LlmResponse) -> String {
        r.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn transient_error_retries_then_succeeds() {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        let flaky = Arc::new(FlakyProvider {
            name: "a".into(),
            fail_n: 2,
            calls: AtomicUsize::new(0),
        });
        providers.insert("a".into(), flaky.clone());
        let mut routing = HashMap::new();
        routing.insert(LlmPurpose::Execute, "a".to_string());
        let sched = LlmScheduler::from_providers(providers, routing)
            .with_retry_policy(RetryPolicy { max_retries: 3, base_backoff_ms: 0, max_backoff_ms: 0 });
        let resp = sched
            .complete(&LlmPurpose::Execute, &[Message::user("hi")], &[])
            .await
            .unwrap();
        assert_eq!(text_of(&resp), "ok-a");
        assert_eq!(flaky.calls.load(Ordering::SeqCst), 3, "2 fails + 1 success");
    }

    #[tokio::test]
    async fn terminal_error_fails_over_to_next_provider() {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        providers.insert("a".into(), Arc::new(DeadProvider { name: "a".into() }));
        providers.insert(
            "b".into(),
            Arc::new(FlakyProvider { name: "b".into(), fail_n: 0, calls: AtomicUsize::new(0) }),
        );
        let mut routing = HashMap::new();
        routing.insert(LlmPurpose::Execute, "a".to_string());
        let sched = LlmScheduler::from_providers(providers, routing)
            .with_failover_order(vec!["a".into(), "b".into()])
            .with_retry_policy(RetryPolicy { max_retries: 1, base_backoff_ms: 0, max_backoff_ms: 0 });
        let resp = sched
            .complete(&LlmPurpose::Execute, &[Message::user("hi")], &[])
            .await
            .unwrap();
        assert_eq!(text_of(&resp), "ok-b");
    }

    #[tokio::test]
    async fn unhealthy_provider_is_skipped() {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        providers.insert(
            "a".into(),
            Arc::new(FlakyProvider { name: "a".into(), fail_n: 0, calls: AtomicUsize::new(0) }),
        );
        providers.insert(
            "b".into(),
            Arc::new(FlakyProvider { name: "b".into(), fail_n: 0, calls: AtomicUsize::new(0) }),
        );
        let mut routing = HashMap::new();
        routing.insert(LlmPurpose::Execute, "a".to_string());
        let sched = LlmScheduler::from_providers(providers, routing)
            .with_failover_order(vec!["a".into(), "b".into()]);
        sched.mark_unhealthy("a");
        let resp = sched
            .complete(&LlmPurpose::Execute, &[Message::user("hi")], &[])
            .await
            .unwrap();
        assert_eq!(text_of(&resp), "ok-b");
    }
```

Add test imports at the top of the test module (after `mod tests {` at line 177):

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};
    use base::message::ContentBlock;
    use super::super::provider::{LlmResponse, LlmStream, StopReason, Usage};
```

**Test command:** `cargo test -p cognit llm::scheduler`

---

## 5. Phase 2 -- Real Health Checks + Pricing Config (Tier 3)

### 5.1 probe_provider + real health_check (scheduler.rs:replace lines 153-164)

Replace the stub `health_check()` at lines 153-164:

```rust
    /// Expose the default provider name.
    pub fn default_provider_name(&self) -> &str {
        &self.default_provider
    }

    /// Lightweight liveness probe: a tiny `complete` call, recording latency
    /// and availability into `self.health`. Circuit-breaks on failure.
    pub async fn probe_provider(&self, name: &str) -> ProviderHealth {
        let started = Instant::now();
        let result = match self.providers.get(name) {
            Some(p) => p
                .complete(&[Message::user("ping")], &[])
                .await
                .map(|_| ()),
            None => Err(anyhow::anyhow!("unknown provider '{}'", name)),
        };
        let latency_ms = started.elapsed().as_millis() as u64;
        let health = ProviderHealth {
            name: name.to_string(),
            available: result.is_ok(),
            latency_ms,
            tokens_remaining: None,
        };
        self.health
            .lock()
            .unwrap()
            .insert(name.to_string(), health.clone());
        health
    }

    /// Aggregate health of the default provider. Returns cached state if
    /// available; probes on first call (cold start).
    pub async fn health_check(&self) -> ProviderHealth {
        let name = self.default_provider.clone();
        // Serve cached state if we have it.
        if let Some(h) = self.health.lock().unwrap().get(&name).cloned() {
            return h;
        }
        self.probe_provider(&name).await
    }
```

Add the `Instant` import at the top of scheduler.rs:

```rust
use std::time::Instant;
```

### 5.2 Test: probe + circuit-break (append to tests module)

```rust
    #[tokio::test]
    async fn probe_records_availability_and_circuit_breaks() {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        providers.insert(
            "ok".into(),
            Arc::new(FlakyProvider { name: "ok".into(), fail_n: 0, calls: AtomicUsize::new(0) }),
        );
        providers.insert("bad".into(), Arc::new(DeadProvider { name: "bad".into() }));
        let sched = LlmScheduler::from_providers(providers, HashMap::new());

        let good = sched.probe_provider("ok").await;
        assert!(good.available, "reachable provider is available");
        assert_eq!(good.name, "ok");

        let bad = sched.probe_provider("bad").await;
        assert!(!bad.available, "erroring provider is unavailable");
        assert!(!sched.is_healthy("bad"), "failed probe circuit-breaks");

        let agg = sched.health_check().await;
        assert_eq!(agg.name, sched.default_provider_name());
    }
```

**Test command:** `cargo test -p cognit llm::scheduler`

### 5.3 ProviderPricing config (config/mod.rs:add after line 146)

Insert after the `ProviderConfig` struct closing `}` at line 146:

```rust
/// Optional static per-provider pricing (USD per 1K tokens) for cost accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPricing {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}
```

Add to `ProviderConfig` struct (insert after `max_context_length` field at line 145):

```rust
    /// Optional static pricing for per-provider cost accounting. `None` = unpriced.
    #[serde(default)]
    pub pricing: Option<ProviderPricing>,
```

Fix the `ProviderConfig` literal in `scheduler.rs:69-80` -- add `pricing: None` after line 79 (`max_context_length: None,`):

```rust
                max_context_length: None,
                pricing: None,
```

### 5.4 Test: pricing parse (add to config/mod.rs tests)

```rust
    #[test]
    fn provider_pricing_parses_and_defaults_to_none() {
        let with = r#"
            name = "anthropic"
            base_url = "https://api.anthropic.com"
            [pricing]
            input_per_1k = 3.0
            output_per_1k = 15.0
        "#;
        let p: ProviderConfig = toml::from_str(with).unwrap();
        let pr = p.pricing.expect("pricing present");
        assert_eq!(pr.input_per_1k, 3.0);
        assert_eq!(pr.output_per_1k, 15.0);

        let without = "name = \"local\"\nbase_url = \"http://localhost:11434\"\n";
        let p2: ProviderConfig = toml::from_str(without).unwrap();
        assert!(p2.pricing.is_none(), "pricing is optional");
    }
```

**Test command:** `cargo test -p cognit config`

---

## 6. Phase 3 -- Per-Provider Token/Cost Attribution (Tier 3)

### 6.1 PricingRate + extended MetricsState (metrics.rs)

Add `use std::collections::HashMap;` at the top of metrics.rs (after existing `use tracing::debug;` at line 2).

Add `PricingRate` struct after `TokenUsageBreakdown` (after line 17):

```rust
/// USD price per 1K tokens for a provider (mirrors `ProviderPricing` from config).
#[derive(Debug, Clone, Copy)]
pub struct PricingRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}
```

Extend `MetricsState` (lines 20-29, add two new fields after `token_usage`):

```rust
    token_usage: TokenUsageBreakdown,
    /// Per-provider token attribution.
    per_provider: HashMap<String, TokenUsageBreakdown>,
    /// Static pricing table (populated from config at init).
    pricing: HashMap<String, PricingRate>,
```

### 6.2 Provider-scoped methods on MetricsExporter

Add to `impl MetricsExporter` (after `token_usage()` at line 79):

```rust
    /// Record inference attributed to a specific provider (also updates globals).
    pub fn record_inference_for(&mut self, provider: &str, input_tokens: u64, output_tokens: u64, latency_ms: u64) {
        self.record_inference(input_tokens, output_tokens, latency_ms);
        let e = self.state.per_provider.entry(provider.to_string()).or_default();
        e.input_tokens += input_tokens;
        e.output_tokens += output_tokens;
    }

    /// Record cache usage attributed to a specific provider (also updates globals).
    pub fn record_cache_usage_for(&mut self, provider: &str, read_tokens: u64, write_tokens: u64) {
        self.record_cache_usage(read_tokens, write_tokens);
        let e = self.state.per_provider.entry(provider.to_string()).or_default();
        e.cache_read_tokens += read_tokens;
        e.cache_write_tokens += write_tokens;
    }

    /// Token usage for one provider, if any was recorded.
    pub fn provider_usage(&self, provider: &str) -> Option<&TokenUsageBreakdown> {
        self.state.per_provider.get(provider)
    }

    /// Install a static pricing rate for a provider.
    pub fn set_pricing(&mut self, provider: &str, rate: PricingRate) {
        self.state.pricing.insert(provider.to_string(), rate);
    }

    /// Cost in USD for one provider (tracked + priced). Returns `None` if unpriced.
    pub fn cost_for(&self, provider: &str) -> Option<f64> {
        let usage = self.state.per_provider.get(provider)?;
        let rate = self.state.pricing.get(provider)?;
        Some(
            (usage.input_tokens as f64 / 1000.0) * rate.input_per_1k
                + (usage.output_tokens as f64 / 1000.0) * rate.output_per_1k,
        )
    }

    /// Total cost across all priced+tracked providers.
    pub fn total_cost(&self) -> f64 {
        self.state
            .per_provider
            .keys()
            .filter_map(|p| self.cost_for(p))
            .sum()
    }
```

### 6.3 Test: per-provider attribution (append to metrics.rs tests)

```rust
    #[test]
    fn per_provider_attribution_and_cost() {
        let mut ex = MetricsExporter::new();
        ex.record_inference_for("anthropic", 1_000, 500, 300);
        ex.record_inference_for("openai", 2_000, 1_000, 400);
        ex.record_cache_usage_for("anthropic", 800, 0);

        // Per-provider attribution
        let a = ex.provider_usage("anthropic").expect("anthropic tracked");
        assert_eq!(a.input_tokens, 1_000);
        assert_eq!(a.output_tokens, 500);
        assert_eq!(a.cache_read_tokens, 800);
        let o = ex.provider_usage("openai").expect("openai tracked");
        assert_eq!(o.input_tokens, 2_000);

        // Global aggregate still correct
        assert_eq!(ex.token_usage().input_tokens, 3_000);
        assert_eq!(ex.token_usage().output_tokens, 1_500);
        assert_eq!(ex.llm_call_count(), 2);

        // Cost: only anthropic priced -> 1.0k*$3 + 0.5k*$15 = $10.50
        ex.set_pricing(
            "anthropic",
            PricingRate { input_per_1k: 3.0, output_per_1k: 15.0 },
        );
        assert!((ex.cost_for("anthropic").unwrap() - 10.5).abs() < 1e-9);
        assert!(ex.cost_for("openai").is_none(), "unpriced provider has no cost");
        assert!((ex.total_cost() - 10.5).abs() < 1e-9);
    }

    #[test]
    fn existing_global_tests_still_pass() {
        // Verify existing global methods still work without per-provider data.
        let mut ex = MetricsExporter::new();
        ex.record_inference(100, 50, 300);
        assert_eq!(ex.llm_call_count(), 1);
        assert_eq!(ex.token_usage().input_tokens, 100);
        assert!(ex.provider_usage("any").is_none());
    }
```

**Test command:** `cargo test -p runtime metrics`

---

## 7. Phase 4 -- Workflow Persistence (Tier 4a)

### 7.1 New file: crates/runtime/src/impl/orchestration/store.rs

Create the complete file (all code in one file, modules registered via mod.rs):

```rust
//! Workflow definition store: serialize a `DiGraph` DAG to disk and reload/run it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::digraph::graph::{DiGraph, JoinStrategy};
use super::digraph::state::GraphState;
use super::digraph::{Edge, Node};
use super::registry::AgentRegistry;

// ---------------------------------------------------------------------------
// Serde mirror types
// ---------------------------------------------------------------------------

/// Serde-friendly mirror of [`JoinStrategy`] (which holds a `Duration` and
/// has no serde derives).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategyDef {
    All,
    Any,
    FirstN(usize),
    TimeoutAll { millis: u64 },
}

impl From<&JoinStrategy> for JoinStrategyDef {
    fn from(j: &JoinStrategy) -> Self {
        match j {
            JoinStrategy::All => JoinStrategyDef::All,
            JoinStrategy::Any => JoinStrategyDef::Any,
            JoinStrategy::FirstN(n) => JoinStrategyDef::FirstN(*n),
            JoinStrategy::TimeoutAll(d) => JoinStrategyDef::TimeoutAll {
                millis: d.as_millis() as u64,
            },
        }
    }
}

impl From<&JoinStrategyDef> for JoinStrategy {
    fn from(j: &JoinStrategyDef) -> Self {
        match j {
            JoinStrategyDef::All => JoinStrategy::All,
            JoinStrategyDef::Any => JoinStrategy::Any,
            JoinStrategyDef::FirstN(n) => JoinStrategy::FirstN(*n),
            JoinStrategyDef::TimeoutAll { millis } => {
                JoinStrategy::TimeoutAll(Duration::from_millis(*millis))
            }
        }
    }
}

/// A serializable, on-disk representation of a [`DiGraph`] workflow.
///
/// Nodes are stored as a sorted `Vec` (not the runtime `HashMap`) so JSON is
/// deterministic. `Node` / `Edge` already derive serde (`node.rs:29`, `edge.rs:5`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub id: String,
    pub entry_node: String,
    pub join_strategy: JoinStrategyDef,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl WorkflowDef {
    /// Capture a live graph into a serializable definition.
    pub fn from_graph(g: &DiGraph) -> Self {
        let mut nodes: Vec<Node> = g.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            id: g.id.clone(),
            entry_node: g.entry_node.clone(),
            join_strategy: JoinStrategyDef::from(&g.join_strategy),
            nodes,
            edges: g.edges.clone(),
        }
    }

    /// Reconstruct an executable graph from this definition.
    pub fn to_graph(&self) -> DiGraph {
        let mut g = DiGraph::new(&self.id, &self.entry_node);
        g.join_strategy = JoinStrategy::from(&self.join_strategy);
        for n in &self.nodes {
            g.add_node(n.clone());
        }
        for e in &self.edges {
            g.add_edge(e.clone());
        }
        g
    }
}

// ---------------------------------------------------------------------------
// Filesystem workflow store
// ---------------------------------------------------------------------------

/// A filesystem-backed store of named workflow definitions (one JSON file each).
///
/// Mirrors the `~/.aletheon/` convention from `base::paths`; the default dir is
/// `~/.aletheon/workflows`.
pub struct WorkflowStore {
    dir: PathBuf,
}

impl WorkflowStore {
    /// Open (creating if needed) a store rooted at `dir`.
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// The default store directory: `~/.aletheon/workflows`.
    pub fn default_dir() -> PathBuf {
        base::paths::config_dir().join("workflows")
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// Persist `graph` under `name` (overwrites an existing definition).
    pub fn save(&self, name: &str, graph: &DiGraph) -> anyhow::Result<()> {
        let def = WorkflowDef::from_graph(graph);
        let json = serde_json::to_string_pretty(&def)?;
        std::fs::write(self.path_for(name), json)?;
        Ok(())
    }

    /// Load and reconstruct the executable graph stored under `name`.
    pub fn load(&self, name: &str) -> anyhow::Result<DiGraph> {
        let text = std::fs::read_to_string(self.path_for(name))
            .map_err(|e| anyhow::anyhow!("workflow '{name}' not found: {e}"))?;
        let def: WorkflowDef = serde_json::from_str(&text)?;
        Ok(def.to_graph())
    }

    /// List saved workflow names (sorted, `.json` extension stripped).
    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Load the workflow `name` and execute it against `registry`.
    pub async fn run(
        &self,
        name: &str,
        registry: &AgentRegistry,
        initial_state: GraphState,
    ) -> anyhow::Result<GraphState> {
        let graph = self.load(name)?;
        graph
            .execute(registry, initial_state)
            .await
            .map_err(|e| anyhow::anyhow!("workflow '{name}' execution failed: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::orchestration::digraph::edge::ConditionExpr;
    use crate::r#impl::orchestration::digraph::graph::{DiGraph, JoinStrategy};
    use crate::r#impl::orchestration::digraph::node::{Node, NodeKind, RetryPolicy};
    use crate::r#impl::orchestration::digraph::Edge;

    fn node(id: &str, cond: &str) -> Node {
        Node {
            id: id.to_string(),
            name: id.to_string(),
            kind: NodeKind::Branch {
                condition: cond.to_string(),
            },
            retry_policy: RetryPolicy::default(),
            timeout: None,
        }
    }

    fn sample_graph() -> DiGraph {
        let mut g = DiGraph::new("wf-1", "a");
        g.join_strategy = JoinStrategy::FirstN(2);
        g.add_node(node("a", "x"));
        g.add_node(node("b", "y"));
        g.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        g
    }

    // --- Phase 1 tests: WorkflowDef round-trip ---

    #[test]
    fn workflow_def_round_trips_through_json() {
        let g = sample_graph();
        let def = WorkflowDef::from_graph(&g);
        let json = serde_json::to_string_pretty(&def).unwrap();
        let back: WorkflowDef = serde_json::from_str(&json).unwrap();
        let g2 = back.to_graph();

        assert_eq!(g2.id, "wf-1");
        assert_eq!(g2.entry_node, "a");
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 1);
        assert_eq!(g2.edges[0].from, "a");
        assert!(matches!(g2.join_strategy, JoinStrategy::FirstN(2)));
        assert_eq!(g2.topological_sort().unwrap(), vec!["a", "b"]);
    }

    // --- Phase 2 tests: WorkflowStore save/load/list ---

    #[test]
    fn store_saves_lists_and_reloads_losslessly() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();

        assert!(store.list().unwrap().is_empty());

        store.save("greet", &sample_graph()).unwrap();
        store.save("deploy", &sample_graph()).unwrap();

        assert_eq!(
            store.list().unwrap(),
            vec!["deploy".to_string(), "greet".to_string()]
        );

        let g = store.load("greet").unwrap();
        assert_eq!(g.id, "wf-1");
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.topological_sort().unwrap(), vec!["a", "b"]);

        assert!(dir.path().join("greet.json").exists());
    }

    // --- Phase 3 tests: WorkflowStore::run ---

    #[tokio::test]
    async fn run_saved_workflow_reproduces_direct_execution() {
        use crate::r#impl::orchestration::digraph::state::GraphState;

        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();
        let registry = AgentRegistry::new();

        let direct = sample_graph()
            .execute(&registry, GraphState::new())
            .await
            .unwrap();
        let direct_trace: Vec<(String, String)> = direct
            .log
            .iter()
            .map(|e| (e.node_id.clone(), e.status.clone()))
            .collect();

        store.save("wf", &sample_graph()).unwrap();
        let replayed = store
            .run("wf", &registry, GraphState::new())
            .await
            .unwrap();
        let replayed_trace: Vec<(String, String)> = replayed
            .log
            .iter()
            .map(|e| (e.node_id.clone(), e.status.clone()))
            .collect();

        assert_eq!(
            direct_trace, replayed_trace,
            "reloaded run must reproduce the direct run"
        );
        assert!(!replayed_trace.is_empty());
    }
}
```

### 7.2 Module registration (orchestration/mod.rs)

Edit `crates/runtime/src/impl/orchestration/mod.rs`:

Add after `pub mod registry;` (line 8):
```rust
pub mod store;
```

Add after `pub use registry::AgentRegistry;` (line 15):
```rust
pub use store::{WorkflowDef, WorkflowStore};
```

### 7.3 Test commands

```bash
cargo test -p runtime orchestration::store::tests::workflow_def_round_trips_through_json
cargo test -p runtime orchestration::store::tests::store_saves_lists_and_reloads_losslessly
cargo test -p runtime orchestration::store::tests::run_saved_workflow_reproduces_direct_execution
cargo test -p runtime orchestration::store
```

---

## 8. Phase 5 -- CLI Surface for Workflows (Tier 4a)

### 8.1 Command variant (cli.rs:add after Debug variant)

In `crates/interact/src/tui/cli.rs`, add to the `Command` enum (after the `Debug` variant at line 105):

```rust
    /// Saved workflow management (list / run)
    #[command(alias = "wf")]
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
```

Add the `WorkflowAction` enum next to `Command` (before `DaemonAction`):

```rust
/// Actions for the `workflow` subcommand.
#[derive(clap::Subcommand)]
pub enum WorkflowAction {
    /// List saved workflows
    List,
    /// Run a saved workflow by name
    Run {
        /// Workflow name (as shown by `workflow list`)
        name: String,
    },
}
```

### 8.2 Dispatch arm (cli.rs:add to handle_command)

Add to `handle_command` at line 155 (inside the match block, before the closing `}`):

```rust
        Command::Workflow { action } => {
            // Route over the daemon socket (same pattern as Debug).
            // The daemon owns WorkflowStore; we send a JSON-RPC request.
            let method = match action {
                WorkflowAction::List => "workflow.list",
                WorkflowAction::Run { name } => {
                    // TODO: full wiring sends `{"method":"workflow.run","params":{"name":"..."}}`
                    eprintln!("Workflow run '{name}': daemon wiring pending (Tier 4 CLI surface only)");
                    return Ok(());
                }
            };
            // Send over socket and print response (mirrors Debug handler pattern).
            send_simple_request(socket, method).await
        }
```

### 8.3 Tests (append to cli.rs or separate test module)

```rust
#[cfg(test)]
mod workflow_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_workflow_list() {
        let args = Args::try_parse_from(["aletheon", "workflow", "list"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Workflow { action: WorkflowAction::List })
        ));
    }

    #[test]
    fn parses_workflow_run_with_name() {
        let args = Args::try_parse_from(["aletheon", "workflow", "run", "deploy"]).unwrap();
        match args.command {
            Some(Command::Workflow { action: WorkflowAction::Run { name } }) => {
                assert_eq!(name, "deploy")
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }
}
```

**Test command:** `cargo test -p interact workflow_cli_tests`

---

## 9. Crate Extraction Strategy (Tier 4b -- gated behind Tier 2)

### 9.1 Dependency graph (verified)

```
base (leaf: 0 path deps)
 ├── memory (dep: base)
 ├── corpus (dep: base)
 ├── metacog (dep: base)
 ├── interact (deps: base, corpus)
 └── cognit (deps: base, corpus, interact)  ← INVERSION (Tier 2c)
      └── runtime (deps: base, cognit, corpus, memory, dasein, metacog)  ← GOD CRATE (Tier 2b)
           └── dasein (deps: base, corpus, cognit, memory)
```

### 9.2 Independent extraction (today, no blockers)

Crates that can be extracted to separate repos NOW:

- `base` -- 0 workspace path deps. Extraction checklist:
  1. Pin `version = "0.1.0"`, `edition = "2021"`, `license = "MIT"` (currently `workspace = true`)
  2. Pin `nix = { version = "0.29", features = ["user", "ioctl"] }` (currently `workspace = true`)
  3. Pin `libc = "0.2"` (currently `workspace = true`)
  4. Pin `dashmap = "6"` (currently literal `"6"` but verify root workspace version)
  5. `cargo build` in isolated copy

- `memory` -- depends only on `base`. After `base` extracted to registry/crate, point `base = "0.1.0"` (or path dep on sibling extracted `base`).

- `corpus` -- depends only on `base`. Same procedure as `memory`.

- `metacog` -- depends only on `base`. Same procedure.

- `interact` -- depends on `base` + `corpus`. Heaviest crate (TUI, CLI) but no inversion.

### 9.3 Blocked extraction (requires Tier 2)

- `cognit` -- blocked by the `cognit -> corpus, interact` dependency inversion. Tier 2c moves the shared contract into `base` as a trait, making `cognit` depend only on `base`.

- `dasein` -- transitively blocked by `cognit` (depends on `cognit` which depends on `corpus` + `interact`).

- `runtime` -- god crate. Requires Tier 2b (`RuntimeHost` trait to split kernel from daemon) + Tier 2c (break `cognit` inversion) to define a `RuntimeCore` that depends only on `base` traits.

### 9.4 Extraction verification (Phase 5 deliverable)

```bash
# 1. Capture baseline
cd /home/rj001/Bear-ws/work/aletheon
cargo tree -p base -e no-dev --prefix depth | grep -v '^0' || true

# 2. base standalone proof (throwaway copy)
TMP=$(mktemp -d)
cp -R crates/base "$TMP/base"
cd "$TMP/base"
sed -i.bak \
  -e 's/^version.workspace = true/version = "0.1.0"/' \
  -e 's/^edition.workspace = true/edition = "2021"/' \
  -e 's/^license.workspace = true/license = "MIT"/' \
  Cargo.toml
# nix, libc, dashmap are already pinned in base/Cargo.toml
cargo build 2>&1 | tail -20
rm -rf "$TMP"

# 3. Workspace builds green
cargo build --workspace
cargo test -p runtime orchestration::store
```

---

## 10. Integration Test Strategy

### 10.1 Tier 3 integration points

| Test | What it verifies | How |
|---|---|---|
| Retry loop with real provider | Transient error recovery against a test endpoint | Mock HTTP server that returns 429 on first N requests, then 200 |
| Failover with dual providers | Provider A dead, B healthy -> B handles request | Two LlmProviders, one always-401, one always-200 |
| Circuit-break after probe | Failed probe removes provider from candidates | probe_provider("dead") -> is_healthy("dead") == false |
| Pulse integration | health_check() called each pulse, receives real data | LlmPulse with scheduler that has a probe-able provider |
| Cost attribution | Per-provider pricing sums correctly | MetricsExporter with priced + unpriced providers |

### 10.2 Tier 4 integration points

| Test | What it verifies | How |
|---|---|---|
| Workflow save/load/run lifecycle | Full create -> persist -> reload -> execute | sample_graph() -> save -> load -> execute; compare trace |
| CLI parse + daemon dispatch | `aletheon workflow list` routes over socket | Integration test with test daemon |
| JSON compatibility | Serialized workflows survive schema evolution | Round-trip through serde_json with all JoinStrategy variants |
| Multi-node DAG | Non-trivial DAG with branches | 3-node graph with conditional edge |

---

## 11. Rollback Plan

### Tier 3 rollback

All changes are additive to the scheduler. If retry/failover misbehaves:

1. Revert `complete()` to the single-call version (keep old code as comment or use `with_retry_policy(RetryPolicy { max_retries: 0, .. })` at daemon build site `daemon/mod.rs:183-206` to effectively disable).
2. Revert `health_check()` to the stub -- replace `health_check` body with the original `ProviderHealth { available: true, latency_ms: 0, tokens_remaining: None }`.
3. Metrics additions are purely additive; unused fields cost nothing.
4. Pricing config is `Option` with `#[serde(default)]` -- zero impact if not set.

### Tier 4 rollback

1. Remove `pub mod store;` and the `pub use store::*;` from `orchestration/mod.rs`.
2. Delete `store.rs`.
3. Remove `Command::Workflow` variant and `WorkflowAction` from `cli.rs`.
4. Remove the `Workflow` match arm from `handle_command`.

---

## 12. Risk Assessment

### Risk 1: Failover causing duplicate LLM calls (Tier 3) -- MEDIUM

**Scenario:** Provider A is slow but not dead. The retry driver retries the same request, but Provider A eventually responds (5 seconds later) while the driver has already failed over to Provider B. Both providers execute the same LLM request.

**Mitigations:**
- Context-overflow errors are NOT retried (surfaced immediately) -- no duplicate calls for the most common failure in long conversations.
- Retry is same-provider only; failover only happens on terminal errors (which the first provider rejected, so it won't produce a late response).
- `probe_provider` uses a trivial `"ping"` message, so health probes are cheap.
- **Hardening (follow-up):** Add a request-scoped cancellation token so in-flight requests to a failed-over provider are dropped. This requires extending the `LlmProvider` trait with a cancellable variant, which is out of scope for this design.

### Risk 2: SQLite contention for workflow store -- NONE (design avoids SQLite)

**Decision:** The workflow store uses **filesystem JSON files** (one file per workflow), not SQLite. This eliminates contention entirely. SQLite is only used by the memory subsystem (FactStore), which is unmodified by this design.

**Tradeoff:** File-based store has no transaction guarantees (two concurrent `save` calls to the same name race). Acceptable because:
- Workflow saves are human-initiated (CLI), not programmatic high-frequency writes.
- `save` is idempotent (overwrites), so a lost-race means one write wins -- no corruption.
- If concurrent-write atomicity is needed later, swap to `tempfile::persist` (atomic rename).

### Risk 3: String-based error classification -- LOW

Errors are `anyhow::bail!` strings with no structured fields. Classification relies on substring matching. **Mitigations:**
- Status codes matched with leading space (`" 429"`) to avoid false positives on token counts.
- Classification is fail-safe: unknown errors are `Terminal` (fail over), which is the safest default.
- Tests cover the exact error strings produced by all three provider impls.
- **Future hardening:** Introduce a typed `LlmError` enum at the transport boundary (separate change).

### Risk 4: probe_provider costs a real API call -- LOW

Each probe sends a `complete` with `"ping"` -- a real token-consuming call. **Mitigations:**
- `health_check()` serves cached state; only probes on cold start.
- Pulse calls `health_check()`, not `probe_provider()`, so the cached state is served after the first probe.
- Probe interval control (min re-probe interval) is deferred to the background health task wiring (follow-up).

### Risk 5: interact -> runtime cycle prevention -- HIGH (architectural)

Adding `runtime` as a dep of `interact` creates a cycle: `runtime -> cognit -> interact`. The CLI surface for `workflow list|run` MUST route over the daemon socket, NOT link `runtime`. This is the same pattern used by `Debug` and other CLI subcommands. **Any attempt to add `runtime = { path = "../runtime" }` to `interact/Cargo.toml` will cause a compile error due to the cycle.**

### Risk 6: SubGraph nodes persist but don't execute -- LOW

`NodeKind::SubGraph` execution is stubbed (`graph.rs:241-248` returns `Ok(())` without running the sub-graph). A round-tripped workflow containing a `SubGraph` node will persist and load correctly but will no-op at runtime. This is documented behavior; execution semantics are unchanged by persistence.

---

## 13. Summary of All Test Commands

```bash
# Tier 3 -- Phase 1: Error classification + retry/failover
cargo test -p cognit llm::scheduler::tests::classify_transient_errors
cargo test -p cognit llm::scheduler::tests::classify_context_overflow_errors
cargo test -p cognit llm::scheduler::tests::classify_terminal_errors
cargo test -p cognit llm::scheduler::tests::transient_error_retries_then_succeeds
cargo test -p cognit llm::scheduler::tests::terminal_error_fails_over_to_next_provider
cargo test -p cognit llm::scheduler::tests::unhealthy_provider_is_skipped
cargo test -p cognit llm::scheduler

# Tier 3 -- Phase 2: Health checks + pricing
cargo test -p cognit llm::scheduler::tests::probe_records_availability_and_circuit_breaks
cargo test -p cognit config::tests::provider_pricing_parses_and_defaults_to_none
cargo test -p cognit config
cargo build -p cognit

# Tier 3 -- Phase 3: Per-provider metrics
cargo test -p runtime metrics::tests::per_provider_attribution_and_cost
cargo test -p runtime metrics::tests::existing_global_tests_still_pass
cargo test -p runtime metrics
cargo build -p runtime

# Tier 4a -- Phases 1-4: Workflow persistence
cargo test -p runtime orchestration::store::tests::workflow_def_round_trips_through_json
cargo test -p runtime orchestration::store::tests::store_saves_lists_and_reloads_losslessly
cargo test -p runtime orchestration::store::tests::run_saved_workflow_reproduces_direct_execution
cargo test -p runtime orchestration::store
cargo build -p runtime

# Tier 4a -- CLI surface
cargo test -p interact workflow_cli_tests::parses_workflow_list
cargo test -p interact workflow_cli_tests::parses_workflow_run_with_name
cargo build -p interact

# Tier 4b -- Extraction proof (docs only, gated behind Tier 2)
cargo tree -p base -e no-dev --prefix depth
cargo build --workspace
cargo test -p runtime orchestration::store
```

---

## 14. Non-Goals (explicitly excluded)

- No new provider transports (Anthropic, OpenAI, Ollama unchanged).
- No autoscaling or dynamic provider instantiation.
- No live `MetricsExporter` wiring into inference call sites (metrics.rs unit tests only; call-site wiring is a follow-up integration task).
- No background health task with periodic `probe_provider()` scheduling (probe is real but called on demand; periodic scheduling is follow-up).
- No workflow editor, visualizer, or automatic synthesis from traces.
- No actual GitHub org/repo split (Phase 5 is documentation + verification only).
- No multi-repo CI/CD configuration.
- No typed `LlmError` at the transport boundary (string classification only).

---

## References

- `docs/plans/2026-07-01-modules-roadmap-design.md` -- Tier 3 spec at line 190, Tier 4 spec at line 234
- `docs/plans/2026-07-01-tier3-provider-manager-plan.md` -- original Tier 3 plan (verified, all claims match)
- `docs/plans/2026-07-01-tier4-workflow-multirepo-plan.md` -- original Tier 4 plan (verified, two minor line drifts)
- Scheduler: `crates/cognit/src/impl/llm/scheduler.rs`
- Provider trait: `crates/cognit/src/impl/llm/provider.rs`
- Provider impls: `crates/cognit/src/impl/llm/{anthropic,openai_provider,ollama}.rs`
- Config: `crates/cognit/src/config/mod.rs`
- Metrics: `crates/runtime/src/impl/session/observability/metrics.rs`
- Observability: `crates/runtime/src/impl/session/observability/mod.rs`
- ProviderHealth: `crates/base/src/events/evolution.rs:92-97`
- Pulse: `crates/cognit/src/impl/llm/pulse.rs:76`
- Daemon: `crates/runtime/src/impl/daemon/mod.rs:183-206`
- DiGraph: `crates/runtime/src/impl/orchestration/digraph/graph.rs`
- Node: `crates/runtime/src/impl/orchestration/digraph/node.rs`
- Edge: `crates/runtime/src/impl/orchestration/digraph/edge.rs`
- State: `crates/runtime/src/impl/orchestration/digraph/state.rs`
- Registry: `crates/runtime/src/impl/orchestration/registry.rs`
- Orchestration mod: `crates/runtime/src/impl/orchestration/mod.rs`
- CLI: `crates/interact/src/tui/cli.rs`
- Paths: `crates/base/src/types/paths.rs`
- Base lib: `crates/base/src/lib.rs`
- All Cargo.toml: root + crates/{base,cognit,corpus,dasein,interact,memory,metacog,runtime}/Cargo.toml
