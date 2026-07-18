//! Chat capability: wraps [`ChannelTurnExecutor`] and executes the plain
//! chat turn for [`Intent::Chat`].
//!
//! An optional [`ChatPreprocessor`] hook runs before the turn executor. The
//! only current implementation is [`super::google_read::GoogleReadPreprocessor`],
//! wired in only when a Google integration is configured — this handler
//! itself carries no domain knowledge of Google.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::channel::{InboundMessage, MessageContent, OutboundMessage};

use crate::dispatcher::ChannelTurnExecutor;
use crate::effect::OutboundEffect;
use crate::handlers::google_read::{ChatPreprocess, ChatPreprocessor};
use crate::intent::Intent;
use crate::registry::{CapabilityHandler, HandlerContext, IntentKind};

pub struct ChatHandler {
    turn_executor: Arc<dyn ChannelTurnExecutor>,
    preprocessor: Option<Arc<dyn ChatPreprocessor>>,
}

impl ChatHandler {
    pub fn new(
        turn_executor: Arc<dyn ChannelTurnExecutor>,
        preprocessor: Option<Arc<dyn ChatPreprocessor>>,
    ) -> Self {
        Self {
            turn_executor,
            preprocessor,
        }
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
        let principal = ctx.principal.as_str();

        let query = if let Some(preprocessor) = &self.preprocessor {
            match preprocessor.preprocess(principal, text).await? {
                ChatPreprocess::Reply(text) => return Ok(vec![Self::reply(ctx, text)]),
                ChatPreprocess::Rewrite(rewritten) => rewritten,
                ChatPreprocess::Passthrough => text.clone(),
            }
        } else {
            text.clone()
        };

        let reply = self
            .turn_executor
            .execute(principal, &query, &ctx.correlation_id)
            .await?;
        Ok(vec![Self::reply(ctx, reply)])
    }
}
