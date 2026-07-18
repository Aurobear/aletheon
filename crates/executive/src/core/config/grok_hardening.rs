//! Feature flags for the Grok-inspired hardening mechanisms.
//!
//! Every flag gates one exec-spec item (see `docs/plans/grok/exec/`). All
//! default to `false`: with the whole section absent or all-off, every gated
//! code path must be equivalent to current behavior (no-op adapter). Consumers
//! read these flags at their integration point; defining them here does not by
//! itself wire any mechanism.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Toggles for the staged Grok-hardening mechanisms. Fail-closed on typos
/// (`deny_unknown_fields`) so a misspelled flag never silently reads as off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GrokHardeningConfig {
    /// G1 — folder trust gating of repo-executable config loading.
    pub folder_trust: bool,
    /// G2 — streaming tool-execution progress events.
    pub streaming_tools: bool,
    /// G3 — prompt queue / interjection with optimistic concurrency.
    pub prompt_queue: bool,
    /// G4 — workspace checkpoint / rewind.
    pub workspace_checkpoint: bool,
    /// G5 — typed lifecycle contributor / hook effects.
    pub lifecycle_hooks: bool,
    /// G6 — subagent resource settlement state machine.
    pub subagent_settlement: bool,
    /// G7 — endpoint-scoped memory-search credentials.
    pub memory_search: bool,
    /// G8 — ACP session-update adapter.
    pub acp_adapter: bool,
    /// S1 — layered sandbox profile enforcement.
    pub sandbox_profiles: bool,
    /// C1 — compaction guardrails + strategy selection (maybe_compact_v2).
    pub compaction_v2: bool,
}

impl GrokHardeningConfig {
    /// True if any hardening mechanism is enabled. Useful for cheap logging /
    /// diagnostics without threading every individual flag.
    pub fn any_enabled(&self) -> bool {
        let Self {
            folder_trust,
            streaming_tools,
            prompt_queue,
            workspace_checkpoint,
            lifecycle_hooks,
            subagent_settlement,
            memory_search,
            acp_adapter,
            sandbox_profiles,
            compaction_v2,
        } = self;
        *folder_trust
            || *streaming_tools
            || *prompt_queue
            || *workspace_checkpoint
            || *lifecycle_hooks
            || *subagent_settlement
            || *memory_search
            || *acp_adapter
            || *sandbox_profiles
            || *compaction_v2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_off() {
        let cfg = GrokHardeningConfig::default();
        assert!(!cfg.any_enabled());
        assert!(!cfg.compaction_v2);
        assert!(!cfg.sandbox_profiles);
    }

    #[test]
    fn parses_enabled_flag_and_defaults_the_rest() {
        let cfg: GrokHardeningConfig = toml::from_str("compaction_v2 = true\n").unwrap();
        assert!(cfg.compaction_v2);
        assert!(cfg.any_enabled());
        // Everything not named stays off.
        assert!(!cfg.sandbox_profiles);
        assert!(!cfg.folder_trust);
    }

    #[test]
    fn empty_section_is_all_off() {
        let cfg: GrokHardeningConfig = toml::from_str("").unwrap();
        assert!(!cfg.any_enabled());
    }

    #[test]
    fn unknown_flag_is_rejected() {
        // A misspelled flag must fail rather than silently read as off.
        let err = toml::from_str::<GrokHardeningConfig>("compaction_v3 = true\n");
        assert!(err.is_err());
    }
}
