//! Workspace trust decisions (G1).
//!
//! Constrains whether *repository-provided executable configuration* (repo-local
//! hooks, MCP server commands, plugins, `.envrc`, LSP commands, agent command
//! extensions) may load. This is orthogonal to whether the cwd is usable as a
//! filesystem workspace — that stays governed by [`crate::WorkspacePolicy`].
//!
//! This module holds pure types plus a pure [`decide`] function. Persistence
//! (trust store) and interactive prompting live in the Executive/Interact edges.
//!
//! See `docs/plans/grok/exec/G1-folder-trust.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::admission::PrincipalId;

/// Categories of repo-provided execution entry points gated by trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExecutableConfigSource {
    RepoHooks,
    RepoMcpServer,
    RepoPlugin,
    EnvrcLoader,
    LspServer,
    RepoAgentCommand,
}

impl ExecutableConfigSource {
    /// All sources, canonical order. Used for the feature-off "trust everything"
    /// path and for tests.
    pub fn all() -> Vec<Self> {
        vec![
            Self::RepoHooks,
            Self::RepoMcpServer,
            Self::RepoPlugin,
            Self::EnvrcLoader,
            Self::LspServer,
            Self::RepoAgentCommand,
        ]
    }
}

/// Client interaction capability. Decides whether an unrecorded-trust situation
/// prompts (interactive) or defaults to distrust (headless/daemon/CI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMode {
    Interactive,
    Headless,
}

/// Canonical identity of a workspace, resisting path-alias / symlink bypass.
/// `canonical_path` comes from the already-canonicalized cwd of a
/// [`crate::WorkspacePolicy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    /// Normalized git remote fingerprint when available, else `None`.
    pub repo_fingerprint: Option<String>,
}

/// Digest of discovered executable config, keyed by category. Config change ->
/// digest change -> prior receipt no longer auto-authorizes. Discovery is
/// read-only; it never interprets or executes the config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredConfigDigest(pub BTreeMap<ExecutableConfigSource, String>);

impl DiscoveredConfigDigest {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Categories present in the digest, canonical order.
    pub fn sources(&self) -> Vec<ExecutableConfigSource> {
        self.0.keys().copied().collect()
    }
}

/// Trust receipt: the persisted unit. Binds principal + workspace + digest +
/// granted scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustReceipt {
    pub principal_id: PrincipalId,
    pub workspace: WorkspaceIdentity,
    pub digest: DiscoveredConfigDigest,
    /// Partial grants allowed (e.g. hooks but not MCP).
    pub granted: Vec<ExecutableConfigSource>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    /// Expiry timestamp; `None` = never expires.
    pub expires_at_unix: Option<u64>,
    /// Granting client/connection identifier, for audit.
    pub granting_client: String,
}

/// Decision input. All fields are trusted (not model-forgeable).
#[derive(Debug, Clone)]
pub struct TrustEvaluationInput {
    pub principal_id: PrincipalId,
    pub workspace: WorkspaceIdentity,
    pub discovered: DiscoveredConfigDigest,
    pub client_mode: ClientMode,
    /// Feature flag: when off, the decision is always `Trusted(all)`.
    pub feature_enabled: bool,
    /// Existing receipt for this (principal, workspace), if any.
    pub existing_receipt: Option<TrustReceipt>,
    /// Broad root that must not record persistent trust (e.g. `$HOME`).
    pub is_broad_unrecordable_root: bool,
    pub now_unix: u64,
}

/// Decision result, limited to three states (aligned with Grok's
/// Trusted/Untrusted/Prompt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTrustDecision {
    /// Load the receipt-granted categories.
    Trusted {
        granted: Vec<ExecutableConfigSource>,
    },
    /// Load no repo-local executable config (normal files still usable).
    Restricted {
        blocked: Vec<ExecutableConfigSource>,
    },
    /// Interactive confirmation required. `findings` = discovered categories.
    PromptRequired {
        findings: Vec<ExecutableConfigSource>,
    },
}

