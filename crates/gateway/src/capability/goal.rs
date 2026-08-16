//! Goal-command capability: wraps the typed Goal Application port for the M2 Goal
//! lifecycle commands (`/goal`, `/goals`, `/status`, `/pause`, `/resume`,
//! `/cancel`), plus `/edit` which drives the `ActivateGoal`
//! `ApprovalResolver`'s revise path.

use std::sync::Arc;

use crate::channel::InboundMessage;
use crate::effect::OutboundEffect;
use crate::intent::Intent;
use crate::ports::ChannelGoalCommandPort;
use crate::registry::{CapabilityHandler, HandlerContext, IntentKind};
use async_trait::async_trait;

pub struct GoalHandler {
    executor: Arc<dyn ChannelGoalCommandPort>,
}

impl GoalHandler {
    pub fn new(executor: Arc<dyn ChannelGoalCommandPort>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl CapabilityHandler for GoalHandler {
    fn intent_kind(&self) -> IntentKind {
        IntentKind::GoalCommand
    }

    async fn handle(
        &self,
        ctx: &HandlerContext,
        _inbound: &InboundMessage,
        intent: &Intent,
    ) -> anyhow::Result<Vec<OutboundEffect>> {
        let Intent::GoalCommand { command, args } = intent else {
            return Ok(vec![]);
        };
        let reply = self
            .executor
            .execute(&ctx.principal, command, args, ctx.timestamp_ms)
            .await?;
        Ok(vec![OutboundEffect::Reply(
            crate::channel::OutboundMessage {
                conversation_id: ctx.conversation_id.clone(),
                content: crate::channel::MessageContent::Text { text: reply },
                actions: vec![],
                reply_to: Some(ctx.message_id.clone()),
                correlation_id: ctx.correlation_id.clone(),
            },
        )])
    }
}
