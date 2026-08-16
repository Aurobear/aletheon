#![allow(deprecated)]

//! Mock implementations to validate all traits compile and are object-safe.

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

use contracts::*;

// ===== Mock BodyRuntime =====

struct MockBodyRuntime;

#[async_trait]
impl Subsystem for MockBodyRuntime {
    fn name(&self) -> &str {
        "mock_body"
    }
    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        Ok(())
    }
    async fn health(&self) -> SubsystemHealth {
        SubsystemHealth::Healthy
    }
    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl BodyRuntime for MockBodyRuntime {
    async fn execute(&self, _action: Action, _ctx: &Context) -> Result<ActionResult> {
        Ok(ActionResult {
            success: true,
            output: "mock".to_string(),
            error: None,
            elapsed_ms: 0,
            truncated: false,
        })
    }
    fn capabilities(&self) -> &[Capability] {
        &[]
    }
    async fn check(&self, _action: &Action, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

// ===== Tests =====

#[tokio::test]
async fn test_all_traits_compile() {
    let _body: Box<dyn BodyRuntime> = Box::new(MockBodyRuntime);
}

#[tokio::test]
async fn test_context_creation() {
    let ctx = Context::new("test_session", PathBuf::from("/tmp"));
    assert_eq!(ctx.session_id, "test_session");
    assert_eq!(ctx.working_dir, PathBuf::from("/tmp"));
}

#[tokio::test]
async fn test_capability_set() {
    let caps = CapabilitySet::new()
        .with(Capability::new(
            "shell.execute",
            CapabilityLevel::SandboxWrite,
            "Run shell commands",
        ))
        .with(Capability::new(
            "memory.write",
            CapabilityLevel::ReadOnly,
            "Write memories",
        ));

    assert!(caps.has("shell.execute", CapabilityLevel::SandboxWrite));
    assert!(!caps.has("shell.execute", CapabilityLevel::SystemChange));
    assert!(caps.has_capability("memory.write"));
    assert!(!caps.has_capability("self.mutate"));
    assert_eq!(caps.max_level(), CapabilityLevel::SandboxWrite);
}

#[tokio::test]
async fn test_version_compatibility() {
    let v1 = Version::new(1, 0, 0);
    let v2 = Version::new(1, 1, 0);
    let v3 = Version::new(2, 0, 0);

    assert!(v1.is_compatible_with(&v2));
    assert!(!v1.is_compatible_with(&v3));
}

#[tokio::test]
async fn test_mock_subsystem_lifecycle() {
    let mut body = MockBodyRuntime;
    let ctx = SubsystemContext {
        name: "test".to_string(),
        working_dir: PathBuf::from("/tmp"),
        config: serde_json::json!({}),
    };

    assert_eq!(body.name(), "mock_body");
    body.init(&ctx).await.unwrap();
    assert_eq!(body.health().await, SubsystemHealth::Healthy);
    body.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_mock_body_execute() {
    let body = MockBodyRuntime;
    let ctx = Context::new("test", PathBuf::from("/tmp"));
    let action = Action {
        name: "test.action".to_string(),
        parameters: serde_json::json!({}),
        requires_sandbox: false,
        timeout: None,
    };

    let result = body.execute(action, &ctx).await.unwrap();
    assert!(result.success);
    assert_eq!(result.output, "mock");
}
