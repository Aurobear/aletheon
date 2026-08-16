//! Greeting capability: responds to `/start` without invoking any LLM turn.
//!
//! The router uses the handler's bounded reply effect when building the
//! durable outbound message; no second greeting policy exists in the router.

use crate::channel::{InboundMessage, MessageContent, OutboundMessage};
use async_trait::async_trait;

use crate::effect::OutboundEffect;
use crate::intent::Intent;
use crate::registry::{CapabilityHandler, HandlerContext, IntentKind};

pub struct GreetingHandler;

#[async_trait]
impl CapabilityHandler for GreetingHandler {
    fn intent_kind(&self) -> IntentKind {
        IntentKind::Greeting
    }

    async fn handle(
        &self,
        ctx: &HandlerContext,
        _inbound: &InboundMessage,
        _intent: &Intent,
    ) -> anyhow::Result<Vec<OutboundEffect>> {
        Ok(vec![OutboundEffect::Reply(OutboundMessage {
            conversation_id: ctx.conversation_id.clone(),
            content: MessageContent::Text {
                text: "Hello! I am Aletheon. How can I help you today?".into(),
            },
            actions: vec![],
            reply_to: Some(ctx.message_id.clone()),
            correlation_id: ctx.correlation_id.clone(),
        })])
    }
}
