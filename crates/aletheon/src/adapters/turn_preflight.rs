//! Concrete pre-cognitive effects behind the Application Turn preflight port.

use std::sync::Arc;

use async_trait::async_trait;

pub struct HostTurnPreflight {
    pub lifecycle: Arc<crate::daemon::turn_lifecycle_adapter::TurnLifecycleAdapter>,
    pub lifecycle_context: crate::daemon::turn_lifecycle_adapter::TurnLifecycleContext,
    pub cancel: tokio_util::sync::CancellationToken,
    pub self_policy: Arc<dyn crate::adapters::conscious::self_policy::SelfPolicyPort>,
    pub storm: Arc<dyn application::turn::ports::StormStatePort>,
    pub sessions: Arc<runtime::session_service::SessionService>,
    pub hooks: Arc<dyn crate::adapters::hooks::TurnHookPort>,
}

#[async_trait]
impl application::turn::context::TurnPreflightPort for HostTurnPreflight {
    async fn prepare_input(
        &self,
        request: &contracts::TurnRequest,
        message: &str,
        session_id: &str,
        current_turn_count: usize,
    ) -> anyhow::Result<
        Result<
            application::turn::context::PreflightPreparedInput,
            application::turn::outcome::TurnPipelineRejection,
        >,
    > {
        let lifecycle_start = self
            .lifecycle
            .before_turn(&self.lifecycle_context, message, &self.cancel)
            .await?;
        let sandbox = match crate::adapters::conscious::self_policy::review_turn_intent(
            self.self_policy.as_ref(),
            message,
            session_id,
            request.context.workspace.cwd(),
            request.context.permission_profile.permits_filesystem_root(),
        )
        .await
        {
            Ok(sandbox) => sandbox,
            Err(rejection) => return Ok(Err(rejection)),
        };
        self.storm.reset().await;
        let effective = crate::host::session::lifecycle_context::persist_before_turn_fragments(
            self.sessions.as_ref(),
            &contracts::SessionId(request.context.thread_id.0.clone()),
            request.context.turn_id,
            &lifecycle_start.effects,
        )
        .await?;
        let effective = match crate::adapters::hooks::prepare_turn_input(
            self.hooks.as_ref(),
            message,
            session_id,
            current_turn_count,
            request.context.workspace.cwd(),
            request.context.repo_hooks_trusted,
            effective,
        )
        .await
        {
            Ok(message) => message,
            Err(rejection) => return Ok(Err(rejection)),
        };
        // Preserve the reviewed sandbox decision as authenticated request
        // state for the capability stage without exposing concrete policy
        // vocabulary to Application.
        Ok(Ok(application::turn::context::PreflightPreparedInput {
            effective_input: effective,
            sandbox,
        }))
    }
}
