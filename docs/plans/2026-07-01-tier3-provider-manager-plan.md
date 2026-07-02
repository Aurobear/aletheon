# Tier 3 — Provider Manager Hardening — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Make multi-provider use robust for real, long-running work: add bounded retry + exponential backoff on transient errors, ordered provider failover on hard errors (not just the single default), real per-provider health probing with circuit-breaking, and per-provider token/cost attribution surfaced through the existing metrics exporter.

**Architecture:** Three seams, all additive. (1) `LlmScheduler::complete` (`crates/cognit/src/impl/llm/scheduler.rs:117-129`) today routes a `LlmPurpose` to exactly one provider and calls it once — no retry, no failover. We wrap the single call in a retry/failover driver that classifies the returned `anyhow::Error` (transient / terminal / context-overflow) to decide retry-vs-failover-vs-surface. (2) `health_check` (`scheduler.rs:157-164`) is a hardcoded `available: true, latency_ms: 0` stub; we replace it with a real per-provider probe that records latency + availability into scheduler-held state, which the candidate ordering then consults (circuit-break). (3) `TokenUsageBreakdown` (`crates/runtime/src/impl/session/observability/metrics.rs:5-11`) is session-global; we add a `provider -> TokenUsageBreakdown` map plus an optional pricing table for cost, keeping every existing global method working. No new provider transports, no autoscaling, pricing table optional/static.

**Tech Stack:** Rust (Cargo workspace), `tokio`, `async-trait`, `anyhow`, `serde`. Crates: `cognit` (Brain), `runtime` (Runtime), `base` (ABI). Errors are untyped `anyhow::Error` produced via `anyhow::bail!("<Provider> API error {status}: {body}")` (`anthropic.rs:248,324`, `openai_provider.rs:368,469`, `ollama.rs:245,318`), so error classification is by inspecting the error's `Display` string / embedded HTTP status.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "Tier 3 — Provider Manager Hardening"

