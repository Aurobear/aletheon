use super::*;

impl ToolRunnerWithGuard {
    /// Re-authorize and audit a read-only cache hit without executing the
    /// underlying tool. Callers must fail open to ordinary execution when the
    /// current policy, loop detector, output guard, or audit rejects the hit.
    pub async fn record_read_only_cache_hit(
        &mut self,
        tool: &dyn Tool,
        input: &serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
        result: &ToolResult,
    ) -> std::result::Result<::contracts::AuditEventId, ToolError> {
        if tool.permission_level() != PermissionLevel::L0 {
            return Err(ToolError::PolicyDenied {
                reason: "only L0 tools may serve cached results".into(),
            });
        }
        let audit_id = ::contracts::AuditEventId::new();
        let start = self.clock.mono_now();
        let unrestricted = ctx
            .approval_authority
            .as_ref()
            .map(|authority| authority.permission_mode.is_full())
            .unwrap_or(false);
        match self.check_policy(tool.name(), input, unrestricted) {
            PolicyVerdict::Allow => {}
            PolicyVerdict::Deny { reason } | PolicyVerdict::RequireApproval { reason } => {
                return Err(ToolError::PolicyDenied { reason });
            }
        }

        let loop_verdict = self.loop_detector.pre_check(tool.name(), input, turn_id);
        match &loop_verdict {
            LoopVerdict::Allow | LoopVerdict::Warn { .. } => {}
            LoopVerdict::Block { reason, suggestion } => {
                return Err(ToolError::LoopBlocked {
                    reason: format!("{reason}. {suggestion}"),
                });
            }
            LoopVerdict::Escalate { reason } => {
                return Err(ToolError::EscalateToHuman {
                    reason: reason.clone(),
                });
            }
            LoopVerdict::InterruptTurn { reason, .. } => {
                return Err(ToolError::InterruptTurn {
                    reason: reason.clone(),
                });
            }
        }
        self.output_guardrail
            .validate(result)
            .await
            .map_err(|error| ToolError::OutputRejected(format!("{error:?}")))?;
        self.loop_detector
            .post_check(tool.name(), input, result, turn_id);
        self.log_audit(
            audit_id,
            tool.name(),
            input,
            tool.permission_level(),
            turn_id,
            &ctx.session_id,
            Some(result),
            &start,
            &format!("cache_hit:{loop_verdict:?}"),
        )
        .await
        .map_err(|error| ToolError::AuditFailed(error.to_string()))?;
        Ok(audit_id)
    }
}
