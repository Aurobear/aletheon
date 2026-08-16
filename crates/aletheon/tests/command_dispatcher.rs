use std::sync::{Arc, Mutex};

use ::contracts::contract::command::{
    ClientCommand, ClientIntent, ClientSurface, CommandId, CommandOutputV1, ExecuteShellIntent,
    StatusSummaryV1, SubmitPromptIntent,
};
use ::contracts::permission::HostPermissionMode;
use ::contracts::{PrincipalId, WorkspacePolicy};
use application::command_dispatcher::{CommandDispatcher, CommandOutput, CommandUseCases};
use async_trait::async_trait;

#[derive(Default)]
struct RecordingUseCases {
    calls: Mutex<Vec<CommandId>>,
}

#[async_trait]
impl CommandUseCases for RecordingUseCases {
    async fn submit_prompt(
        &self,
        intent: &ClientIntent,
        _prompt: &SubmitPromptIntent,
    ) -> anyhow::Result<CommandOutput> {
        self.calls.lock().unwrap().push(CommandId::SubmitPrompt);
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            CommandOutputV1::PromptAccepted,
        ))
    }

    async fn execute_shell(
        &self,
        intent: &ClientIntent,
        _shell: &ExecuteShellIntent,
    ) -> anyhow::Result<CommandOutput> {
        self.calls.lock().unwrap().push(CommandId::ExecuteShell);
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            CommandOutputV1::PromptAccepted,
        ))
    }

    async fn status(
        &self,
        intent: &ClientIntent,
        _status: &::contracts::contract::command::StatusIntent,
    ) -> anyhow::Result<CommandOutput> {
        self.calls.lock().unwrap().push(CommandId::Status);
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            CommandOutputV1::Status(StatusSummaryV1 {
                ready: true,
                summary: "ready".into(),
            }),
        ))
    }
}

#[tokio::test]
async fn u_input_004_shell_sigil_dispatches_a_typed_host_command_intent() {
    let use_cases = Arc::new(RecordingUseCases::default());
    let dispatcher = CommandDispatcher::new(use_cases.clone());
    let intent = ClientIntent::v1(
        ClientSurface::Tui,
        PrincipalId("owner".into()),
        "shell:1",
        ClientCommand::ExecuteShell(ExecuteShellIntent {
            command: "printf governed".into(),
            session_id: None,
            workspace: WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![]).unwrap(),
            permission_mode: HostPermissionMode::Safe,
        }),
    );

    let output = dispatcher.dispatch(intent).await.unwrap();
    assert!(matches!(output.output, CommandOutputV1::PromptAccepted));
    assert_eq!(
        *use_cases.calls.lock().unwrap(),
        vec![CommandId::ExecuteShell]
    );
}

fn prompt_intent(surface: ClientSurface) -> ClientIntent {
    ClientIntent::v1(
        surface,
        PrincipalId("owner".into()),
        format!("{surface:?}"),
        ClientCommand::SubmitPrompt(SubmitPromptIntent {
            content: "same request".into(),
            session_id: None,
            workspace: WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![]).unwrap(),
            requirements: vec![],
            task_kind: None,
            permission_mode: HostPermissionMode::Safe,
            execution_target: ::contracts::ExecutionTargetSelection::default(),
        }),
    )
}

#[tokio::test]
async fn a_entry_001_all_user_surfaces_dispatch_the_same_command_use_case() {
    let use_cases = Arc::new(RecordingUseCases::default());
    let dispatcher = CommandDispatcher::new(use_cases.clone());

    for surface in [
        ClientSurface::Cli,
        ClientSurface::Tui,
        ClientSurface::Gateway,
    ] {
        let output = dispatcher.dispatch(prompt_intent(surface)).await.unwrap();
        assert!(matches!(output.output, CommandOutputV1::PromptAccepted));
    }

    assert_eq!(
        *use_cases.calls.lock().unwrap(),
        vec![
            CommandId::SubmitPrompt,
            CommandId::SubmitPrompt,
            CommandId::SubmitPrompt,
        ]
    );
}

#[tokio::test]
async fn dispatcher_rejects_invalid_intent_before_calling_a_use_case() {
    let use_cases = Arc::new(RecordingUseCases::default());
    let dispatcher = CommandDispatcher::new(use_cases.clone());
    let mut intent = prompt_intent(ClientSurface::Cli);
    intent.correlation_id.clear();

    assert!(dispatcher.dispatch(intent).await.is_err());
    assert!(use_cases.calls.lock().unwrap().is_empty());
}

#[test]
fn prompt_completion_round_trips_through_the_typed_domain_payload() {
    use application::command_dispatcher::PromptCompletion;

    let completion = PromptCompletion {
        response: "hello world".into(),
        stop: ::contracts::TurnStop::Completed,
        failure: None,
        usage: ::contracts::InferenceUsage {
            total_input_tokens: Some(12),
            output_tokens: Some(3),
            uncached_input_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: Some(1),
            cache_telemetry: ::contracts::CacheTelemetry::default(),
        },
        metrics: ::contracts::TurnMetrics {
            tool_calls_made: 1,
            tool_errors: 0,
            provider_retries: 0,
            elapsed_ms: 42,
            iterations: 1,
            completed_normally: true,
        },
    };

    let envelope = CommandOutput::new(
        "prompt:roundtrip",
        CommandOutputV1::PromptCompleted(completion.clone()),
    );
    let wire = serde_json::to_value(&envelope).unwrap();
    assert_eq!(wire["schema_version"], 1);
    assert_eq!(wire["protocol"], "command_output");
    assert_eq!(wire["correlation_id"], "prompt:roundtrip");
    assert_eq!(wire["output"]["kind"], "prompt_completed");
    assert_eq!(wire["output"]["payload"]["response"], "hello world");
    assert_eq!(wire["output"]["payload"]["usage"]["total_input_tokens"], 12);

    let reparsed: CommandOutput = serde_json::from_value(wire).unwrap();
    assert_eq!(reparsed, envelope);
    assert_eq!(
        reparsed.into_v1().unwrap(),
        CommandOutputV1::PromptCompleted(completion)
    );
}

#[test]
fn prompt_completion_rejects_opaque_non_matching_json() {
    assert!(
        application::command_dispatcher::prompt_completion_from_rpc_result(serde_json::json!({
            "unexpected": "shape"
        }))
        .is_err()
    );
}

#[test]
fn command_output_schema_mismatch_is_actionable() {
    let output = CommandOutput {
        protocol: ::contracts::contract::command::CommandOutputProtocol::CommandOutput,
        schema_version: 99,
        correlation_id: "status:1".into(),
        output: CommandOutputV1::PromptAccepted,
    };
    assert_eq!(
        output.validate().unwrap_err().to_string(),
        "unsupported command output schema 99; supported schema is 1"
    );
}
