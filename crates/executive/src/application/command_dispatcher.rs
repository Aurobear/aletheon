//! Canonical application entry for typed user commands.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::contract::command::{ClientCommand, ClientIntent, SubmitPromptIntent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutput {
    PromptAccepted { correlation_id: String },
    Status { ready: bool, summary: String },
}

#[async_trait]
pub trait CommandUseCases: Send + Sync {
    async fn submit_prompt(
        &self,
        intent: &ClientIntent,
        prompt: &SubmitPromptIntent,
    ) -> anyhow::Result<CommandOutput>;

    async fn status(&self, intent: &ClientIntent) -> anyhow::Result<CommandOutput>;
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
            ClientCommand::Status => self.use_cases.status(&intent).await,
        }
    }
}
