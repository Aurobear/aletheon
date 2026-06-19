//! Mock implementations to validate all traits compile and are object-safe.

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

use aletheon_abi::*;

// ===== Mock EventBus =====

struct MockEventBus;

#[async_trait]
impl EventBus for MockEventBus {
    async fn publish(&self, _event: Box<dyn Event>) -> Result<()> {
        Ok(())
    }
    async fn subscribe(
        &self,
        _event_type: EventType,
        _handler: EventHandler,
    ) -> Result<SubscriptionId> {
        Ok(SubscriptionId(1))
    }
    async fn request(
        &self,
        _event: Box<dyn Event>,
        _timeout: std::time::Duration,
    ) -> Result<Box<dyn Event>> {
        unimplemented!()
    }
    async fn unsubscribe(&self, _id: SubscriptionId) -> Result<()> {
        Ok(())
    }
    async fn has_subscribers(&self, _event_type: &EventType) -> bool {
        false
    }
}

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
            side_effects: vec![],
        })
    }
    fn capabilities(&self) -> &[Capability] {
        &[]
    }
    async fn check(&self, _action: &Action, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

// ===== Mock Memory =====

struct MockMemory;

#[async_trait]
impl Subsystem for MockMemory {
    fn name(&self) -> &str {
        "mock_memory"
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
impl MemoryBackend for MockMemory {
    async fn store(&self, _entry: MemoryEntry) -> Result<MemoryHandle> {
        Ok(MemoryHandle {
            id: uuid::Uuid::new_v4(),
            memory_type: MemoryType::Episodic,
        })
    }
    async fn recall(&self, _query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn list(&self, _filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _handle: &MemoryHandle) -> Result<()> {
        Ok(())
    }
    async fn compact(&self, _strategy: CompactStrategy) -> Result<CompactResult> {
        Ok(CompactResult {
            entries_before: 0,
            entries_after: 0,
            entries_removed: 0,
            entries_merged: 0,
        })
    }
    async fn stats(&self) -> Result<MemoryStats> {
        Ok(MemoryStats {
            total_entries: 0,
            by_type: Default::default(),
            total_size_bytes: 0,
            oldest_entry: None,
            newest_entry: None,
        })
    }
}

// ===== Mock SelfField =====

struct MockSelfField;

#[async_trait]
impl Subsystem for MockSelfField {
    fn name(&self) -> &str {
        "mock_self_field"
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
impl SelfFieldOps for MockSelfField {
    async fn review(&self, _intent: &Intent, _ctx: &Context) -> Result<Verdict> {
        Ok(Verdict::Allow)
    }
    async fn identity(&self) -> Result<Identity> {
        Ok(Identity {
            name: "mock".to_string(),
            description: "Mock agent".to_string(),
            version: "0.1.0".to_string(),
            created_at: chrono::Utc::now(),
            last_mutation: None,
        })
    }
    async fn cares(&self) -> Result<Vec<Care>> {
        Ok(vec![])
    }
    async fn narrate(&self, _event: &str, _reason: &str) -> Result<()> {
        Ok(())
    }
    async fn resolve_conflict(&self, _conflict: &Conflict) -> Result<Resolution> {
        Ok(Resolution::AcceptA {
            reason: "mock".to_string(),
        })
    }
    async fn review_mutation(&self, _mutation: &MutationIntent) -> Result<Verdict> {
        Ok(Verdict::Allow)
    }
}

// ===== Mock BrainCore =====

struct MockBrainCore;

#[async_trait]
impl Subsystem for MockBrainCore {
    fn name(&self) -> &str {
        "mock_brain_core"
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
impl BrainCoreOps for MockBrainCore {
    async fn think(&self, _intent: &Intent, _ctx: &Context) -> Result<Plan> {
        Ok(Plan {
            id: uuid::Uuid::new_v4(),
            steps: vec![],
            estimated_cost: CostEstimate::default(),
            risk_level: self_field::RiskLevel::None,
            reasoning: "mock".to_string(),
            alternatives: vec![],
        })
    }
    async fn reflect(&self, _execution: &ExecutionResult) -> Result<Reflection> {
        Ok(Reflection {
            what_worked: vec![],
            what_failed: vec![],
            what_to_improve: vec![],
            confidence: 1.0,
        })
    }
    async fn critique(&self, _plan: &Plan) -> Result<Vec<Critique>> {
        Ok(vec![])
    }
    async fn learn(&self, _experience: &Experience) -> Result<Vec<LearnedRule>> {
        Ok(vec![])
    }
    async fn update_world(&self, _observation: &Observation) -> Result<()> {
        Ok(())
    }
}

// ===== Tests =====

#[tokio::test]
async fn test_all_traits_compile() {
    let _bus: Box<dyn EventBus> = Box::new(MockEventBus);
    let _body: Box<dyn BodyRuntime> = Box::new(MockBodyRuntime);
    let _memory: Box<dyn MemoryBackend> = Box::new(MockMemory);
    let _self_field: Box<dyn SelfFieldOps> = Box::new(MockSelfField);
    let _brain: Box<dyn BrainCoreOps> = Box::new(MockBrainCore);
}

#[tokio::test]
async fn test_context_creation() {
    let ctx = Context::new("test_session", PathBuf::from("/tmp"));
    assert_eq!(ctx.session_id, "test_session");
    assert_eq!(ctx.working_dir, PathBuf::from("/tmp"));

    let child = ctx.child();
    assert_eq!(child.session_id, "test_session");
    assert_eq!(child.trace.parent_span_id, Some(ctx.trace.span_id.clone()));
}

#[tokio::test]
async fn test_capability_set() {
    let caps = CapabilitySet::new()
        .with(Capability::new(
            "shell.execute",
            PermissionLevel::SandboxWrite,
            "Run shell commands",
        ))
        .with(Capability::new(
            "memory.write",
            PermissionLevel::ReadOnly,
            "Write memories",
        ));

    assert!(caps.has("shell.execute", PermissionLevel::SandboxWrite));
    assert!(!caps.has("shell.execute", PermissionLevel::SystemChange));
    assert!(caps.has_capability("memory.write"));
    assert!(!caps.has_capability("self.mutate"));
    assert_eq!(caps.max_level(), PermissionLevel::SandboxWrite);
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

#[tokio::test]
async fn test_mock_self_field_review() {
    let sf = MockSelfField;
    let ctx = Context::new("test", PathBuf::from("/tmp"));
    let intent = Intent {
        action: "test.action".to_string(),
        parameters: serde_json::json!({}),
        source: IntentSource::User,
        description: "Test intent".to_string(),
    };

    let verdict = sf.review(&intent, &ctx).await.unwrap();
    assert!(matches!(verdict, Verdict::Allow));
}
