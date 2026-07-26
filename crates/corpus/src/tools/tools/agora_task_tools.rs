//! Model-visible task operations backed by the same versioned Agora nodes used
//! by Executive stage gates. These replace the separate cognitive task store.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::cognitive_workflow::{
    AgoraProjectionRequest, CognitiveArtifactKind, CognitiveRole, CognitiveStage,
    CognitiveTaskNode, CognitiveTaskNodeId, CognitiveTaskStatus,
};
use fabric::{AgoraOperation, AgoraProposal, AgoraService, AgoraSpaceId, ProcessId};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

#[derive(Clone)]
pub struct AgoraTaskTools {
    service: Arc<dyn AgoraService>,
    host_process: ProcessId,
}

impl AgoraTaskTools {
    pub fn new(service: Arc<dyn AgoraService>, host_process: ProcessId) -> Self {
        Self {
            service,
            host_process,
        }
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(AgoraTaskTool::new(self.clone(), TaskOperation::Create)),
            Arc::new(AgoraTaskTool::new(self.clone(), TaskOperation::Update)),
            Arc::new(AgoraTaskTool::new(self.clone(), TaskOperation::List)),
            Arc::new(AgoraTaskTool::new(self.clone(), TaskOperation::Get)),
        ]
    }

    fn author(&self, context: &ToolContext) -> ProcessId {
        context
            .agent
            .map(|agent| agent.parent_process_id)
            .unwrap_or(self.host_process)
    }

    async fn commit_task(
        &self,
        context: &ToolContext,
        expected_version: u64,
        task: CognitiveTaskNode,
    ) -> anyhow::Result<Value> {
        let author = self.author(context);
        let proposal = AgoraProposal {
            id: Uuid::new_v4(),
            space: AgoraSpaceId(context.session_id.clone()),
            author,
            base_version: expected_version,
            operation: AgoraOperation::UpsertCognitiveTask { task: task.clone() },
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        };
        let permit = fabric::WorkspaceCommitPermit::issue_for(&proposal, i64::MAX)?;
        let proposal_id = self.service.propose(proposal).await?;
        let receipt = self.service.commit(proposal_id, permit).await?;
        Ok(json!({
            "space_id": context.session_id,
            "workspace_version": receipt.commit.version,
            "commit_id": receipt.commit.id,
            "task": task,
        }))
    }
}

#[derive(Debug, Clone, Copy)]
enum TaskOperation {
    Create,
    Update,
    List,
    Get,
}

struct AgoraTaskTool {
    backend: AgoraTaskTools,
    operation: TaskOperation,
}

impl AgoraTaskTool {
    fn new(backend: AgoraTaskTools, operation: TaskOperation) -> Self {
        Self { backend, operation }
    }

    fn result(
        context: &ToolContext,
        start: fabric::MonoTime,
        value: anyhow::Result<Value>,
    ) -> ToolResult {
        match value {
            Ok(value) => ToolResult {
                content: serde_json::to_string_pretty(&value).unwrap_or_default(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: context.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(error) => ToolResult {
                content: json!({ "error": error.to_string() }).to_string(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: context.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

#[async_trait]
impl Tool for AgoraTaskTool {
    fn name(&self) -> &str {
        match self.operation {
            TaskOperation::Create => "task_create",
            TaskOperation::Update => "task_update",
            TaskOperation::List => "task_list",
            TaskOperation::Get => "task_get",
        }
    }

    fn description(&self) -> &str {
        match self.operation {
            TaskOperation::Create => "Create a versioned task in the current Agora workspace",
            TaskOperation::Update => "Update an Agora task at an exact workspace version",
            TaskOperation::List => "List versioned tasks in the current Agora workspace",
            TaskOperation::Get => "Get one task and its bounded committed Agora artifacts",
        }
    }

    fn input_schema(&self) -> Value {
        match self.operation {
            TaskOperation::Create => json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "subject": {"type": "string"},
                    "description": {"type": "string"},
                    "role": {"type": "string", "enum": ["root", "planner", "explorer", "executor", "reviewer", "tester", "fixer"]},
                    "dependencies": {"type": "array", "items": {"type": "string"}},
                    "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
                    "workspace_scope": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["subject", "description"]
            }),
            TaskOperation::Update => json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "status": {"type": "string", "enum": ["pending", "in_progress", "blocked", "completed", "failed", "cancelled"]},
                    "expected_version": {"type": "integer", "minimum": 0}
                },
                "required": ["id", "status", "expected_version"]
            }),
            TaskOperation::Get => json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "role": {"type": "string", "enum": ["root", "planner", "explorer", "executor", "reviewer", "tester", "fixer"]},
                    "max_artifacts": {"type": "integer", "minimum": 0, "maximum": 64}
                },
                "required": ["id"]
            }),
            TaskOperation::List => json!({"type": "object", "properties": {}}),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        match self.operation {
            TaskOperation::List | TaskOperation::Get => ConcurrencyClass::ReadOnly,
            TaskOperation::Create | TaskOperation::Update => ConcurrencyClass::SideEffect,
        }
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self::new(self.backend.clone(), self.operation))
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> ToolResult {
        let start = context.clock.mono_now();
        let result = match self.operation {
            TaskOperation::Create => self.create(input, context).await,
            TaskOperation::Update => self.update(input, context).await,
            TaskOperation::List => self.list(context).await,
            TaskOperation::Get => self.get(input, context).await,
        };
        Self::result(context, start, result)
    }
}

