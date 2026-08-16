//! Typed extension points around the minimal Harness loop.
//!
//! Hooks observe typed lifecycle state. They never inspect acceptance prompts
//! or own Session persistence, and the core loop remains the sole event writer.

use std::sync::Arc;

use async_trait::async_trait;

use contracts::ToolDefinition;

use super::driver::{HarnessPortError, HarnessToolInvocation, HarnessToolOutput};
use super::session_log::TurnEndReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessStepContext {
    pub turn: u64,
    pub step: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessPreStepDecision {
    Continue,
    End(TurnEndReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessRequestErrorDecision {
    Fail,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessTurnStoppingDecision {
    Stop,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessToolPreDecision {
    Execute,
    Return(HarnessToolOutput),
}

#[async_trait]
pub trait HarnessLifecycleHooks: Send + Sync {
    async fn pre_step(
        &self,
        _context: HarnessStepContext,
    ) -> Result<HarnessPreStepDecision, HarnessPortError> {
        Ok(HarnessPreStepDecision::Continue)
    }

    async fn prepare_request(
        &self,
        _context: HarnessStepContext,
        _system_prompt: &mut String,
        _tool_definitions: &mut Vec<ToolDefinition>,
    ) -> Result<(), HarnessPortError> {
        Ok(())
    }

    async fn request_error(
        &self,
        _context: HarnessStepContext,
        _attempt: u32,
        _error: &HarnessPortError,
    ) -> HarnessRequestErrorDecision {
        HarnessRequestErrorDecision::Fail
    }

    async fn turn_stopping(
        &self,
        _context: HarnessStepContext,
    ) -> Result<HarnessTurnStoppingDecision, HarnessPortError> {
        Ok(HarnessTurnStoppingDecision::Stop)
    }

    async fn tool_pre(
        &self,
        _context: HarnessStepContext,
        _invocation: &HarnessToolInvocation,
    ) -> HarnessToolPreDecision {
        HarnessToolPreDecision::Execute
    }

    async fn tool_post(
        &self,
        _context: HarnessStepContext,
        _invocation: &HarnessToolInvocation,
        _output: &mut HarnessToolOutput,
    ) {
    }
}

#[derive(Debug, Default)]
pub struct NoopHarnessLifecycleHooks;

impl HarnessLifecycleHooks for NoopHarnessLifecycleHooks {}

#[derive(Debug)]
pub struct StepBudgetHook {
    max_steps: u32,
}

impl StepBudgetHook {
    pub fn new(max_steps: u32) -> Self {
        Self { max_steps }
    }
}

#[async_trait]
impl HarnessLifecycleHooks for StepBudgetHook {
    async fn pre_step(
        &self,
        context: HarnessStepContext,
    ) -> Result<HarnessPreStepDecision, HarnessPortError> {
        if context.step >= self.max_steps {
            Ok(HarnessPreStepDecision::End(TurnEndReason::Error {
                code: "step_budget_exhausted".into(),
                message: format!("step budget {} exhausted", self.max_steps),
            }))
        } else {
            Ok(HarnessPreStepDecision::Continue)
        }
    }
}

/// Ordered plugin chain. Preparation/pre hooks run registration order; post
/// hooks run reverse order so wrappers unwind deterministically.
#[derive(Default)]
pub struct HarnessHookChain {
    hooks: Vec<Arc<dyn HarnessLifecycleHooks>>,
}

impl HarnessHookChain {
    pub fn new(hooks: Vec<Arc<dyn HarnessLifecycleHooks>>) -> Self {
        Self { hooks }
    }

    pub fn push(&mut self, hook: Arc<dyn HarnessLifecycleHooks>) {
        self.hooks.push(hook);
    }
}

#[async_trait]
impl HarnessLifecycleHooks for HarnessHookChain {
    async fn pre_step(
        &self,
        context: HarnessStepContext,
    ) -> Result<HarnessPreStepDecision, HarnessPortError> {
        for hook in &self.hooks {
            let decision = hook.pre_step(context).await?;
            if !matches!(decision, HarnessPreStepDecision::Continue) {
                return Ok(decision);
            }
        }
        Ok(HarnessPreStepDecision::Continue)
    }

    async fn prepare_request(
        &self,
        context: HarnessStepContext,
        system_prompt: &mut String,
        tool_definitions: &mut Vec<ToolDefinition>,
    ) -> Result<(), HarnessPortError> {
        for hook in &self.hooks {
            hook.prepare_request(context, system_prompt, tool_definitions)
                .await?;
        }
        Ok(())
    }

    async fn request_error(
        &self,
        context: HarnessStepContext,
        attempt: u32,
        error: &HarnessPortError,
    ) -> HarnessRequestErrorDecision {
        for hook in &self.hooks {
            if hook.request_error(context, attempt, error).await
                == HarnessRequestErrorDecision::Retry
            {
                return HarnessRequestErrorDecision::Retry;
            }
        }
        HarnessRequestErrorDecision::Fail
    }

    async fn turn_stopping(
        &self,
        context: HarnessStepContext,
    ) -> Result<HarnessTurnStoppingDecision, HarnessPortError> {
        for hook in &self.hooks {
            if hook.turn_stopping(context).await? == HarnessTurnStoppingDecision::Continue {
                return Ok(HarnessTurnStoppingDecision::Continue);
            }
        }
        Ok(HarnessTurnStoppingDecision::Stop)
    }

    async fn tool_pre(
        &self,
        context: HarnessStepContext,
        invocation: &HarnessToolInvocation,
    ) -> HarnessToolPreDecision {
        for hook in &self.hooks {
            let decision = hook.tool_pre(context, invocation).await;
            if !matches!(decision, HarnessToolPreDecision::Execute) {
                return decision;
            }
        }
        HarnessToolPreDecision::Execute
    }

    async fn tool_post(
        &self,
        context: HarnessStepContext,
        invocation: &HarnessToolInvocation,
        output: &mut HarnessToolOutput,
    ) {
        for hook in self.hooks.iter().rev() {
            hook.tool_post(context, invocation, output).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_step_budget_allows_the_first_model_step() {
        let hook = StepBudgetHook::new(1);
        assert_eq!(
            hook.pre_step(HarnessStepContext { turn: 1, step: 0 })
                .await
                .unwrap(),
            HarnessPreStepDecision::Continue
        );
        assert!(matches!(
            hook.pre_step(HarnessStepContext { turn: 1, step: 1 })
                .await
                .unwrap(),
            HarnessPreStepDecision::End(TurnEndReason::Error { ref code, .. })
                if code == "step_budget_exhausted"
        ));
    }
}
