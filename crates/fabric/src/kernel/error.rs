//! Structured error handling for the agent core.
//!
//! Replaces ad-hoc `anyhow::Error` usage in public APIs with typed errors
//! that carry severity, category, and degradation hints.

use std::time::Duration;

use crate::{Clock, Timer};

// ── Error Severity ──────────────────────────────────────────────────────────

/// How severe an error is and what recovery is possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ErrorSeverity {
    /// Can auto-recover (retry, degrade).
    Recoverable,
    /// Functional degradation (e.g., local inference fail -> cloud).
    Degraded,
    /// Needs user intervention.
    Unrecoverable,
    /// Immediate stop, write security log.
    SecurityViolation,
}

impl ErrorSeverity {
    /// Whether automatic retry makes sense.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Recoverable | Self::Degraded)
    }
}

// ── Error Category ──────────────────────────────────────────────────────────

/// The subsystem where the error originated.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorCategory {
    Llm {
        provider: String,
        kind: LlmErrorKind,
    },
    Tool {
        tool: String,
        kind: ToolErrorKind,
    },
    Sandbox {
        kind: SandboxErrorKind,
    },
    Memory {
        kind: MemoryErrorKind,
    },
    Perception {
        source: String,
        kind: PerceptionErrorKind,
    },
    Ipc {
        kind: IpcErrorKind,
    },
    Config {
        kind: ConfigErrorKind,
    },
    Registry {
        kind: RegistryErrorKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LlmErrorKind {
    Timeout,
    RateLimited,
    InvalidResponse,
    AuthFailure,
    ContextOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolErrorKind {
    Timeout,
    PermissionDenied,
    ResourceExhausted,
    SecurityViolation,
    ExecutionFailed,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SandboxErrorKind {
    BubblewrapMissing,
    NamespaceUnavailable,
    OomKilled,
    Timeout,
    LaunchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MemoryErrorKind {
    StoreFull,
    QueryFailed,
    CorruptionDetected,
    ScopeViolation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PerceptionErrorKind {
    SourceUnavailable,
    ParseFailed,
    BackpressureDrop,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IpcErrorKind {
    ConnectionFailed,
    MessageTooLarge,
    PeerDisconnected,
    ProtocolError,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConfigErrorKind {
    Missing,
    Invalid,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RegistryErrorKind {
    AlreadyExists,
    NotFound,
    DependencyCycle,
    DependencyMissing,
    VersionIncompatible,
}

// ── AgentError ──────────────────────────────────────────────────────────────

/// The unified error type for agent-core public APIs.
///
/// Carries structured metadata (severity + category) alongside a human-readable
/// message and an optional source error for chaining.
#[derive(Debug)]
pub struct AgentError {
    pub severity: ErrorSeverity,
    pub category: ErrorCategory,
    pub message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl AgentError {
    /// Create a new AgentError.
    pub fn new(
        severity: ErrorSeverity,
        category: ErrorCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            category,
            message: message.into(),
            source: None,
        }
    }

    /// Attach a source error for chaining.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Whether automatic retry makes sense.
    pub fn is_retryable(&self) -> bool {
        self.severity.is_retryable()
    }

    /// Shorthand constructors for common cases.
    pub fn llm_timeout(provider: &str) -> Self {
        Self::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Llm {
                provider: provider.to_string(),
                kind: LlmErrorKind::Timeout,
            },
            format!("LLM provider {} timed out", provider),
        )
    }

    pub fn llm_rate_limited(provider: &str) -> Self {
        Self::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Llm {
                provider: provider.to_string(),
                kind: LlmErrorKind::RateLimited,
            },
            format!("LLM provider {} rate limited", provider),
        )
    }

    pub fn tool_timeout(tool: &str) -> Self {
        Self::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Tool {
                tool: tool.to_string(),
                kind: ToolErrorKind::Timeout,
            },
            format!("Tool {} timed out", tool),
        )
    }

    pub fn tool_denied(tool: &str, reason: &str) -> Self {
        Self::new(
            ErrorSeverity::SecurityViolation,
            ErrorCategory::Tool {
                tool: tool.to_string(),
                kind: ToolErrorKind::PermissionDenied,
            },
            format!("Tool {} denied: {}", tool, reason),
        )
    }

    pub fn config_missing(key: &str) -> Self {
        Self::new(
            ErrorSeverity::Unrecoverable,
            ErrorCategory::Config {
                kind: ConfigErrorKind::Missing,
            },
            format!("Missing config key: {}", key),
        )
    }

    pub fn already_exists(name: &str) -> Self {
        Self::new(
            ErrorSeverity::Unrecoverable,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::AlreadyExists,
            },
            format!("'{}' already registered", name),
        )
    }

    pub fn not_found(name: &str) -> Self {
        Self::new(
            ErrorSeverity::Unrecoverable,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::NotFound,
            },
            format!("'{}' not found", name),
        )
    }

    pub fn dependency_cycle(detail: &str) -> Self {
        Self::new(
            ErrorSeverity::Unrecoverable,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::DependencyCycle,
            },
            format!("Dependency cycle: {}", detail),
        )
    }

    pub fn hook_timeout(hook: &str, secs: u64) -> Self {
        Self::new(
            ErrorSeverity::Degraded,
            ErrorCategory::Tool {
                tool: hook.to_string(),
                kind: ToolErrorKind::Timeout,
            },
            format!("Hook '{}' timed out after {}s", hook, secs),
        )
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}/{:?}] {}",
            self.severity, self.category, self.message
        )
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

// AgentError implements std::error::Error, so anyhow::Error automatically
// converts via the blanket From<E: Error> impl.

// Allow converting std::io::Error into AgentError.
impl From<std::io::Error> for AgentError {
    fn from(e: std::io::Error) -> Self {
        Self::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Config {
                kind: ConfigErrorKind::Invalid,
            },
            e.to_string(),
        )
        .with_source(e)
    }
}