**Branch:** `auro/feat/20260701-aletheon-provider-manager` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Fact | Anchor |
|---|---|
| Roadmap paths omit the `crates/` prefix; real files live under `crates/` | `crates/cognit/src/impl/llm/scheduler.rs`, `crates/runtime/src/impl/session/observability/metrics.rs` |
| Crate `[package] name` values for build/test commands | `crates/cognit/Cargo.toml` → `cognit`; `crates/runtime/Cargo.toml` → `runtime`; `crates/base/Cargo.toml` → `base` |
| `complete()` resolves purpose → one provider and calls it once (no retry/failover) | `scheduler.rs:117-129` (resolve at `:123`, single `provider.complete(...)` at `:128`) |
| Routing fallback is only the single `default_provider` | `scheduler.rs:104-109` (`resolve_provider`) |
| `default_provider` = first configured provider | `scheduler.rs:90-94`; test ctor `from_providers` uses first key `:57` |
| `health_check()` is a stub: `available: true, latency_ms: 0` | `scheduler.rs:157-164` |
| `LlmScheduler` fields (private): `providers: HashMap<String, Arc<dyn LlmProvider>>`, `routing`, `default_provider` | `scheduler.rs:43-47` |
| Test-friendly ctor exists (mock providers) | `from_providers(providers, routing)` `scheduler.rs:53-63` |
| `SchedulerConfig` / `SchedulerProviderConfig` shapes (defined in scheduler.rs, NOT config/mod.rs) | `scheduler.rs:36-40` / `:26-33` |
| Scheduler is built in the daemon from `AppConfig.providers` | `crates/runtime/src/impl/daemon/mod.rs:183-206` |
| `LlmProvider` trait methods | `provider.rs:47-78`: `async complete(&self, &[Message], &[ToolDefinition]) -> Result<LlmResponse>` (`:49`); `async complete_stream(...) -> Result<LlmStream>` (`:56`); `fn name(&self) -> &str` (`:63`); `fn max_context_length(&self) -> usize` (`:66`); default `fn model_info(&self) -> ModelInfo` (`:72`) |
| `LlmResponse` fields | `provider.rs:85-93`: `content: Vec<ContentBlock>`, `stop_reason: StopReason`, `usage: Usage`, `cache_hit_tokens: u32`, `cache_miss_tokens: u32` |
| `StopReason` variants | `provider.rs:95-100`: `EndTurn`, `ToolUse`, `MaxTokens` |
| `Usage` fields (`u32`, `Default`) | `provider.rs:102-106`: `input_tokens`, `output_tokens` |
| Provider API errors are string `anyhow` errors carrying the HTTP status | `anthropic.rs:248` `bail!("Anthropic API error {}: {}", status, body)` (status `Display` = e.g. `429 Too Many Requests`); same shape `openai_provider.rs:368,469`, `ollama.rs:245,318` |
| `ProviderHealth` shape (returned by `health_check`) | `crates/base/src/events/evolution.rs:92-97`: `name: String`, `available: bool`, `latency_ms: u64`, `tokens_remaining: Option<u32>` |
| Only caller of `health_check()` today | `crates/cognit/src/impl/llm/pulse.rs:76` (emits `provider_health` in a pulse event) |
| `TokenUsageBreakdown` fields (`u64`) | `metrics.rs:5-11`: `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`; `total()` at `:14-16` |
| `MetricsExporter` accumulators + methods | `metrics.rs:32-110`: `record_inference(input,output,latency)` `:45`, `record_cache_usage(read,write)` `:71`, `token_usage()` `:77` |
| `MetricsExporter`/`TokenUsageBreakdown` re-export | `crates/runtime/src/impl/session/observability/mod.rs:8` |
| `MetricsExporter` is currently dormant — no instantiation / `record_inference` call site outside its own tests | grep `record_inference`/`MetricsExporter` across `crates` = defs + re-export + tests only |
| Pricing/`ProviderConfig` config surface | `crates/cognit/src/config/mod.rs:133-146` (`ProviderConfig`), `Transport` enum `:119-123` |

> **Drift corrected vs roadmap text:** (a) all three "affected files" are under `crates/`; (b) provider ordering + retry policy belong in `scheduler.rs` (that is where `SchedulerConfig` lives), while only the optional **pricing** table is added to `config/mod.rs`; (c) the metrics exporter is dormant, so per-provider attribution is added and unit-tested in isolation (no live call-site wiring required for this tier).

---

## File map

| File | Change |
|---|---|
| `crates/cognit/src/impl/llm/scheduler.rs` | add `ErrorClass` + `classify_error`; add `RetryPolicy` + failover order + per-provider health state to `LlmScheduler`; rewrite `complete()` as a retry/failover driver; replace `health_check` stub with a real probe (`probe_provider` + refreshed aggregate) |
| `crates/cognit/src/config/mod.rs` | add optional static `ProviderPricing { input_per_1k, output_per_1k }` field to `ProviderConfig` (TOML surface for cost) |
| `crates/runtime/src/impl/session/observability/metrics.rs` | add per-provider `HashMap<String, TokenUsageBreakdown>` + `record_inference_for` / `record_cache_usage_for` / `provider_usage`; add `PricingRate` table + `set_pricing` / `cost_for` / `total_cost` |

Default checks per phase: `cargo test -p cognit` (Phases 1–2) and `cargo test -p runtime` (Phase 3); `cargo build -p cognit` / `-p runtime` before commit.

---

## Phase 1 — Error classification + retry/failover in the scheduler

### Task 1: Classify provider errors (transient / terminal / context-overflow)

