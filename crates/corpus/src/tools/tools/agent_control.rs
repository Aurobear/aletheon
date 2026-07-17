//! Thin, context-bound clients for the Fabric Agent control contract.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::agent_control::MAX_LIST_ITEMS;
use fabric::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlError, AgentControlPort, AgentId, AgentListRequest,
    AgentProfileId, AgentSendRequest, AgentSpawnRequest, AgentWaitRequest, RuntimeId,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct AgentControlTools {
    control: Arc<dyn AgentControlPort>,
}

impl AgentControlTools {
    pub fn new(control: Arc<dyn AgentControlPort>) -> Self {
        Self { control }
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        [
            AgentControlOperation::Spawn,
            AgentControlOperation::Wait,
            AgentControlOperation::Send,
            AgentControlOperation::Cancel,
            AgentControlOperation::List,
        ]
        .into_iter()
        .map(|operation| {
            Arc::new(AgentControlTool {
                control: self.control.clone(),
                operation,
            }) as Arc<dyn Tool>
        })
        .collect()
    }
}

#[derive(Clone, Copy)]
enum AgentControlOperation {
    Spawn,
    Wait,
    Send,
    Cancel,
    List,
}

struct AgentControlTool {
    control: Arc<dyn AgentControlPort>,
    operation: AgentControlOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnInput {
    profile: String,
    runtime: String,
    task: String,
    #[serde(default)]
    context: AgentContextFork,
    #[serde(default)]
    tools: Vec<String>,
    budget: AgentBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitInput {
    agent_id: AgentId,
    timeout_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendInput {
    agent_id: AgentId,
    message: String,
    #[serde(default)]
    start_turn: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentInput {
    agent_id: AgentId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    #[serde(default)]
    status: Option<fabric::AgentRunStatus>,
    limit: usize,
}

#[async_trait]
impl Tool for AgentControlTool {
    fn name(&self) -> &str {
        match self.operation {
            AgentControlOperation::Spawn => "agent_spawn",
            AgentControlOperation::Wait => "agent_wait",
            AgentControlOperation::Send => "agent_send",
            AgentControlOperation::Cancel => "agent_cancel",
            AgentControlOperation::List => "agent_list",
        }
    }

    fn description(&self) -> &str {
        match self.operation {
            AgentControlOperation::Spawn => {
                "Spawn a bounded child Agent and return its durable handle"
            }
            AgentControlOperation::Wait => {
                "Wait for a child Agent terminal snapshot with an explicit timeout"
            }
            AgentControlOperation::Send => "Persist and send a message to a live child Agent",
            AgentControlOperation::Cancel => "Cancel a child Agent and return its durable snapshot",
            AgentControlOperation::List => "List durable child Agent snapshots in the caller root",
        }
    }

    fn input_schema(&self) -> Value {
        match self.operation {
            AgentControlOperation::Spawn => json!({
                "type":"object","additionalProperties":false,
                "properties":{
                    "profile":{"type":"string","minLength":1,"maxLength":512},
                    "runtime":{"type":"string","minLength":1,"maxLength":512},
                    "task":{"type":"string","minLength":1,"maxLength":65536},
                    "context":{"type":"object"},
                    "tools":{"type":"array","maxItems":256,"items":{"type":"string","minLength":1,"maxLength":512}},
                    "budget":{
                        "type":"object","additionalProperties":false,
                        "properties":{
                            "max_input_tokens":{"type":"integer","minimum":1},
                            "max_output_tokens":{"type":"integer","minimum":1},
                            "max_tool_calls":{"type":"integer","minimum":1},
                            "max_elapsed_ms":{"type":"integer","minimum":1},
                            "max_cost_usd":{"type":["number","null"],"minimum":0},
                            "max_depth":{"type":"integer","minimum":1}
                        },
                        "required":["max_input_tokens","max_output_tokens","max_tool_calls","max_elapsed_ms","max_depth"]
                    }
                },
                "required":["profile","runtime","task","budget"]
            }),
            AgentControlOperation::Wait => json!({
                "type":"object","additionalProperties":false,
                "properties":{"agent_id":{"type":"string","format":"uuid"},"timeout_ms":{"type":"integer","minimum":1}},
                "required":["agent_id","timeout_ms"]
            }),
            AgentControlOperation::Send => json!({
                "type":"object","additionalProperties":false,
                "properties":{"agent_id":{"type":"string","format":"uuid"},"message":{"type":"string","minLength":1,"maxLength":65536},"start_turn":{"type":"boolean"}},
                "required":["agent_id","message"]
            }),
            AgentControlOperation::Cancel => json!({
                "type":"object","additionalProperties":false,
                "properties":{"agent_id":{"type":"string","format":"uuid"}},
                "required":["agent_id"]
            }),
            AgentControlOperation::List => json!({
                "type":"object","additionalProperties":false,
                "properties":{"status":{"type":["string","null"],"enum":["queued","running","waiting","succeeded","failed","cancelled","interrupted",null]},"limit":{"type":"integer","minimum":1,"maximum":MAX_LIST_ITEMS}},
                "required":["limit"]
            }),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self {
            control: self.control.clone(),
            operation: self.operation,
        })
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> ToolResult {
        let Some(trusted) = context.agent else {
            return error_json("missing_trusted_agent_context");
        };
        let result = match self.operation {
            AgentControlOperation::Spawn => match serde_json::from_value::<SpawnInput>(input) {
                Ok(input) => {
                    let trusted_workspace = match context.effective_workspace_policy() {
                        Ok(workspace) => workspace,
                        Err(_) => return error_json("invalid_trusted_workspace"),
                    };
                    let request = AgentSpawnRequest {
                        root_agent_id: trusted.caller_root_agent_id,
                        parent_agent_id: Some(trusted.parent_agent_id),
                        parent_process_id: Some(trusted.parent_process_id),
                        profile_id: AgentProfileId(input.profile),
                        runtime_id: RuntimeId(input.runtime),
                        trusted_workspace: Some(trusted_workspace),
                        task: input.task,
                        context: input.context,
                        broadcast_refs: vec![],
                        allowed_tools: input.tools,
                        budget: input.budget,
                    };
                    match request.validate() {
                        Ok(()) => self.control.spawn(request).await.and_then(to_value),
                        Err(error) => Err(error),
                    }
                }
                Err(_) => return error_json("invalid_agent_spawn_input"),
            },
            AgentControlOperation::Wait => match serde_json::from_value::<WaitInput>(input) {
                Ok(input) => {
                    let request = AgentWaitRequest {
                        caller_root_agent_id: trusted.caller_root_agent_id,
                        agent_id: input.agent_id,
                        timeout_ms: input.timeout_ms,
                    };
                    match request.validate() {
                        Ok(()) => self.control.wait(request).await.and_then(to_value),
                        Err(error) => Err(error),
                    }
                }
                Err(_) => return error_json("invalid_agent_wait_input"),
            },
            AgentControlOperation::Send => match serde_json::from_value::<SendInput>(input) {
                Ok(input) => {
                    let request = AgentSendRequest {
                        caller_root_agent_id: trusted.caller_root_agent_id,
                        sender_agent_id: Some(trusted.parent_agent_id),
                        agent_id: input.agent_id,
                        kind: fabric::AgentMessageKind::Input,
                        delivery_id: None,
                        correlation_id: None,
                        deadline_mono_ms: None,
                        message: input.message,
                        start_turn: input.start_turn,
                    };
                    match request.validate() {
                        Ok(()) => self.control.send(request).await.and_then(to_value),
                        Err(error) => Err(error),
                    }
                }
                Err(_) => return error_json("invalid_agent_send_input"),
            },
            AgentControlOperation::Cancel => match serde_json::from_value::<AgentInput>(input) {
                Ok(input) => self
                    .control
                    .cancel(trusted.caller_root_agent_id, input.agent_id)
                    .await
                    .and_then(to_value),
                Err(_) => return error_json("invalid_agent_cancel_input"),
            },
            AgentControlOperation::List => match serde_json::from_value::<ListInput>(input) {
                Ok(input) => {
                    let request = AgentListRequest {
                        caller_root_agent_id: trusted.caller_root_agent_id,
                        status: input.status,
                        limit: input.limit,
                    };
                    match request.validate() {
                        Ok(()) => self.control.list(request).await.and_then(to_value),
                        Err(error) => Err(error),
                    }
                }
                Err(_) => return error_json("invalid_agent_list_input"),
            },
        };
        match result {
            Ok(value) => success_json(value),
            Err(error) => control_error_json(error),
        }
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, AgentControlError> {
    serde_json::to_value(value).map_err(|_| AgentControlError {
        kind: fabric::AgentControlErrorKind::Runtime,
        message: "Agent result serialization failed".into(),
    })
}

fn success_json(value: Value) -> ToolResult {
    ToolResult {
        content: serde_json::to_string(&json!({"ok":true,"result":value})).unwrap(),
        is_error: false,
        metadata: ToolResultMeta::default(),
    }
}

fn control_error_json(error: AgentControlError) -> ToolResult {
    let kind = serde_json::to_value(error.kind).unwrap_or(json!("runtime"));
    ToolResult {
        content: serde_json::to_string(&json!({"ok":false,"error":{"kind":kind}})).unwrap(),
        is_error: true,
        metadata: ToolResultMeta::default(),
    }
}

fn error_json(code: &str) -> ToolResult {
    ToolResult {
        content: serde_json::to_string(
            &json!({"ok":false,"error":{"kind":"invalid_request","code":code}}),
        )
        .unwrap(),
        is_error: true,
        metadata: ToolResultMeta::default(),
    }
}
