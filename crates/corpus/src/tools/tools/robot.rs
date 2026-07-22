//! Narrow governed robot tools backed only by the Fabric embodiment port.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("'{field}' must be a non-empty string"))
}

fn result(ctx: &ToolContext, start: fabric::MonoTime, value: Result<Value, String>) -> ToolResult {
    let (content, is_error) = match value {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(content) => (content, false),
            Err(error) => (format!("robot result serialization failed: {error}"), true),
        },
        Err(error) => (error, true),
    };
    ToolResult {
        content,
        is_error,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

macro_rules! robot_tool {
    ($name:ident, $tool_name:literal, $description:literal, $permission:expr, $concurrency:expr, $schema:expr, |$this:ident, $input:ident| $body:block) => {
        pub struct $name {
            port: Arc<dyn fabric::EmbodimentExecutionPort>,
        }

        impl $name {
            pub fn new(port: Arc<dyn fabric::EmbodimentExecutionPort>) -> Self {
                Self { port }
            }
        }

        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str { $tool_name }
            fn description(&self) -> &str { $description }
            fn input_schema(&self) -> Value { $schema }
            fn permission_level(&self) -> PermissionLevel { $permission }
            fn concurrency_class(&self) -> ConcurrencyClass { $concurrency }
            fn boxed_clone(&self) -> Box<dyn Tool> { Box::new(Self::new(self.port.clone())) }
            async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
                let start = ctx.clock.mono_now();
                let $this = self;
                let $input = input;
                let value: Result<Value, String> = async move $body.await;
                result(ctx, start, value)
            }
        }
    };
}

fn device_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"device": {"type": "string", "minLength": 1}},
        "required": ["device"],
        "additionalProperties": false
    })
}

robot_tool!(
    RobotObserveTool,
    "robot.observe",
    "Read current normalized robot observations",
    PermissionLevel::L0,
    ConcurrencyClass::ReadOnly,
    device_schema(),
    |this, input| {
        let device = required_string(&input, "device")?;
        this.port
            .observe(&fabric::DeviceId(device))
            .await
            .map(|value| json!(value))
            .map_err(|error| error.to_string())
    }
);

robot_tool!(
    RobotGetStateTool,
    "robot.get_state",
    "Read the latest normalized robot state",
    PermissionLevel::L0,
    ConcurrencyClass::ReadOnly,
    device_schema(),
    |this, input| {
        let device = required_string(&input, "device")?;
        this.port
            .get_state(&fabric::DeviceId(device))
            .await
            .map(|value| json!(value))
            .map_err(|error| error.to_string())
    }
);

robot_tool!(
    RobotListSkillsTool,
    "robot.list_skills",
    "List registered skills for a robot device",
    PermissionLevel::L0,
    ConcurrencyClass::ReadOnly,
    device_schema(),
    |this, input| {
        let device = required_string(&input, "device")?;
        this.port
            .list_skills(&fabric::DeviceId(device))
            .await
            .map(|value| json!(value))
            .map_err(|error| error.to_string())
    }
);

robot_tool!(
    RobotExecuteSkillTool,
    "robot.execute_skill",
    "Execute one registered robot skill with JSON parameters",
    PermissionLevel::L2,
    ConcurrencyClass::SideEffect,
    json!({
        "type": "object",
        "properties": {
            "device": {"type": "string", "minLength": 1},
            "skill": {"type": "string", "minLength": 1},
            "parameters": {"type": "object"}
        },
        "required": ["device", "skill"],
        "additionalProperties": false
    }),
    |this, input| {
        let device = required_string(&input, "device")?;
        let skill = required_string(&input, "skill")?;
        let parameters = input
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !parameters.is_object() {
            return Err("'parameters' must be an object".into());
        }
        this.port
            .execute_skill(fabric::SkillRequest {
                device: fabric::DeviceId(device),
                skill: fabric::SkillId(skill),
                parameters,
            })
            .await
            .map(|value| json!(value))
            .map_err(|error| error.to_string())
    }
);

robot_tool!(
    RobotCancelTool,
    "robot.cancel",
    "Cancel one active robot operation by host-issued UUID",
    PermissionLevel::L2,
    ConcurrencyClass::SideEffect,
    json!({
        "type": "object",
        "properties": {"operation_id": {"type": "string", "format": "uuid"}},
        "required": ["operation_id"],
        "additionalProperties": false
    }),
    |this, input| {
        let operation = required_string(&input, "operation_id")?
            .parse::<fabric::OperationId>()
            .map_err(|_| "'operation_id' must be a canonical UUID".to_string())?;
        this.port
            .cancel(&operation)
            .await
            .map(|()| json!({"operation_id": operation, "cancelled": true}))
            .map_err(|error| error.to_string())
    }
);

