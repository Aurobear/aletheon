use super::{ReActLoop, PLAN_MODE_MARKER};

impl ReActLoop {
    /// Enable/disable plan mode. Injected into user message, NOT system prompt.
    pub fn set_plan_mode(&mut self, enabled: bool) {
        self.plan_mode = enabled;
    }

    /// Queue a memory update for the next user message.
    pub fn queue_memory_update(&mut self, update: String) {
        self.pending_memory.push(update);
    }

    /// Set the Dasein context provider for per-turn SelfField state injection.
    pub fn set_dasein_context_provider(
        &mut self,
        provider: Box<dyn Fn() -> Option<String> + Send + Sync>,
    ) {
        self.dasein_ctx_provider = Some(provider);
    }

    /// Compose user message with mid-session injections.
    /// Changes go here, NOT into system prompt, to preserve cache stability.
    pub fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        if self.plan_mode {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        if !self.pending_memory.is_empty() {
            let updates = self
                .pending_memory
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        let goal_ctx = self.goal_tracker.get_context();
        if !goal_ctx.is_empty() {
            parts.push(format!("<goal-context>\n{}\n</goal-context>", goal_ctx));
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }

    /// Compose user message with mid-session injections plus DaseinContext.
    ///
    /// The DaseinContext is injected as a `<dasein-state>` XML block in the
    /// user message (not the system prompt) to preserve cache stability.
    /// This lets the LLM perceive the system's existential state --
    /// mood, temporal flow, involvement network, and care structure.
    pub fn compose_user_message_with_dasein(
        &self,
        input: &str,
        dasein_context: Option<&str>,
    ) -> String {
        let mut parts = Vec::new();

        if self.plan_mode {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        if !self.pending_memory.is_empty() {
            let updates = self
                .pending_memory
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        let goal_ctx = self.goal_tracker.get_context();
        if !goal_ctx.is_empty() {
            parts.push(format!("<goal-context>\n{}\n</goal-context>", goal_ctx));
        }

        if let Some(ctx) = dasein_context {
            if !ctx.is_empty() {
                parts.push(format!("<dasein-state>\n{}\n</dasein-state>", ctx));
            }
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }
}
