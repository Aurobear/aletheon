//! Typed lifecycle contributor model (G5, fabric layer).
//!
//! In-process contributors receive an immutable data-only [`LifecycleInput`] at
//! a [`LifecyclePhase`] and return bounded declarative [`LifecycleEffect`]s.
//! Contributors never own the turn loop and cannot invoke tools — high-risk
//! effects are re-authorized/executed by the Executive. This module holds the
//! pure types plus bounded-effect validation; the registry and dispatch live in
//! the Executive.
//!
//! This is orthogonal to the command-hook path (external processes) which stays
//! in corpus. See `docs/plans/grok/exec/G5-lifecycle-hooks.md`.

use serde::{Deserialize, Serialize};

/// Lifecycle dispatch points (in-process counterparts to hook points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LifecyclePhase {
    BeforeSessionStart,
    AfterSessionStart,
    BeforeSessionEnd,
    AfterSessionEnd,
    BeforeTurnInput,
    AfterContextProjection,
    BeforeModelCall,
    BeforeToolBatch,
    AfterToolTerminal,
    AfterTurnTerminal,
    OnAbort,
}

impl LifecyclePhase {
    /// Whether `RejectInput` is meaningful at this phase (only input-gating
    /// phases can reject).
    pub fn allows_reject(&self) -> bool {
        matches!(self, Self::BeforeTurnInput | Self::BeforeToolBatch)
    }
}

/// Immutable, data-only dispatch input. Carries read-only trusted identifiers
/// but never a forgeable approval authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleInput {
    pub phase: LifecyclePhase,
    pub principal_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub session_id: String,
    /// Phase-relevant read-only snapshot (e.g. tool terminal call_id/name).
    pub detail: serde_json::Value,
}

/// Declarative effects a contributor may return. Contributors cannot advance
/// the turn or call tools directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleEffect {
    Continue,
    /// Append a bounded context fragment, with attributed source.
    AddContextFragment {
        source: String,
        content: String,
    },
    EmitEvent {
        schema: String,
        payload: serde_json::Value,
    },
    RequestCheckpoint,
    RequestCancellation {
        reason: String,
    },
    /// Only valid at input-gating phases (see [`LifecyclePhase::allows_reject`]).
    RejectInput {
        reason: String,
    },
}

/// Effect bounds.
pub const MAX_EFFECTS_PER_DISPATCH: usize = 32;
pub const MAX_CONTEXT_FRAGMENT_BYTES: usize = 8 * 1024;

/// Why a contributor's returned effects were rejected during validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectRejection {
    TooManyEffects { count: usize },
    FragmentTooLarge { source: String, bytes: usize },
    RejectNotAllowedAtPhase { phase: LifecyclePhase },
}

/// Validate a contributor's returned effects against the bounds and phase
/// rules. Pure; the Executive calls this before interpreting effects.
pub fn validate_effects(
    phase: LifecyclePhase,
    effects: &[LifecycleEffect],
) -> Result<(), EffectRejection> {
    if effects.len() > MAX_EFFECTS_PER_DISPATCH {
        return Err(EffectRejection::TooManyEffects {
            count: effects.len(),
        });
    }
    for effect in effects {
        match effect {
            LifecycleEffect::AddContextFragment { source, content } => {
                if content.len() > MAX_CONTEXT_FRAGMENT_BYTES {
                    return Err(EffectRejection::FragmentTooLarge {
                        source: source.clone(),
                        bytes: content.len(),
                    });
                }
            }
            LifecycleEffect::RejectInput { .. } if !phase.allows_reject() => {
                return Err(EffectRejection::RejectNotAllowedAtPhase { phase });
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_small_effect_sets_validate() {
        assert!(validate_effects(LifecyclePhase::AfterToolTerminal, &[]).is_ok());
        assert!(validate_effects(
            LifecyclePhase::AfterToolTerminal,
            &[LifecycleEffect::Continue],
        )
        .is_ok());
    }

    #[test]
    fn too_many_effects_rejected() {
        let effects = vec![LifecycleEffect::Continue; MAX_EFFECTS_PER_DISPATCH + 1];
        assert_eq!(
            validate_effects(LifecyclePhase::AfterToolTerminal, &effects),
            Err(EffectRejection::TooManyEffects {
                count: MAX_EFFECTS_PER_DISPATCH + 1
            })
        );
    }

    #[test]
    fn oversized_fragment_rejected() {
        let big = LifecycleEffect::AddContextFragment {
            source: "mem".to_string(),
            content: "x".repeat(MAX_CONTEXT_FRAGMENT_BYTES + 1),
        };
        assert!(matches!(
            validate_effects(LifecyclePhase::AfterContextProjection, &[big]),
            Err(EffectRejection::FragmentTooLarge { .. })
        ));
    }

    #[test]
    fn reject_only_allowed_at_input_gating_phases() {
        let reject = LifecycleEffect::RejectInput {
            reason: "blocked".to_string(),
        };
        // Allowed at input-gating phases.
        assert!(validate_effects(LifecyclePhase::BeforeTurnInput, &[reject.clone()]).is_ok());
        assert!(validate_effects(LifecyclePhase::BeforeToolBatch, &[reject.clone()]).is_ok());
        // Rejected elsewhere.
        assert_eq!(
            validate_effects(LifecyclePhase::AfterTurnTerminal, &[reject]),
            Err(EffectRejection::RejectNotAllowedAtPhase {
                phase: LifecyclePhase::AfterTurnTerminal
            })
        );
    }

    #[test]
    fn phase_ordering_is_stable() {
        // Ord derive gives a deterministic contributor bucket order.
        assert!(LifecyclePhase::BeforeSessionStart < LifecyclePhase::AfterSessionStart);
        assert!(LifecyclePhase::BeforeModelCall < LifecyclePhase::AfterToolTerminal);
    }

    #[test]
    fn input_serde_roundtrip() {
        let input = LifecycleInput {
            phase: LifecyclePhase::AfterToolTerminal,
            principal_id: "local-uid:1000".to_string(),
            thread_id: "t1".to_string(),
            turn_id: Some("turn1".to_string()),
            session_id: "s1".to_string(),
            detail: serde_json::json!({"call_id": "c1"}),
        };
        let json = serde_json::to_string(&input).unwrap();
        let back: LifecycleInput = serde_json::from_str(&json).unwrap();
        assert_eq!(input, back);
    }
}
