//! Canonical application entry for typed user commands.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::contract::command::{
    ClientCommand, ClientIntent, ExecuteShellIntent, StatusIntent, SubmitPromptIntent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutput {
    PromptAccepted {
        correlation_id: String,
    },
    /// Completed prompt result projected by a synchronous host adapter.
    ///
    /// `result` is the application result object, not a JSON-RPC envelope;
    /// presentation transports remain responsible for their own framing.
    PromptCompleted {
        correlation_id: String,
        result: serde_json::Value,
    },
    Status {
        ready: bool,
        summary: String,
    },
    /// Full status projection retained for compatibility clients while the
    /// versioned read model is introduced by the later Session nodes.
    StatusProjected {
        correlation_id: String,
        result: serde_json::Value,
    },
    Rejected {
        code: i64,
        message: String,
    },
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
