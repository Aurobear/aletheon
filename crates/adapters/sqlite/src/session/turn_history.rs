//! Canonical Session history projection for model replay.

pub async fn resume_before_current_user(
    sessions: &runtime::session_service::SessionService,
    session_id: &contracts::SessionId,
    current_input: &str,
) -> anyhow::Result<Vec<contracts::Message>> {
    let mut messages = sessions.resume(session_id).await?.messages;
    if messages.last().is_some_and(|last| {
        last.role == contracts::Role::User
            && last.content.iter().any(|block| {
                matches!(block, contracts::ContentBlock::Text { text } if text == current_input)
            })
    }) {
        messages.pop();
    }
    Ok(messages)
}