**Files:** Modify `crates/cognit/src/impl/llm/scheduler.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// scheduler.rs tests module (append)
#[test]
fn classify_error_buckets_by_signature() {
    // Transient: retryable HTTP + network signatures
    for s in [
        "Anthropic API error 429 Too Many Requests: rate limited",
        "OpenAI API error 503 Service Unavailable: overloaded",
        "Anthropic API error 529: {\"error\":\"overloaded_error\"}",
        "error sending request for url (...): connection reset",
        "request timed out",
    ] {
        assert_eq!(classify_error(&anyhow::anyhow!(s.to_string())), ErrorClass::Transient, "{s}");
    }
    // Context overflow: surface immediately, never failover
    for s in [
        "OpenAI API error 400: This model's maximum context length is 128000 tokens",
        "Anthropic API error 400: prompt is too long: 250000 tokens > 200000",
    ] {
        assert_eq!(classify_error(&anyhow::anyhow!(s.to_string())), ErrorClass::ContextOverflow, "{s}");
    }
    // Terminal: auth / bad request → failover to next provider, no retry
    for s in [
        "Anthropic API error 401 Unauthorized: invalid x-api-key",
        "OpenAI API error 403 Forbidden",
    ] {
        assert_eq!(classify_error(&anyhow::anyhow!(s.to_string())), ErrorClass::Terminal, "{s}");
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`ErrorClass` / `classify_error` undefined).

Run: `cargo test -p cognit llm::scheduler::tests::classify_error_buckets_by_signature`

- [ ] **Step 3: Implement the classifier**

```rust
/// How a provider error should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Retryable: rate limit, 5xx, overloaded, network/timeout.
    Transient,
    /// Prompt exceeds the model context window — retry/failover won't help.
    ContextOverflow,
    /// Auth / bad request / unknown — do not retry; failover to next provider.
    Terminal,
}

/// Classify an `anyhow` provider error by inspecting its Display string.
/// Errors here are untyped `bail!("<Provider> API error {status}: {body}")`
/// plus reqwest transport errors, so we match on stable substrings.
pub fn classify_error(err: &anyhow::Error) -> ErrorClass {
    let m = err.to_string().to_ascii_lowercase();
    // Context overflow first (some providers report it as 400).
    if m.contains("maximum context length")
        || m.contains("context length")
        || m.contains("context_length_exceeded")
        || m.contains("prompt is too long")
        || m.contains("too many tokens")
    {
        return ErrorClass::ContextOverflow;
    }
    if m.contains("429")
        || m.contains("too many requests")
        || m.contains("overloaded")
        || m.contains(" 500")
        || m.contains(" 502")
        || m.contains(" 503")
        || m.contains(" 504")
        || m.contains(" 529")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("error sending request")
        || m.contains("connection")
    {
        return ErrorClass::Transient;
    }
    ErrorClass::Terminal
}
```

- [ ] **Step 4: Run — expected PASS.** Also `cargo test -p cognit llm::scheduler`.

- [ ] **Step 5: Commit**

```bash
git add crates/cognit/src/impl/llm/scheduler.rs
git commit -m "feat(scheduler): classify provider errors (transient/terminal/context-overflow)"
```

### Task 2: Bounded retry + ordered provider failover in `complete()`

**Files:** Modify `crates/cognit/src/impl/llm/scheduler.rs`.

- [ ] **Step 1: Write the failing tests** (compile against the real `LlmProvider` trait)

```rust
// scheduler.rs tests module (append). Mocks implement the real trait
// (provider.rs:47-78) so tests exercise the production types.
use std::sync::atomic::{AtomicUsize, Ordering};
use base::message::ContentBlock;
use super::super::provider::{LlmResponse, LlmStream, StopReason, Usage};

/// Fails transiently `fail_n` times, then returns EndTurn.
struct FlakyProvider { name: String, fail_n: usize, calls: AtomicUsize }
#[async_trait::async_trait]
impl LlmProvider for FlakyProvider {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_n {
            anyhow::bail!("Anthropic API error 429 Too Many Requests: slow down");
        }
        Ok(LlmResponse {
            content: vec![ContentBlock::Text { text: format!("ok-{}", self.name) }],
            stop_reason: StopReason::EndTurn, usage: Usage::default(),
            cache_hit_tokens: 0, cache_miss_tokens: 0,
        })
    }
    async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmStream> { unimplemented!() }
    fn name(&self) -> &str { &self.name }
    fn max_context_length(&self) -> usize { 200_000 }
}