impl AgoraTaskTool {
    async fn create(&self, input: Value, context: &ToolContext) -> anyhow::Result<Value> {
        let subject = required_string(&input, "subject")?;
        let description = required_string(&input, "description")?;
        let view = self
            .backend
            .service
            .view(fabric::AgoraViewRequest {
                space: AgoraSpaceId(context.session_id.clone()),
            })
            .await?;
        let id = input
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let role = parse_role(
            input
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("executor"),
        )?;
        let task = CognitiveTaskNode {
            id: CognitiveTaskNodeId(id),
            parent_id: None,
            objective: format!("{subject}: {description}"),
            role,
            stage: CognitiveStage::Contract,
            status: CognitiveTaskStatus::Pending,
            owner: Some(self.backend.author(context)),
            dependencies: string_array(&input, "dependencies")
                .into_iter()
                .map(CognitiveTaskNodeId)
                .collect(),
            acceptance_criteria: string_array(&input, "acceptance_criteria"),
            workspace_scope: string_array(&input, "workspace_scope"),
            required_artifact_kinds: Vec::new(),
            artifact_refs: Vec::new(),
            unresolved_finding_ids: Vec::new(),
        };
        self.backend.commit_task(context, view.version, task).await
    }

    async fn update(&self, input: Value, context: &ToolContext) -> anyhow::Result<Value> {
        let id = CognitiveTaskNodeId(required_string(&input, "id")?);
        let expected_version = input
            .get("expected_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("expected_version is required"))?;
        let projection = self
            .backend
            .service
            .project_task(AgoraProjectionRequest {
                space: AgoraSpaceId(context.session_id.clone()),
                task_node_id: id,
                role: CognitiveRole::Root,
                max_artifacts: 0,
                include_kinds: Vec::new(),
            })
            .await?;
        anyhow::ensure!(
            projection.workspace_version == expected_version,
            "version conflict: expected {}, actual {}",
            expected_version,
            projection.workspace_version
        );
        let mut task = projection.task;
        task.status = parse_status(&required_string(&input, "status")?)?;
        self.backend
            .commit_task(context, expected_version, task)
            .await
    }

    async fn list(&self, context: &ToolContext) -> anyhow::Result<Value> {
        let list = self
            .backend
            .service
            .list_tasks(AgoraSpaceId(context.session_id.clone()))
            .await?;
        Ok(serde_json::to_value(list)?)
    }

    async fn get(&self, input: Value, context: &ToolContext) -> anyhow::Result<Value> {
        let role = parse_role(input.get("role").and_then(Value::as_str).unwrap_or("root"))?;
        let projection = self
            .backend
            .service
            .project_task(AgoraProjectionRequest {
                space: AgoraSpaceId(context.session_id.clone()),
                task_node_id: CognitiveTaskNodeId(required_string(&input, "id")?),
                role,
                max_artifacts: input
                    .get("max_artifacts")
                    .and_then(Value::as_u64)
                    .unwrap_or(16) as usize,
                include_kinds: Vec::<CognitiveArtifactKind>::new(),
            })
            .await?;
        Ok(serde_json::to_value(projection)?)
    }
}

fn required_string(input: &Value, key: &str) -> anyhow::Result<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}

