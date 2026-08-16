//! Canonical application entry for typed user commands.

use std::sync::Arc;

use ::contracts::contract::command::{
    ClientCommand, ClientIntent, CommandOutputEnvelopeV1, ExecuteShellIntent, StatusIntent,
    SubmitPromptIntent,
};
use async_trait::async_trait;

pub use ::contracts::contract::command::{CommandOutputV1, PromptCompletionV1 as PromptCompletion};

/// The application returns the same versioned output contract consumed by all
/// clients. JSON-RPC framing is added only by the host adapter.
pub type CommandOutput = CommandOutputEnvelopeV1;

/// The only V0 prompt-result migration adapter. Remove after legacy direct
/// orchestrator callers have moved to `CommandOutputEnvelopeV1`.
pub fn prompt_completion_from_rpc_result(
    result: serde_json::Value,
) -> anyhow::Result<PromptCompletion> {
    serde_json::from_value(result).map_err(anyhow::Error::from)
}

#[async_trait]
pub trait CommandUseCases: Send + Sync {
    async fn submit_prompt(
        &self,
        intent: &ClientIntent,
        prompt: &SubmitPromptIntent,
    ) -> anyhow::Result<CommandOutput>;

    async fn execute_shell(
        &self,
        intent: &ClientIntent,
        shell: &ExecuteShellIntent,
    ) -> anyhow::Result<CommandOutput>;

    async fn status(
        &self,
        intent: &ClientIntent,
        status: &StatusIntent,
    ) -> anyhow::Result<CommandOutput>;
}

/// The sole application handler for [`ClientIntent`].
///
/// Presentation adapters may classify and validate their own syntax, but they
/// must not select a domain use case directly. This dispatcher validates the
/// neutral envelope and performs the only command-to-use-case match.
pub struct CommandDispatcher {
    use_cases: Arc<dyn CommandUseCases>,
}

impl CommandDispatcher {
    pub fn new(use_cases: Arc<dyn CommandUseCases>) -> Self {
        Self { use_cases }
    }

    pub async fn dispatch(&self, intent: ClientIntent) -> anyhow::Result<CommandOutput> {
        intent.validate()?;
        match &intent.command {
            ClientCommand::SubmitPrompt(prompt) => {
                self.use_cases.submit_prompt(&intent, prompt).await
            }
            ClientCommand::ExecuteShell(shell) => {
                self.use_cases.execute_shell(&intent, shell).await
            }
            ClientCommand::Status(status) => self.use_cases.status(&intent, status).await,
        }
    }
}
