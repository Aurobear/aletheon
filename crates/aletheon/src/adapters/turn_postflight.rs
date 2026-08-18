//! Concrete post-cognitive effects behind the Application settlement port.

use std::sync::Arc;

use async_trait::async_trait;

pub struct HostTurnPostEffects {
    pub self_policy: Arc<dyn crate::adapters::conscious::self_policy::SelfPolicyPort>,
    pub memory_gateway: Arc<mnemosyne::MemoryGatewayService>,
    pub principal: contracts::PrincipalId,
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: std::path::PathBuf,
    pub assistant_item_id: String,
    pub lifecycle: Arc<crate::daemon::turn_lifecycle_adapter::TurnLifecycleAdapter>,
    pub lifecycle_context: crate::daemon::turn_lifecycle_adapter::TurnLifecycleContext,
    pub cancel: tokio_util::sync::CancellationToken,
}

#[async_trait]
impl application::turn::post_turn::TurnPostEffectsPort for HostTurnPostEffects {
    async fn complete_policy(
        &self,
        turn_count: usize,
        input: &str,
        result: &contracts::TurnResult,
        succeeded: bool,
    ) {
        crate::adapters::conscious::self_policy::complete_turn(
            self.self_policy.as_ref(),
            turn_count,
            input,
            &result.output,
            &result.stop,
            succeeded,
        )
        .await;
    }

    async fn observe_assistant(&self, output: &str) {
        if let Err(error) = adapters_gbrain::context_memory::observe_native_assistant(
            self.memory_gateway.as_ref(),
            &self.principal,
            &self.session_id,
            &self.turn_id,
            &self.working_dir,
            &self.assistant_item_id,
            output,
        )
        .await
        {
            tracing::warn!(%error, "native assistant memory observation degraded");
        }
    }

    async fn after_terminal(&self, succeeded: bool) -> anyhow::Result<()> {
        self.lifecycle
            .after_turn(&self.lifecycle_context, succeeded, &self.cancel)
            .await
    }
}