fn string_array(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn parse_role(value: &str) -> anyhow::Result<CognitiveRole> {
    match value {
        "root" => Ok(CognitiveRole::Root),
        "planner" => Ok(CognitiveRole::Planner),
        "explorer" => Ok(CognitiveRole::Explorer),
        "executor" => Ok(CognitiveRole::Executor),
        "reviewer" => Ok(CognitiveRole::Reviewer),
        "tester" => Ok(CognitiveRole::Tester),
        "fixer" => Ok(CognitiveRole::Fixer),
        _ => anyhow::bail!("unknown cognitive role {value}"),
    }
}

fn parse_status(value: &str) -> anyhow::Result<CognitiveTaskStatus> {
    match value {
        "pending" => Ok(CognitiveTaskStatus::Pending),
        "in_progress" => Ok(CognitiveTaskStatus::Running),
        "blocked" => Ok(CognitiveTaskStatus::Blocked),
        "completed" => Ok(CognitiveTaskStatus::Completed),
        "failed" => Ok(CognitiveTaskStatus::Failed),
        "cancelled" => Ok(CognitiveTaskStatus::Cancelled),
        _ => anyhow::bail!("unknown cognitive task status {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(session: &str) -> ToolContext {
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: std::env::current_dir().unwrap(),
            session_id: session.into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn model_tools_and_host_gate_observe_same_task_version() {
        let service: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )));
        let tools = AgoraTaskTools::new(service.clone(), ProcessId::new()).tools();
        let create = tools
            .iter()
            .find(|tool| tool.name() == "task_create")
            .unwrap();
        let created = create
            .execute(
                json!({
                    "id": "implementation",
                    "subject": "Implement",
                    "description": "versioned cognitive workflow",
                    "role": "executor",
                    "acceptance_criteria": ["tests pass"]
                }),
                &context("root-turn-a"),
            )
            .await;
        assert!(!created.is_error, "{}", created.content);
        let created: Value = serde_json::from_str(&created.content).unwrap();
        assert_eq!(created["workspace_version"], 1);

        let host_projection = service
            .project_task(AgoraProjectionRequest {
                space: AgoraSpaceId("root-turn-a".into()),
                task_node_id: CognitiveTaskNodeId("implementation".into()),
                role: CognitiveRole::Root,
                max_artifacts: 0,
                include_kinds: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(host_projection.workspace_version, 1);
        assert_eq!(host_projection.task.status, CognitiveTaskStatus::Pending);

        let update = tools
            .iter()
            .find(|tool| tool.name() == "task_update")
            .unwrap();
        let updated = update
            .execute(
                json!({
                    "id": "implementation",
                    "status": "in_progress",
                    "expected_version": host_projection.workspace_version
                }),
                &context("root-turn-a"),
            )
            .await;
        assert!(!updated.is_error, "{}", updated.content);
        let list = service
            .list_tasks(AgoraSpaceId("root-turn-a".into()))
            .await
            .unwrap();
        assert_eq!(list.workspace_version, 2);
        assert_eq!(list.tasks[0].status, CognitiveTaskStatus::Running);
    }

    #[tokio::test]
    async fn task_update_rejects_stale_model_version() {
        let service: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )));
        let tools = AgoraTaskTools::new(service, ProcessId::new()).tools();
        let create = tools
            .iter()
            .find(|tool| tool.name() == "task_create")
            .unwrap();
        create
            .execute(
                json!({"id": "task", "subject": "S", "description": "D"}),
                &context("root-turn-b"),
            )
            .await;
        let update = tools
            .iter()
            .find(|tool| tool.name() == "task_update")
            .unwrap();
        let stale = update
            .execute(
                json!({"id": "task", "status": "completed", "expected_version": 0}),
                &context("root-turn-b"),
            )
            .await;
        assert!(stale.is_error);
        assert!(stale.content.contains("version conflict"));
    }
}
