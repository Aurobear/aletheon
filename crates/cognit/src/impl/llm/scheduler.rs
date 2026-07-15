//! Centralized LLM routing.
//!
//! Other modules do NOT hold LlmProvider directly. They call LlmScheduler::request()
//! which routes to the right provider based on LlmPurpose.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;

use fabric::evolution::{LlmPurpose, ProviderHealth};
use fabric::message::Message;
use fabric::Clock;
use fabric::Timer;

use super::provider::{LlmProvider, LlmResponse, ToolDefinition};
use super::provider_factory::create_provider_by_kind;
use crate::config::{ProviderConfig, Transport};

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
    if m.contains(" 429")
        || m.contains("429 too many requests")
        || m.contains(" 500")
        || m.contains(" 502")
        || m.contains(" 503")
        || m.contains(" 504")
        || m.contains(" 529")
        || m.contains("overloaded")
        || m.contains("timed out")
        || m.contains("timeout")
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

/// Routing rule: maps a purpose to a provider name.
#[derive(Debug, Clone)]
pub struct RoutingRule {
    pub purpose: LlmPurpose,
    pub provider_name: String,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone)]
pub struct SchedulerProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub kind: String, // "anthropic" | "openai" | "ollama"
    pub model: String,
}

/// Full scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub providers: Vec<SchedulerProviderConfig>,
    pub routing: Vec<RoutingRule>,
}

/// Centralized LLM scheduler with purpose-based routing and failover.
pub struct LlmScheduler {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    routing: HashMap<LlmPurpose, String>, // purpose -> provider_name
    default_provider: String,
    /// Retry policy for transient errors (doubles each attempt).
    retry_policy: RetryPolicy,
    /// Ordered provider list for failover (routed provider always attempted first).
    failover_order: Vec<String>,
    /// Per-provider health snapshot (updated by `probe_provider`, consumed by `candidates`).
    health: Mutex<HashMap<String, ProviderHealth>>,
    clock: Arc<dyn Clock>,
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
        Self {
            max_retries: 2,
            base_backoff_ms: 200,
            max_backoff_ms: 4_000,
        }
    }
}

impl LlmScheduler {
    /// Create a scheduler directly from pre-built providers and routing rules.
    ///
    /// Useful for testing with mock providers.
    pub fn from_providers(
        providers: HashMap<String, Arc<dyn LlmProvider>>,
        routing: HashMap<LlmPurpose, String>,
        clock: Arc<dyn Clock>,
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
            clock,
        }
    }

    /// Create a new scheduler from config.
    pub fn new(config: &SchedulerConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        let mut providers = HashMap::new();
        for pc in &config.providers {
            let provider_config = ProviderConfig {
                name: pc.name.clone(),
                base_url: pc.base_url.clone(),
                api_key: resolve_api_key(&pc.api_key, &pc.name),
                transport: match pc.kind.as_str() {
                    "anthropic" => Transport::Anthropic,
                    "ollama" => Transport::Openai,
                    _ => Transport::Openai,
                },
                models: vec![pc.model.clone()],
                max_context_length: None,
                pricing: None,
            };
            let provider = create_provider_by_kind(&pc.kind, &provider_config, &pc.model)?;
            providers.insert(pc.name.clone(), provider);
        }

        let mut routing = HashMap::new();
        for rule in &config.routing {
            routing.insert(rule.purpose.clone(), rule.provider_name.clone());
        }

        let default_provider = config
            .providers
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_default();

        let failover_order: Vec<String> = config.providers.iter().map(|p| p.name.clone()).collect();

        Ok(Self {
            providers,
            routing,
            default_provider,
            retry_policy: RetryPolicy::default(),
            failover_order,
            health: Mutex::new(HashMap::new()),
            clock,
        })
    }

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

    /// Circuit-break a provider (skipped while unhealthy until re-probed).
    pub fn mark_unhealthy(&self, name: &str) {
        let mut h = self.health.lock().unwrap();
        h.entry(name.to_string()).or_insert_with(|| ProviderHealth {
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

    /// Ordered candidates for a purpose, skipping circuit-broken providers.
    fn candidates(&self, purpose: &LlmPurpose) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(self.providers.len());

        let push = |name: String, out: &mut Vec<String>, seen: &mut HashSet<String>| {
            if self.providers.contains_key(&name)
                && self.is_healthy(&name)
                && seen.insert(name.clone())
            {
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

    /// Route a purpose to a provider name.
    fn resolve_provider(&self, purpose: &LlmPurpose) -> &str {
        self.routing
            .get(purpose)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_provider)
    }

    /// Get a provider by name.
    pub fn provider(&self, name: &str) -> Option<&Arc<dyn LlmProvider>> {
        self.providers.get(name)
    }

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
                                aletheon_kernel::chronos::SystemTimer
                                    .sleep(Duration::from_millis(backoff))
                                    .await;
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

        Err(last_err
            .unwrap_or_else(|| anyhow::anyhow!("All providers failed for purpose {:?}", purpose)))
    }

    /// Get the provider for task execution (Engine use).
    pub fn executor_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Execute);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers
                .values()
                .next()
                .expect("No LLM providers configured")
        })
    }

    /// Get the provider for reflection (CognitCore use).
    pub fn reflector_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Reflect);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers
                .values()
                .next()
                .expect("No LLM providers configured")
        })
    }

    /// Expose the default provider name.
    pub fn default_provider_name(&self) -> &str {
        &self.default_provider
    }

    /// Lightweight liveness probe: a tiny `complete` call, recording latency
    /// and availability into `self.health`. Circuit-breaks on failure.
    pub async fn probe_provider(&self, name: &str) -> ProviderHealth {
        let started = self.clock.mono_now();
        let result = match self.providers.get(name) {
            Some(p) => p.complete(&[Message::user("ping")], &[]).await.map(|_| ()),
            None => Err(anyhow::anyhow!("unknown provider '{}'", name)),
        };
        let latency_ms = self.clock.mono_now().0.saturating_sub(started.0);
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
}

