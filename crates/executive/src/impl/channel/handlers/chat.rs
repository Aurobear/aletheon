//! Chat capability: wraps [`ChannelTurnExecutor`] and executes the plain
//! chat turn for [`Intent::Chat`].
//!
//! The Google-read preflight (account picker / `<trusted-google-account>`
//! wrap) is kept inline here exactly as it was in the god-object router —
//! extracting it into its own `GoogleReadHandler` capability is Phase 2.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::channel::{InboundMessage, MessageContent, OutboundMessage};

use crate::r#impl::channel::dispatcher::{ChannelTurnExecutor, GoogleChannelAccountDirectory};
use crate::r#impl::channel::effect::OutboundEffect;
use crate::r#impl::channel::intent::Intent;
use crate::r#impl::channel::registry::{CapabilityHandler, HandlerContext, IntentKind};
use crate::r#impl::channel::telegram;

pub struct ChatHandler {
    turn_executor: Arc<dyn ChannelTurnExecutor>,
    google_accounts: Option<Arc<dyn GoogleChannelAccountDirectory>>,
}

impl ChatHandler {
    pub fn new(
        turn_executor: Arc<dyn ChannelTurnExecutor>,
        google_accounts: Option<Arc<dyn GoogleChannelAccountDirectory>>,
    ) -> Self {
        Self {
            turn_executor,
            google_accounts,
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

        let mut selected_query = None;
        if ctx.channel == "telegram"
            && telegram::is_google_read_query(text)
            && self.google_accounts.is_some()
        {
            let labels = self
                .google_accounts
                .as_ref()
                .expect("checked above")
                .active_account_labels(principal)
                .await?;
            if labels.len() > 1 {
                return Ok(vec![Self::reply(
                    ctx,
                    telegram::account_choice_prompt(&labels),
                )]);
            } else if let Some(label) = labels.first() {
                selected_query = Some(telegram::selected_account_context(label, text));
            }
        }

        let query = selected_query.as_deref().unwrap_or(text.as_str());
        let reply = self
            .turn_executor
            .execute(principal, query, &ctx.correlation_id)
            .await?;
        Ok(vec![Self::reply(ctx, reply)])
    }
}
