//! Chat capability: routes the plain chat turn through a typed Application port.

use std::sync::Arc;

use crate::channel::{InboundMessage, MessageContent, OutboundMessage};
use ::contracts::PrincipalId;
use async_trait::async_trait;

use crate::effect::OutboundEffect;
use crate::intent::Intent;
use crate::ports::{ChannelChatCommandPort, ChannelChatRequest};
use crate::registry::{CapabilityHandler, HandlerContext, IntentKind};

pub struct ChatHandler {
    executor: Arc<dyn ChannelChatCommandPort>,
}

impl ChatHandler {
    pub fn new(executor: Arc<dyn ChannelChatCommandPort>) -> Self {
        Self { executor }
    }

    fn reply(ctx: &HandlerContext, text: String) -> OutboundEffect {
        OutboundEffect::Reply(OutboundMessage {
            conversation_id: ctx.conversation_id.clone(),
            content: MessageContent::Text { text },
            actions: vec![],
            reply_to: Some(ctx.message_id.clone()),
            correlation_id: ctx.correlation_id.clone(),
        })
    }
}

#[async_trait]
impl CapabilityHandler for ChatHandler {
    fn intent_kind(&self) -> IntentKind {
        IntentKind::Chat
    }

    async fn handle(
        &self,
        ctx: &HandlerContext,
        _inbound: &InboundMessage,
        intent: &Intent,
    ) -> anyhow::Result<Vec<OutboundEffect>> {
        let Intent::Chat(text) = intent else {
            return Ok(vec![]);
        };
        let reply = self
            .executor
            .execute(&ChannelChatRequest {
                principal: PrincipalId(ctx.principal.clone()),
                correlation_id: ctx.correlation_id.clone(),
                text: text.clone(),
            })
            .await?;
        Ok(vec![Self::reply(ctx, reply)])
    }
}
