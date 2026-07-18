//! Pure outbound-rendering helpers for the channel router.

use fabric::channel::{ActionType, ConversationId, MessageContent, OutboundMessage, UserAction};
use fabric::{ApprovalCategory, ApprovalSnapshot};

pub fn render_approval_notification(
    conversation_id: ConversationId,
    approval: &ApprovalSnapshot,
) -> OutboundMessage {
    if approval.category == ApprovalCategory::ActivateGoal {
        let text = format!(
            "Goal {} requires owner confirmation.\nRisk: {:?}\nExpires: {} ms\n{}",
            approval.goal_id.0, approval.risk, approval.expires_at_ms, approval.summary
        );
        let action = |suffix: &str, label: &str, action_type| UserAction {
            action_id: format!("{}:{suffix}", approval.id),
            label: label.into(),
            action_type,
        };
        return OutboundMessage {
            conversation_id,
            content: MessageContent::Text { text },
            actions: vec![
                action("confirm", "Confirm", ActionType::Approve),
                action("edit", "Edit", ActionType::Callback),
                action("reject", "Reject", ActionType::Reject),
            ],
            reply_to: None,
            correlation_id: format!("approval:{}", approval.id),
        };
    }
    let changed_files = approval
        .subject
        .attributes
        .get("changed_file_count")
        .map(String::as_str)
        .unwrap_or("unknown");
    let verification = approval
        .subject
        .attributes
        .get("verification_summary")
        .map(String::as_str)
        .unwrap_or("required checks passed");
    let text = format!(
        "Goal {} requires approval.\nChanged files: {}\nVerification: {}\nRisk: {:?}\nExpires: {} ms\n{}",
        approval.goal_id.0,
        changed_files,
        verification,
        approval.risk,
        approval.expires_at_ms,
        approval.summary
    );
    let action = |suffix: &str, label: &str, action_type| UserAction {
        action_id: format!("{}:{suffix}", approval.id),
        label: label.into(),
        action_type,
    };
    OutboundMessage {
        conversation_id,
        content: MessageContent::Text { text },
        actions: vec![
            action("apply", "Apply", ActionType::Approve),
            action("view_diff", "View Diff", ActionType::Callback),
            action("revision", "Request Revision", ActionType::Callback),
            action("reject", "Reject", ActionType::Reject),
        ],
        reply_to: None,
        correlation_id: format!("approval:{}", approval.id),
    }
}
