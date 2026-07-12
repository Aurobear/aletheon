pub mod conversions;

use crate::security::{AuditLogger, ToolRunnerWithGuard};
use crate::tools::tools::ToolRegistry;
use anyhow::Result;
use async_trait::async_trait;
use fabric::body::{Action, ActionResult, BodyRuntime};
use fabric::capability::Capability;
use fabric::context::Context;
use fabric::subsystem::{Subsystem, SubsystemContext, SubsystemHealth, Version};
use fabric::Clock;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Aletheon body runtime — tools + sandbox integration.
pub struct AletheonBodyRuntime {
    registry: ToolRegistry,
    runner: Mutex<ToolRunnerWithGuard>,
    capabilities: Vec<Capability>,
    initialized: bool,
    clock: Arc<dyn Clock>,
}

impl AletheonBodyRuntime {
    /// Create a new AletheonBodyRuntime with default tools and security.
    pub fn new(working_dir: PathBuf, clock: Arc<dyn Clock>) -> Result<Self> {
        let registry = ToolRegistry::default();
        let audit_logger = AuditLogger::new(working_dir.join("audit.jsonl"))?;
        let runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, clock.clone());

        let capabilities = Self::build_capabilities(&registry);

        Ok(Self {
            registry,
            runner: Mutex::new(runner),
            capabilities,
            initialized: false,
            clock,
        })
    }

    /// Create with explicit runner (for testing).
    pub fn with_runner(registry: ToolRegistry, runner: ToolRunnerWithGuard, clock: Arc<dyn Clock>) -> Self {
        let capabilities = Self::build_capabilities(&registry);

        Self {
            registry,
            runner: Mutex::new(runner),
            capabilities,
            initialized: false,
            clock,
        }
    }

    /// Build capability list from registered tool definitions.
    fn build_capabilities(registry: &ToolRegistry) -> Vec<Capability> {
        registry
            .definitions()
            .iter()
            .map(|def| {
                // Get permission level by looking up the tool
                let level = registry
                    .get(&def.name)
                    .map(|t| t.permission_level())
                    .unwrap_or(crate::tools::tools::PermissionLevel::L0);
                conversions::tool_to_capability(&def.name, level, &def.description)
            })
            .collect()
    }

    /// Access the tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

#[async_trait]
impl Subsystem for AletheonBodyRuntime {
    fn name(&self) -> &str {
        "aletheon-body"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        self.initialized = true;
        info!(
            "AletheonBodyRuntime initialized with {} capabilities",
            self.capabilities.len()
        );
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if self.initialized && !self.capabilities.is_empty() {
            SubsystemHealth::Healthy
        } else if self.initialized {
            SubsystemHealth::Degraded {
                reason: "No tools registered".to_string(),
            }
        } else {
            SubsystemHealth::Failed {
                reason: "Not initialized".to_string(),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.initialized = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl BodyRuntime for AletheonBodyRuntime {
    async fn execute(&self, action: Action, ctx: &Context) -> Result<ActionResult> {
        let start = self.clock.mono_now();

        // 1. Find the tool
        let tool = self
            .registry
            .get(&action.name)
            .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", action.name))?;

        // 2. Convert context
        let tool_ctx = conversions::context_to_tool_context(ctx, self.clock.clone());

        // 3. Execute through the security runner
        let turn_id = format!("body-{}", ctx.request_id);
        let mut runner = self.runner.lock().await;
        let result = runner
            .execute_tool(tool.as_ref(), action.parameters, &tool_ctx, &turn_id)
            .await;
        drop(runner);

        match result {
            Ok(tool_result) => {
                let mut action_result = conversions::tool_result_to_action_result(&tool_result);
                action_result.elapsed_ms = self.clock.mono_now().0.saturating_sub(start.0);
                Ok(action_result)
            }
            Err(e) => {
                let err_display = e.to_string();
                // Handle ToolError variants into ActionResult errors
                Ok(ActionResult {
                    success: false,
                    output: String::new(),
                    error: Some(err_display),
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    side_effects: Vec::new(),
                })
            }
        }
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn check(&self, action: &Action, _ctx: &Context) -> Result<()> {
        if self.registry.get(&action.name).is_none() {
            return Err(anyhow::anyhow!("Tool not found: {}", action.name));
        }
        // The actual policy/loop checks happen in execute() via ToolRunnerWithGuard
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use fabric::context::Context;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_runtime_init_and_health() {
        let clock = Arc::new(TestClock::default());
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
                clock.clone(),
            ),
            clock,
        );
        // Not yet initialized
        assert_eq!(
            rt.health().await,
            SubsystemHealth::Failed {
                reason: "Not initialized".to_string()
            }
        );

        let mut rt = rt;
        let ctx = SubsystemContext {
            name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            config: serde_json::json!({}),
            bus: None,
        };
        rt.init(&ctx).await.unwrap();

        let health = rt.health().await;
        assert_eq!(health, SubsystemHealth::Healthy);
    }

    #[tokio::test]
    async fn test_capabilities_populated() {
        let clock = Arc::new(TestClock::default());
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
                clock.clone(),
            ),
            clock,
        );
        assert!(!rt.capabilities().is_empty());
    }

    #[tokio::test]
    async fn test_check_unknown_tool() {
        let clock = Arc::new(TestClock::default());
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
                clock.clone(),
            ),
            clock,
        );
        let action = Action {
            name: "nonexistent_tool".to_string(),
            parameters: serde_json::json!({}),
            requires_sandbox: false,
            timeout: None,
        };
        let ctx = Context::new("test", PathBuf::from("/tmp"));
        assert!(rt.check(&action, &ctx).await.is_err());
    }

    #[tokio::test]
    async fn test_version() {
        let clock = Arc::new(TestClock::default());
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
                clock.clone(),
            ),
            clock,
        );
        assert_eq!(rt.version(), Version::new(0, 1, 0));
    }
}
