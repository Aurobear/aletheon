//! Chat capability: wraps [`ChannelTurnExecutor`] and executes the plain
//! chat turn for [`Intent::Chat`].
//!
//! An optional [`ChatPreprocessor`] hook runs before the turn executor. The
//! only current implementation is [`super::external_read::ExternalReadPreprocessor`],
//! wired in only when a external-source integration is configured — this handler
//! itself carries no domain knowledge of a concrete provider.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::channel::{InboundMessage, MessageContent, OutboundMessage};
use fabric::contract::command::{ClientCommand, ClientIntent, ClientSurface, SubmitPromptIntent};
use fabric::permission::HostPermissionMode;
use fabric::{PrincipalId, SessionId, WorkspacePolicy};

use crate::dispatcher::ChannelTurnExecutor;
use crate::effect::OutboundEffect;
use crate::handlers::external_read::{ChatPreprocess, ChatPreprocessor};
use crate::intent::Intent;
use crate::registry::{CapabilityHandler, HandlerContext, IntentKind};

pub struct ChatHandler {
    turn_executor: Arc<dyn ChannelTurnExecutor>,
    preprocessor: Option<Arc<dyn ChatPreprocessor>>,
    workspace: WorkspacePolicy,
}

impl ChatHandler {
    pub fn new(
        turn_executor: Arc<dyn ChannelTurnExecutor>,
        preprocessor: Option<Arc<dyn ChatPreprocessor>>,
        workspace: WorkspacePolicy,
    ) -> Self {
        Self {
            turn_executor,
            preprocessor,
            workspace,
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

        let client_intent = ClientIntent::v1(
            ClientSurface::Gateway,
            PrincipalId(principal.to_owned()),
            ctx.correlation_id.clone(),
            ClientCommand::SubmitPrompt(SubmitPromptIntent {
                content: query,
                session_id: Some(SessionId(principal.to_owned())),
                workspace: self.workspace.clone(),
                requirements: Vec::new(),
                task_kind: None,
                permission_mode: HostPermissionMode::Safe,
            }),
        );
        let reply = self.turn_executor.execute(&client_intent).await?;
        Ok(vec![Self::reply(ctx, reply)])
    }
}