/// Always returns a terminal (non-retryable) error.
struct DeadProvider { name: String }
#[async_trait::async_trait]
impl LlmProvider for DeadProvider {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmResponse> {
        anyhow::bail!("Anthropic API error 401 Unauthorized: invalid x-api-key");
    }
    async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> Result<LlmStream> { unimplemented!() }
    fn name(&self) -> &str { &self.name }
    fn max_context_length(&self) -> usize { 200_000 }
}

fn text_of(r: &LlmResponse) -> String {
    r.content.iter().filter_map(|b| match b {
        ContentBlock::Text { text } => Some(text.clone()), _ => None
    }).collect()
}

#[tokio::test]
async fn transient_error_retries_then_succeeds() {
    let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    let flaky = Arc::new(FlakyProvider { name: "a".into(), fail_n: 2, calls: AtomicUsize::new(0) });
    providers.insert("a".into(), flaky.clone());
    let mut routing = HashMap::new();
    routing.insert(LlmPurpose::Execute, "a".to_string());
    let sched = LlmScheduler::from_providers(providers, routing)
        .with_retry_policy(RetryPolicy { max_retries: 3, base_backoff_ms: 0, max_backoff_ms: 0 });
    let resp = sched.complete(&LlmPurpose::Execute, &[Message::user("hi")], &[]).await.unwrap();
    assert_eq!(text_of(&resp), "ok-a");
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 3, "2 transient failures + 1 success");
}

#[tokio::test]
async fn hard_failure_fails_over_to_next_provider() {
    let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    providers.insert("a".into(), Arc::new(DeadProvider { name: "a".into() }));
    providers.insert("b".into(), Arc::new(FlakyProvider { name: "b".into(), fail_n: 0, calls: AtomicUsize::new(0) }));
    let mut routing = HashMap::new();
    routing.insert(LlmPurpose::Execute, "a".to_string());
    let sched = LlmScheduler::from_providers(providers, routing)
        .with_failover_order(vec!["a".into(), "b".into()])
        .with_retry_policy(RetryPolicy { max_retries: 1, base_backoff_ms: 0, max_backoff_ms: 0 });
    let resp = sched.complete(&LlmPurpose::Execute, &[Message::user("hi")], &[]).await.unwrap();
    assert_eq!(text_of(&resp), "ok-b", "terminal error on 'a' must fail over to 'b'");
}

#[tokio::test]
async fn unhealthy_provider_is_skipped() {
    let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    providers.insert("a".into(), Arc::new(FlakyProvider { name: "a".into(), fail_n: 0, calls: AtomicUsize::new(0) }));
    providers.insert("b".into(), Arc::new(FlakyProvider { name: "b".into(), fail_n: 0, calls: AtomicUsize::new(0) }));
    let mut routing = HashMap::new();
    routing.insert(LlmPurpose::Execute, "a".to_string());
    let sched = LlmScheduler::from_providers(providers, routing)
        .with_failover_order(vec!["a".into(), "b".into()]);
    sched.mark_unhealthy("a"); // circuit-break 'a'
    let resp = sched.complete(&LlmPurpose::Execute, &[Message::user("hi")], &[]).await.unwrap();
    assert_eq!(text_of(&resp), "ok-b", "unhealthy 'a' skipped, 'b' used");
}
```

- [ ] **Step 2: Run — expected FAIL** (`RetryPolicy`, `with_retry_policy`, `with_failover_order`, `mark_unhealthy`, and the retry/failover body do not exist).

Run: `cargo test -p cognit llm::scheduler::tests::hard_failure_fails_over_to_next_provider`

- [ ] **Step 3: Implement retry policy, failover order, health state, and the driver**

Add imports and fields:

```rust
use std::sync::Mutex;
use std::time::Instant;
use tokio::time::{sleep, Duration};

