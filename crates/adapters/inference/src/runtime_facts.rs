//! Typed runtime-fact projection into model context and cache shape.

use contracts::{ContentBlock, Role};

pub fn bind_runtime_facts(
    messages: &mut [::contracts::Message],
    facts: &::contracts::ModelRuntimeFacts,
) {
    let encoded = serde_json::to_string(facts).unwrap_or_else(|_| "{}".into());
    let identity = format!(
        "\n\n<runtime-facts>\nThe Aletheon host reports these authoritative runtime facts for this turn: {encoded}. If asked about model identity or context capacity, use these values exactly. Do not claim a different vendor, model version, or context limit from training priors. Clarify that you cannot independently verify the provider behind the host-reported effective_model_id.\n</runtime-facts>"
    );
    if let Some(system) = messages
        .iter_mut()
        .find(|message| message.role == Role::System)
    {
        if let Some(ContentBlock::Text { text }) = system.content.first_mut() {
            text.push_str(&identity);
        }
    }
}

pub(crate) fn stable_system_prefix_wire(messages: &[::contracts::Message]) -> String {
    let system = messages
        .iter()
        .filter(|message| message.role == Role::System)
        .collect::<Vec<_>>();
    serde_json::to_string(&system).expect("system message projection serializes")
}

pub async fn track_prefix_shape(
    trackers: &tokio::sync::Mutex<runtime::cache_shape::PrefixShapeTrackerStore>,
    thread_id: &str,
    facts: &contracts::ModelRuntimeFacts,
    messages: &[contracts::Message],
    tools: &[contracts::ToolDefinition],
    profile_name: &str,
    rewrite_version: u64,
) -> (
    Option<String>,
    Option<runtime::cache_shape::LocalMissReason>,
    bool,
) {
    let provider_id = facts.provider_id.as_deref().unwrap_or("unknown");
    let transport = facts.transport.as_deref().unwrap_or("unknown");
    let system_prefix = stable_system_prefix_wire(messages);
    match runtime::cache_shape::InferencePrefixShape::compute(
        provider_id,
        &facts.effective_model_id,
        transport,
        &system_prefix,
        tools,
        &runtime::cache_shape::agent_profile_digest(profile_name),
        rewrite_version,
    ) {
        Ok(shape) => {
            let digest = shape.digest();
            let mut trackers = trackers.lock().await;
            let (had_previous, reason) = trackers.track(thread_id, &shape);
            if let Some(reason) = reason {
                tracing::info!(
                    reason = reason.as_str(),
                    prefix_shape_digest = digest,
                    "host-controlled inference prefix shape changed"
                );
            }
            (Some(digest), reason, had_previous && reason.is_none())
        }
        Err(error) => {
            tracing::warn!(%error, "prefix shape diagnostic degraded");
            (None, None, false)
        }
    }
}