robot_tool!(
    RobotSafeStopTool,
    "robot.safe_stop",
    "Request a governed safe stop for one robot device",
    PermissionLevel::L2,
    ConcurrencyClass::SideEffect,
    device_schema(),
    |this, input| {
        let device = required_string(&input, "device")?;
        this.port
            .safe_stop(&fabric::DeviceId(device.clone()))
            .await
            .map(|()| json!({"device": device, "stopped": true}))
            .map_err(|error| error.to_string())
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingPort {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl fabric::EmbodimentExecutionPort for RecordingPort {
        async fn observe(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Vec<fabric::EmbodiedObservation>, fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("observe".into());
            Ok(vec![])
        }
        async fn get_state(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Option<fabric::EmbodiedObservation>, fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("get_state".into());
            Ok(None)
        }
        async fn list_skills(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<Vec<fabric::SkillDescriptor>, fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("list_skills".into());
            Ok(vec![])
        }
        async fn execute_skill(
            &self,
            request: fabric::SkillRequest,
        ) -> Result<fabric::SkillResult, fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("execute_skill".into());
            Ok(fabric::SkillResult {
                operation_id: fabric::OperationId::new(),
                skill: request.skill,
                device: request.device,
                outcome: fabric::SkillOutcome::Succeeded,
                duration_ms: 1,
                evidence: vec![],
            })
        }
        async fn cancel(
            &self,
            _operation_id: &fabric::OperationId,
        ) -> Result<(), fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("cancel".into());
            Ok(())
        }
        async fn safe_stop(
            &self,
            _device: &fabric::DeviceId,
        ) -> Result<(), fabric::SkillDispatchError> {
            self.calls.lock().unwrap().push("safe_stop".into());
            Ok(())
        }
    }

    #[test]
    fn public_matrix_is_narrow_and_governed() {
        let port = Arc::new(RecordingPort::default());
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(RobotObserveTool::new(port.clone())),
            Box::new(RobotGetStateTool::new(port.clone())),
            Box::new(RobotListSkillsTool::new(port.clone())),
            Box::new(RobotExecuteSkillTool::new(port.clone())),
            Box::new(RobotCancelTool::new(port.clone())),
            Box::new(RobotSafeStopTool::new(port)),
        ];
        let expected = [
            (
                "robot.observe",
                PermissionLevel::L0,
                ConcurrencyClass::ReadOnly,
            ),
            (
                "robot.get_state",
                PermissionLevel::L0,
                ConcurrencyClass::ReadOnly,
            ),
            (
                "robot.list_skills",
                PermissionLevel::L0,
                ConcurrencyClass::ReadOnly,
            ),
            (
                "robot.execute_skill",
                PermissionLevel::L2,
                ConcurrencyClass::SideEffect,
            ),
            (
                "robot.cancel",
                PermissionLevel::L2,
                ConcurrencyClass::SideEffect,
            ),
            (
                "robot.safe_stop",
                PermissionLevel::L2,
                ConcurrencyClass::SideEffect,
            ),
        ];
        for (tool, (name, permission, concurrency)) in tools.iter().zip(expected) {
            assert_eq!(tool.name(), name);
            assert_eq!(tool.permission_level(), permission);
            assert_eq!(tool.concurrency_class(), concurrency);
            let schema = tool.input_schema().to_string();
            for forbidden in ["topic", "service", "joint", "bus"] {
                assert!(!schema.contains(forbidden), "{name} exposed {forbidden}");
            }
        }
    }

    #[tokio::test]
    async fn each_adapter_calls_exactly_one_matching_port_method() {
        let port = Arc::new(RecordingPort::default());
        let operation = fabric::OperationId::new();
        let tools: Vec<(Box<dyn Tool>, Value)> = vec![
            (
                Box::new(RobotObserveTool::new(port.clone())),
                json!({"device":"bot"}),
            ),
            (
                Box::new(RobotGetStateTool::new(port.clone())),
                json!({"device":"bot"}),
            ),
            (
                Box::new(RobotListSkillsTool::new(port.clone())),
                json!({"device":"bot"}),
            ),
            (
                Box::new(RobotExecuteSkillTool::new(port.clone())),
                json!({"device":"bot","skill":"navigate","parameters":{}}),
            ),
            (
                Box::new(RobotCancelTool::new(port.clone())),
                json!({"operation_id":operation.0.to_string()}),
            ),
            (
                Box::new(RobotSafeStopTool::new(port.clone())),
                json!({"device":"bot"}),
            ),
        ];
        let root = tempfile::tempdir().unwrap();
        let context = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: root.path().to_path_buf(),
            session_id: "robot-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        for (tool, input) in tools {
            assert!(
                !tool.execute(input, &context).await.is_error,
                "{}",
                tool.name()
            );
        }
        assert_eq!(
            *port.calls.lock().unwrap(),
            [
                "observe",
                "get_state",
                "list_skills",
                "execute_skill",
                "cancel",
                "safe_stop"
            ]
        );
    }
}