/// Bounded exponential-backoff retry policy for transient errors.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: usize,   // attempts AFTER the first try
    pub base_backoff_ms: u64, // first backoff; doubles each retry
    pub max_backoff_ms: u64,  // cap
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 2, base_backoff_ms: 200, max_backoff_ms: 4_000 }
    }
}

pub struct LlmScheduler {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    routing: HashMap<LlmPurpose, String>,
    default_provider: String,
    retry_policy: RetryPolicy,                       // NEW
    failover_order: Vec<String>,                     // NEW: ordered candidate list
    health: Mutex<HashMap<String, ProviderHealth>>,  // NEW: probe results / circuit-break
}
```

Extend the two constructors (`from_providers` `:53-63`, `new` `:66-101`) to initialise the new fields. Default `failover_order` = provider names with `default_provider` first (`new`) / arbitrary but stable for `from_providers`. Add chainable setters used by tests:

```rust
pub fn with_retry_policy(mut self, p: RetryPolicy) -> Self { self.retry_policy = p; self }
pub fn with_failover_order(mut self, order: Vec<String>) -> Self { self.failover_order = order; self }

/// Circuit-break a provider (skipped while unhealthy).
pub fn mark_unhealthy(&self, name: &str) {
    let mut h = self.health.lock().unwrap();
    let e = h.entry(name.to_string()).or_insert_with(|| ProviderHealth {
        name: name.to_string(), available: true, latency_ms: 0, tokens_remaining: None,
    });
    e.available = false;
}

fn is_healthy(&self, name: &str) -> bool {
    self.health.lock().unwrap().get(name).map(|h| h.available).unwrap_or(true)
}

/// Ordered candidate providers: routed provider first, then failover_order,
/// then any remaining, de-duplicated, skipping circuit-broken ones.
fn candidates(&self, purpose: &LlmPurpose) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut push = |name: &str, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if self.providers.contains_key(name) && self.is_healthy(name) && seen.insert(name.to_string()) {
            out.push(name.to_string());
        }
    };
    let routed = self.resolve_provider(purpose).to_string();
    push(&routed, &mut out, &mut seen);
    for n in self.failover_order.clone() { push(&n, &mut out, &mut seen); }
    for n in self.providers.keys().cloned().collect::<Vec<_>>() { push(&n, &mut out, &mut seen); }
    out
}
```

Rewrite `complete()` (`scheduler.rs:117-129`) as the driver:

```rust
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
        let provider = match self.providers.get(name) { Some(p) => p, None => continue };
        let mut attempt = 0usize;
        loop {
            match provider.complete(messages, tools).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    match classify_error(&e) {
                        ErrorClass::ContextOverflow => return Err(e), // surface immediately
                        ErrorClass::Transient if attempt < self.retry_policy.max_retries => {
                            let shift = attempt as u32;
                            let backoff = self.retry_policy.base_backoff_ms
                                .saturating_mul(1u64 << shift)
                                .min(self.retry_policy.max_backoff_ms);
                            if backoff > 0 { sleep(Duration::from_millis(backoff)).await; }
                            attempt += 1;
                            continue; // retry same provider
                        }
                        _ => { last_err = Some(e); break; } // exhausted or terminal → next provider
                    }
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all providers failed for {:?}", purpose)))
}
```

> `executor_provider`/`reflector_provider` (`:132-151`) are unchanged — they hand out a single provider for direct use and are out of scope for failover.

- [ ] **Step 4: Run — expected PASS.** Full module: `cargo test -p cognit llm::scheduler`.

- [ ] **Step 5: Commit**

```bash
git add crates/cognit/src/impl/llm/scheduler.rs
git commit -m "feat(scheduler): bounded retry + backoff and ordered provider failover"
```

---

## Phase 2 — Real health checks + pricing config surface

### Task 3: Per-provider health probe replacing the stub

**Files:** Modify `crates/cognit/src/impl/llm/scheduler.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// scheduler.rs tests module (append). Reuses FlakyProvider / DeadProvider from Task 2.
#[tokio::test]
async fn probe_records_availability_and_circuit_breaks() {
    let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    providers.insert("ok".into(), Arc::new(FlakyProvider { name: "ok".into(), fail_n: 0, calls: AtomicUsize::new(0) }));
    providers.insert("bad".into(), Arc::new(DeadProvider { name: "bad".into() }));
    let sched = LlmScheduler::from_providers(providers, HashMap::new());

    let good = sched.probe_provider("ok").await;
    assert!(good.available, "reachable provider is available");
    assert_eq!(good.name, "ok");

    let bad = sched.probe_provider("bad").await;
    assert!(!bad.available, "erroring provider is unavailable");
    assert!(!sched.is_healthy("bad"), "failed probe circuit-breaks the provider");

    // Aggregate health_check reflects the default provider's recorded state.
    let agg = sched.health_check().await;
    assert_eq!(agg.name, sched.default_provider_name());
}
```

- [ ] **Step 2: Run — expected FAIL** (`probe_provider`, `default_provider_name` missing; `health_check` still a stub).

Run: `cargo test -p cognit llm::scheduler::tests::probe_records_availability_and_circuit_breaks`

- [ ] **Step 3: Implement the probe + rewrite `health_check`**

```rust
/// Expose the default provider name (for aggregate reporting / tests).
pub fn default_provider_name(&self) -> &str { &self.default_provider }

