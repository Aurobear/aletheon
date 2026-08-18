//! Projection of Runtime lifecycle context effects into canonical Session history.

pub async fn persist_before_turn_fragments(
    sessions: &runtime::session_service::SessionService,
    session_id: &contracts::SessionId,
    turn_id: Option<contracts::TurnId>,
    effects: &[runtime::lifecycle_contributors::LifecycleEffect],
) -> anyhow::Result<String> {
    let mut prefix = String::new();
    let mut fragments = Vec::new();
    for effect in effects {
        if let runtime::lifecycle_contributors::LifecycleEffect::AddContextFragment {
            source,
            content,
        } = effect
        {
            prefix.push_str(&format!("[lifecycle:{source}]\n{content}\n"));
            fragments.push((source.clone(), content.clone()));
        }
    }
    if fragments.is_empty() {
        return Ok(prefix);
    }

    let turn_id =
        turn_id.ok_or_else(|| anyhow::anyhow!("lifecycle context requires an explicit turn_id"))?;
    sessions
        .persist_context_fragments(
            session_id,
            turn_id,
            runtime::lifecycle::LifecyclePhase::BeforeTurnInput,
            fragments,
        )
        .await?;
    Ok(prefix)
}