// ── Backoff Strategy ────────────────────────────────────────────────────────

/// How to space out retry attempts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BackoffStrategy {
    /// Fixed delay between retries.
    Fixed { delay: Duration },
    /// Exponential backoff: base * 2^attempt, capped at max.
    Exponential { base: Duration, max: Duration },
    /// Exponential with jitter -- recommended for production.
    ExponentialWithJitter {
        base: Duration,
        max: Duration,
        jitter: Duration,
    },
    /// Linear backoff: base * attempt.
    Linear { base: Duration },
}

impl BackoffStrategy {
    /// Compute the delay for the given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed { delay } => *delay,
            Self::Exponential { base, max } => {
                let delay = base.mul_f64(2f64.powi(attempt as i32));
                delay.min(*max)
            }
            Self::ExponentialWithJitter { base, max, jitter } => {
                let delay = base.mul_f64(2f64.powi(attempt as i32));
                let capped = delay.min(*max);
                // Deterministic jitter based on attempt (no random in async context)
                let jitter_offset = Duration::from_millis(
                    (jitter.as_millis() as u64 * (attempt as u64 + 1))
                        % jitter.as_millis().max(1) as u64,
                );
                capped + jitter_offset
            }
            Self::Linear { base } => *base * (attempt + 1),
        }
    }
}

/// Recommended default for tool call retries.
pub fn tool_backoff() -> BackoffStrategy {
    BackoffStrategy::ExponentialWithJitter {
        base: Duration::from_millis(500),
        max: Duration::from_secs(30),
        jitter: Duration::from_millis(100),
    }
}

/// Recommended default for LLM call retries.
pub fn llm_backoff() -> BackoffStrategy {
    BackoffStrategy::ExponentialWithJitter {
        base: Duration::from_secs(1),
        max: Duration::from_secs(60),
        jitter: Duration::from_millis(200),
    }
}

// ── Degradation Strategy ────────────────────────────────────────────────────

/// A single step in a degradation chain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DegradationStrategy {
    /// Retry with backoff.
    Retry {
        max_attempts: u32,
        backoff: BackoffStrategy,
    },
    /// Fall back to local inference.
    FallbackToLocal,
    /// Fall back to cloud inference.
    FallbackToCloud,
    /// Reduce context window size.
    ReduceContext,
    /// Skip the failing tool call.
    SkipTool,
    /// Ask the user for help.
    AskUser,
}

/// Action to take when a tool error is encountered.
#[derive(Debug, Clone)]
pub enum ToolErrorAction {
    /// Retry after a delay.
    Retry { delay: Duration },
    /// Skip this tool call.
    Skip { reason: String },
    /// Request elevated permission.
    RequestPermission { required_level: String },
    /// Degrade to an alternative.
    Degrade { alternative: String },
    /// Abort -- no retry, no degradation.
    Abort { reason: String },
    /// Report to user and let them decide.
    ReportToUser { message: String },
}