/// Pure decision function. No I/O, no side effects — fully unit-testable.
///
/// Priority (aligned with Grok folder_trust semantics):
/// 1. feature off              -> `Trusted(all)`   (equivalent to current no-gate behavior)
/// 2. no discovered config     -> `Trusted([])`    (nothing to gate)
/// 3. broad unrecordable root  -> `Restricted`     (don't record persistent trust at `$HOME`)
/// 4. valid receipt, digest unchanged -> `Trusted(receipt.granted)`
/// 5. headless                 -> `Restricted`     (cannot prompt; default distrust)
/// 6. otherwise (interactive)  -> `PromptRequired`
pub fn decide(input: &TrustEvaluationInput) -> WorkspaceTrustDecision {
    use WorkspaceTrustDecision::*;

    // 1. feature off: equivalent to current no-gate behavior.
    if !input.feature_enabled {
        return Trusted {
            granted: ExecutableConfigSource::all(),
        };
    }

    let found = input.discovered.sources();

    // 2. no executable config: nothing to gate.
    if found.is_empty() {
        return Trusted { granted: vec![] };
    }

    // 3. broad root: do not record persistent trust; restrict directly.
    if input.is_broad_unrecordable_root {
        return Restricted { blocked: found };
    }

    // 4. valid receipt with unchanged digest, matching principal + workspace.
    if let Some(r) = &input.existing_receipt {
        let not_expired = r.expires_at_unix.is_none_or(|e| input.now_unix < e);
        let digest_match = r.digest == input.discovered;
        let principal_match = r.principal_id == input.principal_id;
        let workspace_match = r.workspace == input.workspace;
        if not_expired && digest_match && principal_match && workspace_match {
            return Trusted {
                granted: r.granted.clone(),
            };
        }
        // digest changed / expired / mismatch -> fall through, do not inherit.
    }

    // 5. headless: cannot prompt -> default distrust.
    if input.client_mode == ClientMode::Headless {
        return Restricted { blocked: found };
    }

    // 6. interactive: prompt.
    PromptRequired { findings: found }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> WorkspaceIdentity {
        WorkspaceIdentity {
            canonical_path: PathBuf::from("/home/user/project"),
            repo_fingerprint: Some("git:abc123".to_string()),
        }
    }

    fn digest_with(sources: &[ExecutableConfigSource]) -> DiscoveredConfigDigest {
        let mut m = BTreeMap::new();
        for (i, s) in sources.iter().enumerate() {
            m.insert(*s, format!("sha256:{i}"));
        }
        DiscoveredConfigDigest(m)
    }

    fn base_input() -> TrustEvaluationInput {
        TrustEvaluationInput {
            principal_id: PrincipalId("local-uid:1000".to_string()),
            workspace: ws(),
            discovered: digest_with(&[ExecutableConfigSource::RepoHooks]),
            client_mode: ClientMode::Interactive,
            feature_enabled: true,
            existing_receipt: None,
            is_broad_unrecordable_root: false,
            now_unix: 1_000,
        }
    }

    fn receipt_for(
        input: &TrustEvaluationInput,
        granted: Vec<ExecutableConfigSource>,
        expires_at_unix: Option<u64>,
    ) -> TrustReceipt {
        TrustReceipt {
            principal_id: input.principal_id.clone(),
            workspace: input.workspace.clone(),
            digest: input.discovered.clone(),
            granted,
            created_at_unix: 0,
            updated_at_unix: 0,
            expires_at_unix,
            granting_client: "conn-1".to_string(),
        }
    }

    /// case 1: feature off -> Trusted(all).
    #[test]
    fn feature_off_trusts_all() {
        let mut input = base_input();
        input.feature_enabled = false;
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::Trusted {
                granted: ExecutableConfigSource::all()
            }
        );
    }

    /// case 2: no discovered config -> Trusted([]).
    #[test]
    fn empty_discovery_trusts_nothing_to_gate() {
        let mut input = base_input();
        input.discovered = DiscoveredConfigDigest::default();
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::Trusted { granted: vec![] }
        );
    }

    /// case 3: broad root -> Restricted.
    #[test]
    fn broad_root_is_restricted() {
        let mut input = base_input();
        input.is_broad_unrecordable_root = true;
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::Restricted {
                blocked: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// case 4a: valid receipt, digest matches -> Trusted(receipt.granted).
    #[test]
    fn valid_receipt_matching_digest_trusts_granted() {
        let mut input = base_input();
        let granted = vec![ExecutableConfigSource::RepoHooks];
        input.existing_receipt = Some(receipt_for(&input, granted.clone(), None));
        assert_eq!(decide(&input), WorkspaceTrustDecision::Trusted { granted });
    }

    /// case 4b: digest changed -> does not inherit (interactive -> Prompt).
    #[test]
    fn changed_digest_does_not_inherit() {
        let mut input = base_input();
        let receipt = receipt_for(&input, vec![ExecutableConfigSource::RepoHooks], None);
        input.existing_receipt = Some(receipt);
        // Config changed after the receipt was minted.
        input.discovered = digest_with(&[
            ExecutableConfigSource::RepoHooks,
            ExecutableConfigSource::RepoMcpServer,
        ]);
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::PromptRequired {
                findings: vec![
                    ExecutableConfigSource::RepoHooks,
                    ExecutableConfigSource::RepoMcpServer
                ]
            }
        );
    }

    /// case 5: headless + config + no receipt -> Restricted.
    #[test]
    fn headless_without_receipt_is_restricted() {
        let mut input = base_input();
        input.client_mode = ClientMode::Headless;
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::Restricted {
                blocked: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// case 6: interactive + config + no receipt -> PromptRequired.
    #[test]
    fn interactive_without_receipt_prompts() {
        let input = base_input();
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::PromptRequired {
                findings: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// multi-user isolation: Alice's receipt does not authorize Bob.
    #[test]
    fn other_principal_receipt_does_not_authorize() {
        let mut input = base_input();
        let mut alice_receipt = receipt_for(&input, vec![ExecutableConfigSource::RepoHooks], None);
        alice_receipt.principal_id = PrincipalId("local-uid:2000".to_string());
        input.existing_receipt = Some(alice_receipt);
        // Bob (input.principal_id) gets no inheritance -> interactive prompt.
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::PromptRequired {
                findings: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// expired receipt does not authorize.
    #[test]
    fn expired_receipt_does_not_authorize() {
        let mut input = base_input();
        input.now_unix = 5_000;
        input.existing_receipt = Some(receipt_for(
            &input,
            vec![ExecutableConfigSource::RepoHooks],
            Some(4_000), // expired before now_unix
        ));
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::PromptRequired {
                findings: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// workspace identity mismatch does not authorize (path-alias defense).
    #[test]
    fn mismatched_workspace_does_not_authorize() {
        let mut input = base_input();
        let mut receipt = receipt_for(&input, vec![ExecutableConfigSource::RepoHooks], None);
        receipt.workspace.canonical_path = PathBuf::from("/home/user/other");
        input.existing_receipt = Some(receipt);
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::PromptRequired {
                findings: vec![ExecutableConfigSource::RepoHooks]
            }
        );
    }

    /// decide is total: always returns one of the three states, and feature-off
    /// is always Trusted(all).
    #[test]
    fn feature_off_is_always_trusted_all_regardless_of_other_fields() {
        let mut input = base_input();
        input.feature_enabled = false;
        input.client_mode = ClientMode::Headless;
        input.is_broad_unrecordable_root = true;
        input.discovered = digest_with(&ExecutableConfigSource::all());
        assert_eq!(
            decide(&input),
            WorkspaceTrustDecision::Trusted {
                granted: ExecutableConfigSource::all()
            }
        );
    }
}
