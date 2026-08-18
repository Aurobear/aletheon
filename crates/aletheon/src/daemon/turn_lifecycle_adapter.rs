//! Host effects for Runtime Turn lifecycle contributor dispatch.

use std::sync::Arc;

use runtime::event_projection::CanonicalEventBus;
use tokio_util::sync::CancellationToken;

pub struct TurnLifecycleAdapter {
    registry: Arc<runtime::lifecycle_contributors::LifecycleRegistry>,
    enabled: bool,
    event_bus: Option<Arc<CanonicalEventBus>>,
}

#[derive(Clone)]
pub struct TurnLifecycleContext {
    pub principal_id: contracts::PrincipalId,
    pub thread_id: contracts::ThreadId,
    pub turn_id: Option<contracts::TurnId>,
    pub session_id: String,
}

impl TurnLifecycleAdapter {
    pub fn new(
        registry: Arc<runtime::lifecycle_contributors::LifecycleRegistry>,
        enabled: bool,
        event_bus: Option<Arc<CanonicalEventBus>>,
    ) -> Self {
        Self {
            registry,
            enabled,
            event_bus,
        }
    }

    pub async fn dispatch(
        &self,
        input: runtime::lifecycle_contributors::LifecycleInput,
        cancel: &CancellationToken,
    ) -> anyhow::Result<runtime::lifecycle_contributors::LifecycleDispatch> {
        use runtime::lifecycle_contributors::LifecycleEffect;

        let target = format!("thread:{}", input.thread_id.0);
        let dispatch = self
            .registry
            .dispatch_if_enabled(self.enabled, input)
            .await?;
        for effect in &dispatch.effects {
            match effect {
                LifecycleEffect::EmitEvent { schema, payload } => {
                    if let Some(bus) = &self.event_bus {
                        publish_requested_lifecycle_event(bus, schema, &target, payload.clone())
                            .await?;
                    }
                }
                LifecycleEffect::RequestCancellation { .. } => cancel.cancel(),
                _ => {}
            }
            if let Some(bus) = &self.event_bus {
                if let Err(error) = bus
                    .publish_event(
                        contracts::SchemaId::from("aletheon.event.lifecycle_effect_applied/v1"),
                        &target,
                        serde_json::json!({"effect": format!("{effect:?}")}),
                    )
                    .await
                {
                    tracing::warn!(%error, "best-effort lifecycle effect audit event was not published");
                }
            }
        }
        Ok(dispatch)
    }

    async fn dispatch_phase(
        &self,
        context: &TurnLifecycleContext,
        phase: runtime::lifecycle_contributors::LifecyclePhase,
        detail: serde_json::Value,
        cancel: &CancellationToken,
    ) -> anyhow::Result<runtime::lifecycle_contributors::LifecycleDispatch> {
        self.dispatch(
            runtime::lifecycle_contributors::LifecycleInput {
                phase,
                principal_id: context.principal_id.clone(),
                thread_id: context.thread_id.clone(),
                turn_id: context.turn_id,
                session_id: context.session_id.clone(),
                detail,
            },
            cancel,
        )
        .await
    }

    pub async fn before_turn(
        &self,
        context: &TurnLifecycleContext,
        message: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<runtime::lifecycle_contributors::LifecycleDispatch> {
        self.dispatch_phase(
            context,
            runtime::lifecycle_contributors::LifecyclePhase::BeforeTurnInput,
            serde_json::json!({"message": message}),
            cancel,
        )
        .await
    }

    pub async fn before_tool_batch(
        &self,
        context: &TurnLifecycleContext,
        tool_count: usize,
        prompt_profile: serde_json::Value,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        use runtime::lifecycle_contributors::LifecycleEffect;
        let dispatch = self
            .dispatch_phase(
                context,
                runtime::lifecycle_contributors::LifecyclePhase::BeforeToolBatch,
                serde_json::json!({
                    "tool_count": tool_count,
                    "prompt_construction_profile": prompt_profile,
                }),
                cancel,
            )
            .await?;
        if let Some(reason) = dispatch.effects.iter().find_map(|effect| match effect {
            LifecycleEffect::RejectInput { reason } => Some(reason),
            _ => None,
        }) {
            anyhow::bail!("lifecycle contributor rejected tool batch: {reason}");
        }
        Ok(())
    }

    pub async fn after_tool(
        &self,
        context: &TurnLifecycleContext,
        tool: &application::turn::evidence::ToolTerminalEvidence,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        self.dispatch_phase(
            context,
            runtime::lifecycle_contributors::LifecyclePhase::AfterToolTerminal,
            serde_json::json!({
                "call_id": tool.call_id,
                "tool_name": tool.name,
                "is_error": tool.is_error,
            }),
            cancel,
        )
        .await?;
        Ok(())
    }

    pub async fn after_turn(
        &self,
        context: &TurnLifecycleContext,
        succeeded: bool,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        self.dispatch_phase(
            context,
            runtime::lifecycle_contributors::LifecyclePhase::AfterTurnTerminal,
            serde_json::json!({"succeeded": succeeded}),
            cancel,
        )
        .await?;
        Ok(())
    }

    pub async fn on_abort(
        &self,
        context: &TurnLifecycleContext,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        self.dispatch_phase(
            context,
            runtime::lifecycle_contributors::LifecyclePhase::OnAbort,
            serde_json::json!({"reason": "turn pipeline error"}),
            cancel,
        )
        .await?;
        Ok(())
    }
}

pub(crate) async fn publish_requested_lifecycle_event(
    bus: &CanonicalEventBus,
    schema: &str,
    target: &str,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    bus.publish_event(contracts::SchemaId::from(schema), target, payload)
        .await
        .map_err(|error| anyhow::anyhow!("requested lifecycle event publish failed: {error}"))
}