/// Lightweight liveness probe: a tiny `complete` call, timing the round trip.
/// Records availability + latency into `self.health` (circuit-break on failure).
pub async fn probe_provider(&self, name: &str) -> ProviderHealth {
    let started = Instant::now();
    let result = match self.providers.get(name) {
        Some(p) => p.complete(&[Message::user("ping")], &[]).await.map(|_| ()),
        None => Err(anyhow::anyhow!("unknown provider '{}'", name)),
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    let health = ProviderHealth {
        name: name.to_string(),
        available: result.is_ok(),
        latency_ms,
        tokens_remaining: None,
    };
    self.health.lock().unwrap().insert(name.to_string(), health.clone());
    health
}

/// Aggregate health of the default provider. Probes if we have no cached state.
pub async fn health_check(&self) -> ProviderHealth {
    let name = self.default_provider.clone();
    if let Some(h) = self.health.lock().unwrap().get(&name).cloned() {
        return h;
    }
    self.probe_provider(&name).await
}
```

> Replaces the hardcoded stub at `scheduler.rs:157-164`. The only caller (`pulse.rs:76`) keeps its `ProviderHealth` shape and now receives measured latency/availability. `is_healthy`/`candidates` (Phase 1) already consult this state, so a failed probe automatically removes a provider from the failover list until re-probed.

- [ ] **Step 4: Run — expected PASS.** `cargo test -p cognit llm::scheduler`.

- [ ] **Step 5: Commit**

```bash
git add crates/cognit/src/impl/llm/scheduler.rs
git commit -m "feat(scheduler): real per-provider health probe + circuit-break (replace stub)"
```

### Task 4: Optional static pricing on `ProviderConfig`

**Files:** Modify `crates/cognit/src/config/mod.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// config/mod.rs tests module (append or create #[cfg(test)] mod tests)
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

- [ ] **Step 2: Run — expected FAIL** (`ProviderPricing` / `ProviderConfig.pricing` missing).

Run: `cargo test -p cognit config::tests::provider_pricing_parses_and_defaults_to_none`

- [ ] **Step 3: Add the pricing struct + field**

```rust
/// Optional static per-provider pricing (USD per 1K tokens) for cost accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPricing {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}
```

Add to `ProviderConfig` (after `max_context_length`, `:145`):

```rust
    /// Optional static pricing for per-provider cost accounting. `None` = unpriced.
    #[serde(default)]
    pub pricing: Option<ProviderPricing>,
```

> Additive + `#[serde(default)]`, so every existing config and the `ProviderConfig` literal built in `scheduler.rs:69-80` remain valid (that literal does not name-init `pricing`; verify whether it uses `..Default::default()` — `ProviderConfig` has no `Default`, so it currently lists fields explicitly and **must add `pricing: None`**; make that one-line addition in the same commit to keep `cognit` compiling).

- [ ] **Step 4: Run — expected PASS.** `cargo build -p cognit && cargo test -p cognit config`.

- [ ] **Step 5: Commit**

```bash
git add crates/cognit/src/impl/llm/scheduler.rs crates/cognit/src/config/mod.rs
git commit -m "feat(config): optional static per-provider pricing (input/output per 1k)"
```

---

## Phase 3 — Per-provider token/cost attribution in metrics

### Task 5: Attribute `TokenUsageBreakdown` by provider + compute cost

**Files:** Modify `crates/runtime/src/impl/session/observability/metrics.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// metrics.rs tests module (append)
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

    // Global aggregate still correct (unchanged public surface).
    assert_eq!(ex.token_usage().input_tokens, 3_000);
    assert_eq!(ex.token_usage().output_tokens, 1_500);
    assert_eq!(ex.llm_call_count(), 2);

    // Cost: only anthropic priced -> 1.0k*3 + 0.5k*15 = 10.5 USD.
    ex.set_pricing("anthropic", PricingRate { input_per_1k: 3.0, output_per_1k: 15.0 });
    assert!((ex.cost_for("anthropic").unwrap() - 10.5).abs() < 1e-9);
    assert!(ex.cost_for("openai").is_none(), "unpriced provider has no cost");
    assert!((ex.total_cost() - 10.5).abs() < 1e-9);
}
```

- [ ] **Step 2: Run — expected FAIL** (`record_inference_for`, `record_cache_usage_for`, `provider_usage`, `PricingRate`, `set_pricing`, `cost_for`, `total_cost` missing).

Run: `cargo test -p runtime session::observability::metrics::tests::per_provider_attribution_and_cost`

- [ ] **Step 3: Implement per-provider maps + pricing**

Add `use std::collections::HashMap;` at the top, then:

```rust
/// USD price per 1K tokens for a provider (mirrors config `ProviderPricing`).
#[derive(Debug, Clone, Copy)]
pub struct PricingRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}
```

Extend `MetricsState` (`metrics.rs:20-29`) with two maps:

```rust
    per_provider: HashMap<String, TokenUsageBreakdown>,
    pricing: HashMap<String, PricingRate>,
```

Add provider-scoped recorders that also keep the existing global accumulators authoritative (existing `record_inference` `:45` / `record_cache_usage` `:71` stay as the un-attributed path):

```rust
/// Record an inference attributed to a specific provider (also updates globals).
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

/// Token usage attributed to one provider, if any was recorded.
pub fn provider_usage(&self, provider: &str) -> Option<&TokenUsageBreakdown> {
    self.state.per_provider.get(provider)
}

/// Install a static pricing rate for a provider.
pub fn set_pricing(&mut self, provider: &str, rate: PricingRate) {
    self.state.pricing.insert(provider.to_string(), rate);
}

/// Cost in USD for one provider, if it is both tracked and priced.
pub fn cost_for(&self, provider: &str) -> Option<f64> {
    let usage = self.state.per_provider.get(provider)?;
    let rate = self.state.pricing.get(provider)?;
    Some((usage.input_tokens as f64 / 1000.0) * rate.input_per_1k
        + (usage.output_tokens as f64 / 1000.0) * rate.output_per_1k)
}

/// Total cost across all priced+tracked providers.
pub fn total_cost(&self) -> f64 {
    self.state.per_provider.keys().filter_map(|p| self.cost_for(p)).sum()
}
```

> `TokenUsageBreakdown` already derives `Default` (`metrics.rs:5`), so `entry(..).or_default()` compiles. Existing global tests (`test_record_inference_accumulates` etc.) are untouched because the global accumulators still update first.

- [ ] **Step 4: Run — expected PASS.** Full module: `cargo test -p runtime metrics` and `cargo build -p runtime`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/session/observability/metrics.rs
git commit -m "feat(metrics): per-provider token attribution + optional cost accounting"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** retry/backoff + ordered failover + error classification (Tasks 1–2) ↔ Tier 3 problem 1 & "retry + failover policy"; real health probe + circuit-break (Task 3) ↔ problem 2 & "real health checks"; pricing config (Task 4) + per-provider attribution/cost (Task 5) ↔ problem 3 & "per-provider accounting". Roadmap tests all mapped: injected transient retries then succeeds (Task 2 test 1), hard failure fails over (test 2), unhealthy skipped (test 3), token/cost attributed to the right provider (Task 5).
- **Type consistency:** mocks implement the real `LlmProvider` (`complete`/`complete_stream`/`name`/`max_context_length`, `provider.rs:47-78`); `LlmResponse` literals use the real fields incl. `cache_hit_tokens`/`cache_miss_tokens: u32` (`:90-92`); `StopReason::EndTurn` (`:97`); `ProviderHealth` built with exact fields `name/available/latency_ms/tokens_remaining` (`evolution.rs:92-97`); `TokenUsageBreakdown` `u64` fields + `Default` (`metrics.rs:5-11`). Error classification keys off the real `bail!` message shape (`anthropic.rs:248`).
- **Placeholder scan:** none — every step ships real Rust + exact `cargo` commands. The only cross-cutting edit called out explicitly is adding `pricing: None` to the `ProviderConfig` literal in `scheduler.rs:69-80` (Task 4 Step 3).
- **Build targets:** `cargo {build,test} -p cognit` (Phases 1–2), `-p runtime` (Phase 3) — package names verified from `Cargo.toml` `[package] name`.

## Risks / notes for the implementer

- **String-based error classification is inherently fuzzy.** Errors are untyped `anyhow` strings, so `classify_error` matches substrings/status codes. Guard against over-broad matches (e.g. the `" 500"` / `" 502"` patterns include the leading space to avoid matching token counts like "50000"); if a provider changes its error wording, only the classifier needs updating. A future hardening (out of scope) is a typed `LlmError` at the transport boundary.
- **Idempotency of retries.** Retrying `complete` re-sends the same request. That is safe for chat/completion (no side effects), but do NOT extend this retry driver to tool-executing calls without an idempotency review.
- **`probe_provider` costs a real token call.** The pulse loop (`pulse.rs:76`) calls `health_check` each pulse; `health_check` now serves cached state and only probes when cold. Do not call `probe_provider` in a tight loop — consider a min re-probe interval when wiring a background health task (a follow-up; this plan only makes the probe real, not scheduled).
- **Failover order source.** `with_failover_order` is set programmatically here; the config-driven ordering (feeding `failover_order` from `AppConfig.providers` order at the daemon build site `daemon/mod.rs:183-206`) is the natural wiring step but is additive and left for the integration commit — the scheduler already defaults to a stable order from its provider map.
- **Metrics exporter is dormant.** `MetricsExporter` has no live instantiation today (only re-exported at `observability/mod.rs:8`), so Task 5 is unit-tested in isolation; wiring `record_inference_for` into the inference call sites (e.g. `cognitive_loop.rs:422`, `react_loop/step.rs:37`) is a separate integration task, not part of this design-only tier.
- **Pricing is static/optional (non-goal honored).** No dynamic pricing lookups; unpriced providers simply return `None` from `cost_for`. `PricingRate` (metrics) mirrors `ProviderPricing` (config) but is intentionally a separate type to avoid a `runtime → cognit` type dependency; convert at the wiring site.
- **No new transports, no autoscaling** — Tier 3 is confined to the three files in the file map. If a task tempts a transport or scheduling change, it is out of scope.
