//! Durable approval projection and daemon notification for an active Turn.

use std::sync::Arc;

pub async fn project(
    sessions: &runtime::session_service::SessionService,
    notice: application::turn::ports::ApprovalNotice,
    notification: Option<&Arc<dyn application::turn::service::TurnNotificationPort>>,
) {
    if let Err(error) = sessions
        .append_protocol_approval_event(
            &notice.session_id,
            notice.turn_id,
            notice.approval_id.clone(),
            notice.tool.clone(),
            notice.action_summary.clone(),
            notice.risk_level.clone(),
            notice.detail.clone(),
            notice.scope_subject.clone(),
        )
        .await
    {
        tracing::warn!(
            %error,
            session = %notice.session_id.0,
            turn = %notice.turn_id.0,
            approval = %notice.approval_id,
            "failed to persist approval request for reconnect"
        );
    }
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "approval_request",
        "params": {
            "approval_id": notice.approval_id,
            "session": notice.session_id.0,
            "turn": notice.turn_id.0,
            "tool": notice.tool,
            "action_summary": notice.action_summary,
            "risk_level": notice.risk_level,
            "detail": notice.detail,
            "scope_subject": notice.scope_subject,
        }
    });
    match notification {
        Some(port) if port.send(payload.to_string()).await.is_err() => {
            tracing::warn!("approval request notification client disconnected");
        }
        None => tracing::warn!("no Turn notification port; approval will timeout fail-closed"),
        Some(_) => {}
    }
}
