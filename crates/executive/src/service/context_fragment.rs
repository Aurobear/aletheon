//! Context fragment persistence for durable workspace context (M4-T4).
//!
//! Consumes G5 `LifecycleEffect::AddContextFragment` and the bounded-effect
//! validation from `fabric::types::lifecycle`. At each turn-loop phase where
//! lifecycle contributions are dispatched, the executive persists validated
//! `AddContextFragment` effects as canonical session items.
//!
//! Gated behind `grok_hardening.compaction_v2`.

use anyhow::Result;
use fabric::{
    ItemPayload, SESSION_SCHEMA_VERSION, SessionAppendStore, SessionId, TurnId,
    types::lifecycle::{
        LifecycleEffect, LifecyclePhase, MAX_CONTEXT_FRAGMENT_BYTES, validate_effects,
    },
};

/// Persist one or more context fragments emitted as lifecycle effects.
///
/// Returns the number of fragments that were persisted. Fragments that
/// exceed `MAX_CONTEXT_FRAGMENT_BYTES` are rejected with a warning.
pub async fn inject_context_fragments(
    store: &dyn SessionAppendStore,
    session_id: &SessionId,
    turn_id: TurnId,
    sequence: &mut u64,
    phase: LifecyclePhase,
    effects: &[LifecycleEffect],
) -> Result<usize> {
    validate_effects(phase, effects).map_err(|rejection| {
        anyhow::anyhow!("lifecycle effect validation rejected: {rejection:?}")
    })?;

    let mut persisted = 0usize;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    for effect in effects {
        if let LifecycleEffect::AddContextFragment { source, content } = effect {
            if content.len() > MAX_CONTEXT_FRAGMENT_BYTES {
                tracing::warn!(
                    source = %source,
                    bytes = content.len(),
                    limit = MAX_CONTEXT_FRAGMENT_BYTES,
                    "context fragment exceeds limit; truncating",
                );
            }
            let bounded = truncate_at_char_boundary(content, MAX_CONTEXT_FRAGMENT_BYTES);
            let payload = ItemPayload::SystemNotice {
                content: format!(
                    "[context-fragment source={source} turn={} phase={phase:?}]\n{bounded}",
                    turn_id.0,
                ),
            };
            let item = fabric::ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: fabric::ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence: *sequence,
                created_at_ms: now_ms,
                payload,
            };
            // When `compaction_v2` is enabled the caller should track this
            // append result through the durable-write tracker (M4-T1).
            store
                .append(session_id, *sequence, item)
                .await
                .map_err(|error| {
                    anyhow::anyhow!("context fragment append failed for source={source}: {error:#}")
                })?;
            *sequence += 1;
            persisted += 1;
        }
    }

    Ok(persisted)
}

/// Truncate a string to a byte limit without splitting a UTF-8 code point.
fn truncate_at_char_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_safe() {
        let s = "hello world";
        assert_eq!(truncate_at_char_boundary(s, 5), "hello");
        assert_eq!(truncate_at_char_boundary(s, 50), s);
    }

    #[test]
    fn truncate_multibyte_boundary() {
        let s = "hello\u{1f600}world";
        let result = truncate_at_char_boundary(s, 8);
        assert!(result.len() <= 8);
        assert!(result.starts_with("hello"));
    }

    #[test]
    fn effect_validation_rejects_oversized() {
        let big = LifecycleEffect::AddContextFragment {
            source: "test".into(),
            content: "x".repeat(MAX_CONTEXT_FRAGMENT_BYTES + 1),
        };
        let result = validate_effects(LifecyclePhase::AfterContextProjection, &[big]);
        assert!(result.is_err());
    }

    #[test]
    fn effect_validation_accepts_valid() {
        let ok = LifecycleEffect::AddContextFragment {
            source: "test".into(),
            content: "small fragment".into(),
        };
        assert!(validate_effects(LifecyclePhase::AfterTurnTerminal, &[ok]).is_ok());
    }
}