/// Resolve API key: config value first, then env var `<NAME>_API_KEY`.
fn resolve_api_key(api_key: &str, provider_name: &str) -> String {
    if !api_key.is_empty() {
        return api_key.to_string();
    }
    let env_name = format!("{}_API_KEY", provider_name.to_uppercase().replace('-', "_"));
    std::env::var(&env_name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::provider::{LlmResponse, LlmStream, StopReason, Usage};
    use super::*;
    use fabric::message::ContentBlock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_resolve_api_key_from_config() {
        assert_eq!(resolve_api_key("sk-secret", "test"), "sk-secret");
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        // When api_key is empty, falls back to env var
        let result = resolve_api_key("", "nonexistent_provider_xyz");
        assert_eq!(result, ""); // env var not set
    }

    #[test]
    fn test_scheduler_config_construction() {
        let config = SchedulerConfig {
            providers: vec![SchedulerProviderConfig {
                name: "executor".to_string(),
                base_url: "https://api.openai.com".to_string(),
                api_key: "sk-test".to_string(),
                kind: "openai".to_string(),
                model: "gpt-4o".to_string(),
            }],
            routing: vec![RoutingRule {
                purpose: LlmPurpose::Execute,
                provider_name: "executor".to_string(),
            }],
        };
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.routing.len(), 1);
    }

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
            anyhow::bail!("mock(SlowProvider): streaming not implemented")
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
            anyhow::bail!("mock(DeadProvider): streaming not supported for dead/exhausted provider")
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
        let sched = LlmScheduler::from_providers(
            providers,
            routing,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .with_retry_policy(RetryPolicy {
            max_retries: 3,
            base_backoff_ms: 0,
            max_backoff_ms: 0,
        });
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
            Arc::new(FlakyProvider {
                name: "b".into(),
                fail_n: 0,
                calls: AtomicUsize::new(0),
            }),
        );
        let mut routing = HashMap::new();
        routing.insert(LlmPurpose::Execute, "a".to_string());
        let sched = LlmScheduler::from_providers(
            providers,
            routing,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .with_failover_order(vec!["a".into(), "b".into()])
        .with_retry_policy(RetryPolicy {
            max_retries: 1,
            base_backoff_ms: 0,
            max_backoff_ms: 0,
        });
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
            Arc::new(FlakyProvider {
                name: "a".into(),
                fail_n: 0,
                calls: AtomicUsize::new(0),
            }),
        );
        providers.insert(
            "b".into(),
            Arc::new(FlakyProvider {
                name: "b".into(),
                fail_n: 0,
                calls: AtomicUsize::new(0),
            }),
        );
        let mut routing = HashMap::new();
        routing.insert(LlmPurpose::Execute, "a".to_string());
        let sched = LlmScheduler::from_providers(
            providers,
            routing,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .with_failover_order(vec!["a".into(), "b".into()]);
        sched.mark_unhealthy("a");
        let resp = sched
            .complete(&LlmPurpose::Execute, &[Message::user("hi")], &[])
            .await
            .unwrap();
        assert_eq!(text_of(&resp), "ok-b");
    }

    #[tokio::test]
    async fn probe_records_availability_and_circuit_breaks() {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        providers.insert(
            "ok".into(),
            Arc::new(FlakyProvider {
                name: "ok".into(),
                fail_n: 0,
                calls: AtomicUsize::new(0),
            }),
        );
        providers.insert("bad".into(), Arc::new(DeadProvider { name: "bad".into() }));
        let sched = LlmScheduler::from_providers(
            providers,
            HashMap::new(),
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );

        let good = sched.probe_provider("ok").await;
        assert!(good.available, "reachable provider is available");
        assert_eq!(good.name, "ok");

        let bad = sched.probe_provider("bad").await;
        assert!(!bad.available, "erroring provider is unavailable");
        assert!(!sched.is_healthy("bad"), "failed probe circuit-breaks");

        let agg = sched.health_check().await;
        assert_eq!(agg.name, sched.default_provider_name());
    }
}
