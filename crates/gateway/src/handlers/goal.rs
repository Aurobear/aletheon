//! Goal-command capability: wraps [`ChannelGoalExecutor`] for the M2 Goal
//! lifecycle commands (`/goal`, `/goals`, `/status`, `/pause`, `/resume`,
//! `/cancel`), plus `/edit` which drives the `ActivateGoal`
//! [`ApprovalResolver`]'s revise path.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::channel::InboundMessage;
use fabric::{ApprovalCategory, GoalId};

use crate::dispatcher::ChannelGoalExecutor;
use crate::effect::OutboundEffect;
use crate::intent::Intent;
use crate::registry::{
    ApprovalResolverRegistry, CapabilityHandler, HandlerContext, IntentKind,
};

pub struct GoalHandler {
    executor: Arc<dyn ChannelGoalExecutor>,
    approval_resolvers: Arc<ApprovalResolverRegistry>,
}

impl GoalHandler {
    pub fn new(
        executor: Arc<dyn ChannelGoalExecutor>,
        approval_resolvers: Arc<ApprovalResolverRegistry>,
    ) -> Self {
        Self {
            executor,
            approval_resolvers,
        }
    }

    async fn execute_goal_command(
        &self,
        owner: &str,
        command: &str,
        args: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        if command == "/edit" {
            let (id, intent) = args
                .trim()
                .split_once(char::is_whitespace)
                .ok_or_else(|| anyhow::anyhow!("usage: /edit <goal-id> <revised-intent>"))?;
            let id = id
                .parse::<i64>()
                .map(GoalId)
                .map_err(|_| anyhow::anyhow!("usage: /edit <goal-id> <revised-intent>"))?;
            if intent.trim().is_empty() {
                anyhow::bail!("usage: /edit <goal-id> <revised-intent>");
            }
            let resolver = self
                .approval_resolvers
                .resolve_category_only(ApprovalCategory::ActivateGoal)
                .ok_or_else(|| anyhow::anyhow!("Gmail draft executor is not configured"))?;
            let approval = resolver
                .revise_draft(owner, id, intent.trim(), now_ms)
                .await?;
            return Ok(format!(
                "Goal {} revised; fresh confirmation {} is pending.",
                id.0, approval.id
            ));
        }
        if command == "/goal" {
            if args.trim().is_empty() {
                anyhow::bail!("usage: /goal <intent>");
            }
            let goal = self.executor.create_draft(owner, args.trim()).await?;
            return Ok(format!(
                "Goal {} created as draft: {}",
                goal.id.0, goal.spec.original_intent
            ));
        }
        if command == "/goals" {
            let goals = self.executor.list(owner).await?;
            if goals.is_empty() {
                return Ok("No goals.".into());
            }
            return Ok(goals
                .iter()
                .map(|g| format!("{} {} {}", g.id.0, g.state, g.spec.original_intent))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let id = args
            .trim()
            .parse::<i64>()
            .map(GoalId)
            .map_err(|_| anyhow::anyhow!("usage: {command} <goal-id>"))?;
        let goal = match command {
            "/status" => self.executor.show(owner, id).await?,
            "/pause" => self.executor.pause(owner, id).await?,
            "/resume" => self.executor.resume(owner, id).await?,
            "/cancel" => self.executor.cancel(owner, id).await?,
            _ => anyhow::bail!("unsupported goal command"),
        };
        Ok(format!("Goal {}: {}", goal.id.0, goal.state))
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
            .execute_goal_command(&ctx.principal, command, args, ctx.timestamp_ms)
            .await?;
        Ok(vec![OutboundEffect::Reply(fabric::channel::OutboundMessage {
            conversation_id: ctx.conversation_id.clone(),
            content: fabric::channel::MessageContent::Text { text: reply },
            actions: vec![],
            reply_to: Some(ctx.message_id.clone()),
            correlation_id: ctx.correlation_id.clone(),
        })])
    }
}
