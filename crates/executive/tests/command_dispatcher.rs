use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use executive::application::{CommandDispatcher, CommandOutput, CommandUseCases};
use fabric::contract::command::{
    ClientCommand, ClientIntent, ClientSurface, CommandId, SubmitPromptIntent,
};
use fabric::permission::HostPermissionMode;
use fabric::{PrincipalId, WorkspacePolicy};

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
        Ok(CommandOutput::PromptAccepted {
            correlation_id: intent.correlation_id.clone(),
        })
    }

    async fn status(&self, _intent: &ClientIntent) -> anyhow::Result<CommandOutput> {
        self.calls.lock().unwrap().push(CommandId::Status);
        Ok(CommandOutput::Status {
            ready: true,
            summary: "ready".into(),
        })
    }
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
        assert!(matches!(output, CommandOutput::PromptAccepted { .. }));
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