/// Determine the action for a tool error.
pub fn handle_tool_error(error: &AgentError, tool_name: &str, attempt: u32) -> ToolErrorAction {
    match &error.category {
        ErrorCategory::Tool { kind, .. } => match kind {
            ToolErrorKind::Timeout => {
                if attempt < 2 {
                    ToolErrorAction::Retry {
                        delay: tool_backoff().delay_for_attempt(attempt),
                    }
                } else {
                    ToolErrorAction::Skip {
                        reason: format!("Tool {} timed out after {} retries", tool_name, attempt),
                    }
                }
            }
            ToolErrorKind::PermissionDenied => ToolErrorAction::RequestPermission {
                required_level: "L2".to_string(),
            },
            ToolErrorKind::ResourceExhausted => ToolErrorAction::Degrade {
                alternative: format!("Skip {} and inform user", tool_name),
            },
            ToolErrorKind::SecurityViolation => ToolErrorAction::Abort {
                reason: format!("Security violation in tool {}", tool_name),
            },
            _ => ToolErrorAction::ReportToUser {
                message: error.message.clone(),
            },
        },
        ErrorCategory::Llm {
            kind: LlmErrorKind::Timeout | LlmErrorKind::RateLimited,
            ..
        } => {
            if attempt < 3 {
                ToolErrorAction::Retry {
                    delay: llm_backoff().delay_for_attempt(attempt),
                }
            } else {
                ToolErrorAction::ReportToUser {
                    message: format!("LLM unavailable after {} retries", attempt),
                }
            }
        }
        _ => ToolErrorAction::ReportToUser {
            message: error.message.clone(),
        },
    }
}

// ── Degradation Chain ───────────────────────────────────────────────────────

/// Execute an operation through a chain of degradation strategies.
///
/// Tries each strategy in order. Returns on first success.
/// Returns `AgentError` with `AllStrategiesExhausted` if all fail.
pub struct DegradationChain {
    pub strategies: Vec<DegradationStrategy>,
}

impl DegradationChain {
    pub fn new(strategies: Vec<DegradationStrategy>) -> Self {
        Self { strategies }
    }

    /// Execute an operation through the chain.
    ///
    /// The `operation` closure is called for each Retry strategy.
    /// Other strategies modify behavior but don't re-invoke the operation
    /// (they return control to the caller to handle).
    ///
    /// `clock` is optional; when `None`, retry delays fall back to
    /// `tokio::time::sleep`.
    pub async fn execute<F, Fut, T>(
        &self,
        mut operation: F,
        clock: Option<&dyn Clock>,
    ) -> Result<T, AgentError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, AgentError>>,
    {
        let mut last_error: Option<AgentError> = None;

        for strategy in &self.strategies {
            match strategy {
                DegradationStrategy::Retry {
                    max_attempts,
                    backoff,
                } => {
                    for attempt in 0..*max_attempts {
                        match operation().await {
                            Ok(val) => return Ok(val),
                            Err(e) => {
                                if !e.is_retryable() {
                                    return Err(e);
                                }
                                last_error = Some(e);
                                if attempt < max_attempts - 1 {
                                    let delay = backoff.delay_for_attempt(attempt);
                                    if let Some(c) = clock {
                                        Timer::sleep(c, delay).await;
                                    } else {
                                        tokio::time::sleep(delay).await;
                                    }
                                }
                            }
                        }
                    }
                }
                DegradationStrategy::FallbackToLocal => {
                    // Caller should interpret this as "switch to local model"
                    // and re-invoke. We can't do it here without the provider.
                    continue;
                }
                DegradationStrategy::FallbackToCloud => {
                    continue;
                }
                DegradationStrategy::ReduceContext => {
                    // Caller should reduce context and retry.
                    continue;
                }
                DegradationStrategy::SkipTool => {
                    // If we have a last error, return it as "skipped".
                    if let Some(e) = last_error {
                        return Err(e);
                    }
                    continue;
                }
                DegradationStrategy::AskUser => {
                    return Err(last_error.unwrap_or_else(|| {
                        AgentError::new(
                            ErrorSeverity::Unrecoverable,
                            ErrorCategory::Config {
                                kind: ConfigErrorKind::Invalid,
                            },
                            "All degradation strategies exhausted",
                        )
                    }));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AgentError::new(
                ErrorSeverity::Unrecoverable,
                ErrorCategory::Config {
                    kind: ConfigErrorKind::Invalid,
                },
                "All degradation strategies exhausted",
            )
        }))
    }
}

