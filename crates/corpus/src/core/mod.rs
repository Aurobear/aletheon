pub mod conversions;

use crate::security::security::{AuditLogger, ToolRunnerWithGuard};
use crate::tools::tools::ToolRegistry;
use base::body::{Action, ActionResult, BodyRuntime};
use base::capability::Capability;
use base::context::Context;
use base::subsystem::{Subsystem, SubsystemContext, SubsystemHealth, Version};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::info;

/// Aletheon body runtime — tools + sandbox integration.
pub struct AletheonBodyRuntime {
    registry: ToolRegistry,
    runner: Mutex<ToolRunnerWithGuard>,
    capabilities: Vec<Capability>,
    initialized: bool,
}

impl AletheonBodyRuntime {
    /// Create a new AletheonBodyRuntime with default tools and security.
    pub fn new(working_dir: PathBuf) -> Result<Self> {
        let registry = ToolRegistry::default();
        let audit_logger = AuditLogger::new(working_dir.join("audit.jsonl"))?;
        let runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger);

        let capabilities = Self::build_capabilities(&registry);

        Ok(Self {
            registry,
            runner: Mutex::new(runner),
            capabilities,
            initialized: false,
        })
    }

    /// Create with explicit runner (for testing).
    pub fn with_runner(registry: ToolRegistry, runner: ToolRunnerWithGuard) -> Self {
        let capabilities = Self::build_capabilities(&registry);

        Self {
            registry,
            runner: Mutex::new(runner),
            capabilities,
            initialized: false,
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
        let start = std::time::Instant::now();

        // 1. Find the tool
        let tool = self
            .registry
            .get(&action.name)
            .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", action.name))?;

        // 2. Convert context
        let tool_ctx = conversions::context_to_tool_context(ctx);

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
                action_result.elapsed_ms = start.elapsed().as_millis() as u64;
                Ok(action_result)
            }
            Err(e) => {
                let err_display = e.to_string();
                // Handle ToolError variants into ActionResult errors
                Ok(ActionResult {
                    success: false,
                    output: String::new(),
                    error: Some(err_display),
                    elapsed_ms: start.elapsed().as_millis() as u64,
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
    use base::context::Context;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_runtime_init_and_health() {
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
            ),
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
            bus: std::sync::Arc::new(base::CommunicationBus::new()),
        };
        rt.init(&ctx).await.unwrap();

        let health = rt.health().await;
        assert_eq!(health, SubsystemHealth::Healthy);
    }

    #[tokio::test]
    async fn test_capabilities_populated() {
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
            ),
        );
        assert!(!rt.capabilities().is_empty());
    }

    #[tokio::test]
    async fn test_check_unknown_tool() {
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
            ),
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
        let rt = AletheonBodyRuntime::with_runner(
            ToolRegistry::default(),
            ToolRunnerWithGuard::with_default_sandbox(
                AuditLogger::new(PathBuf::from("/dev/null")).unwrap(),
            ),
        );
        assert_eq!(rt.version(), Version::new(0, 1, 0));
    }
}