/// Default degradation chain for LLM inference.
pub fn llm_degradation_chain() -> DegradationChain {
    DegradationChain::new(vec![
        DegradationStrategy::Retry {
            max_attempts: 3,
            backoff: llm_backoff(),
        },
        DegradationStrategy::FallbackToLocal,
        DegradationStrategy::ReduceContext,
        DegradationStrategy::AskUser,
    ])
}

/// Default degradation chain for tool execution.
pub fn tool_degradation_chain() -> DegradationChain {
    DegradationChain::new(vec![
        DegradationStrategy::Retry {
            max_attempts: 2,
            backoff: tool_backoff(),
        },
        DegradationStrategy::SkipTool,
        DegradationStrategy::AskUser,
    ])
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn test_error_severity_retryable() {
        assert!(ErrorSeverity::Recoverable.is_retryable());
        assert!(ErrorSeverity::Degraded.is_retryable());
        assert!(!ErrorSeverity::Unrecoverable.is_retryable());
        assert!(!ErrorSeverity::SecurityViolation.is_retryable());
    }

    #[test]
    fn test_agent_error_display() {
        let e = AgentError::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Llm {
                provider: "openai".to_string(),
                kind: LlmErrorKind::Timeout,
            },
            "request timed out",
        );
        let display = format!("{}", e);
        assert!(display.contains("Recoverable"));
        assert!(display.contains("request timed out"));
    }

    #[test]
    fn test_agent_error_with_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let agent_err = AgentError::new(
            ErrorSeverity::Recoverable,
            ErrorCategory::Config {
                kind: ConfigErrorKind::Missing,
            },
            "config missing",
        )
        .with_source(io_err);

        assert!(agent_err.source().is_some());
    }

    #[test]
    fn test_agent_error_into_anyhow() {
        let e = AgentError::config_missing("api_key");
        let anyhow_err: anyhow::Error = e.into();
        assert!(anyhow_err.to_string().contains("api_key"));
    }

    #[test]
    fn test_backoff_fixed() {
        let backoff = BackoffStrategy::Fixed {
            delay: Duration::from_millis(100),
        };
        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(5), Duration::from_millis(100));
    }

    #[test]
    fn test_backoff_exponential() {
        let backoff = BackoffStrategy::Exponential {
            base: Duration::from_millis(100),
            max: Duration::from_secs(5),
        };
        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(400));
        // Capped at max
        assert_eq!(backoff.delay_for_attempt(10), Duration::from_secs(5));
    }

    #[test]
    fn test_backoff_linear() {
        let backoff = BackoffStrategy::Linear {
            base: Duration::from_millis(100),
        };
        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(300));
    }

    #[test]
    fn test_handle_tool_error_timeout_retry() {
        let e = AgentError::tool_timeout("bash_exec");
        let action = handle_tool_error(&e, "bash_exec", 0);
        assert!(matches!(action, ToolErrorAction::Retry { .. }));
    }

    #[test]
    fn test_handle_tool_error_timeout_skip() {
        let e = AgentError::tool_timeout("bash_exec");
        let action = handle_tool_error(&e, "bash_exec", 3);
        assert!(matches!(action, ToolErrorAction::Skip { .. }));
    }

    #[test]
    fn test_handle_tool_error_security_abort() {
        let e = AgentError::new(
            ErrorSeverity::SecurityViolation,
            ErrorCategory::Tool {
                tool: "bash_exec".to_string(),
                kind: ToolErrorKind::SecurityViolation,
            },
            "blocked by policy",
        );
        let action = handle_tool_error(&e, "bash_exec", 0);
        assert!(matches!(action, ToolErrorAction::Abort { .. }));
    }

    #[test]
    fn test_handle_tool_error_permission_request() {
        let e = AgentError::new(
            ErrorSeverity::SecurityViolation,
            ErrorCategory::Tool {
                tool: "file_write".to_string(),
                kind: ToolErrorKind::PermissionDenied,
            },
            "needs L2",
        );
        let action = handle_tool_error(&e, "file_write", 0);
        assert!(matches!(action, ToolErrorAction::RequestPermission { .. }));
    }

    #[test]
    fn test_shorthand_constructors() {
        let e = AgentError::llm_timeout("openai");
        assert!(e.is_retryable());
        assert!(e.message.contains("openai"));

        let e = AgentError::llm_rate_limited("anthropic");
        assert!(e.is_retryable());

        let e = AgentError::tool_timeout("bash_exec");
        assert!(e.is_retryable());

        let e = AgentError::tool_denied("bash_exec", "policy");
        assert!(!e.is_retryable());

        let e = AgentError::config_missing("key");
        assert!(!e.is_retryable());
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no access");
        let agent_err: AgentError = io_err.into();
        assert_eq!(agent_err.severity, ErrorSeverity::Recoverable);
    }

    #[tokio::test]
    async fn test_degradation_chain_retry_success() {
        let chain = DegradationChain::new(vec![DegradationStrategy::Retry {
            max_attempts: 3,
            backoff: BackoffStrategy::Fixed {
                delay: Duration::from_millis(1),
            },
        }]);

        let attempt = std::sync::atomic::AtomicU32::new(0);
        let result = chain
            .execute(
                || {
                    let prev = attempt.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async move {
                        if prev < 1 {
                            Err(AgentError::new(
                                ErrorSeverity::Recoverable,
                                ErrorCategory::Llm {
                                    provider: "test".to_string(),
                                    kind: LlmErrorKind::Timeout,
                                },
                                "timeout",
                            ))
                        } else {
                            Ok(42)
                        }
                    }
                },
                None,
            )
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_degradation_chain_retry_exhausted() {
        let chain = DegradationChain::new(vec![
            DegradationStrategy::Retry {
                max_attempts: 2,
                backoff: BackoffStrategy::Fixed {
                    delay: Duration::from_millis(1),
                },
            },
            DegradationStrategy::AskUser,
        ]);

        let result = chain
            .execute(
                || async {
                    Err::<i32, _>(AgentError::new(
                        ErrorSeverity::Recoverable,
                        ErrorCategory::Llm {
                            provider: "test".to_string(),
                            kind: LlmErrorKind::Timeout,
                        },
                        "timeout",
                    ))
                },
                None,
            )
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_registry_constructors() {
        let e = AgentError::already_exists("my_subsystem");
        assert!(!e.is_retryable());
        assert!(e.message.contains("my_subsystem"));
        assert!(e.message.contains("already registered"));
        assert!(matches!(
            e.category,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::AlreadyExists
            }
        ));

        let e = AgentError::not_found("missing");
        assert!(!e.is_retryable());
        assert!(e.message.contains("missing"));
        assert!(matches!(
            e.category,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::NotFound
            }
        ));

        let e = AgentError::dependency_cycle("A -> B -> A");
        assert!(!e.is_retryable());
        assert!(e.message.contains("A -> B -> A"));
        assert!(matches!(
            e.category,
            ErrorCategory::Registry {
                kind: RegistryErrorKind::DependencyCycle
            }
        ));

        let e = AgentError::hook_timeout("pre_init", 30);
        assert!(e.is_retryable());
        assert!(e.message.contains("pre_init"));
        assert!(e.message.contains("30s"));
        assert!(matches!(
            e.category,
            ErrorCategory::Tool {
                kind: ToolErrorKind::Timeout,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_degradation_chain_unrecoverable_no_retry() {
        let chain = DegradationChain::new(vec![
            DegradationStrategy::Retry {
                max_attempts: 3,
                backoff: BackoffStrategy::Fixed {
                    delay: Duration::from_millis(1),
                },
            },
            DegradationStrategy::AskUser,
        ]);

        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result = chain
            .execute(
                || {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async {
                        Err::<i32, _>(AgentError::new(
                            ErrorSeverity::SecurityViolation,
                            ErrorCategory::Tool {
                                tool: "test".to_string(),
                                kind: ToolErrorKind::SecurityViolation,
                            },
                            "blocked",
                        ))
                    }
                },
                None,
            )
            .await;

        // Should fail immediately -- SecurityViolation is not retryable
        assert!(result.is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
