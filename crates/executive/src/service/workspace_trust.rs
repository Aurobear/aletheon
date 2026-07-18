//! Executive orchestration for G1 workspace trust decisions.
//!
//! Discovery is deliberately read-only: it hashes a bounded set of known
//! repository-provided executable configuration files without parsing or
//! executing any of them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::PrincipalId;
use fabric::workspace_trust::{
    ClientMode, DiscoveredConfigDigest, ExecutableConfigSource, TrustEvaluationInput, TrustReceipt,
    WorkspaceIdentity, WorkspaceTrustDecision, decide,
};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

#[async_trait]
pub trait TrustStore: Send + Sync {
    async fn get(
        &self,
        principal: &PrincipalId,
        workspace: &WorkspaceIdentity,
    ) -> Option<TrustReceipt>;

    async fn put(&self, receipt: TrustReceipt);
}

#[async_trait]
pub trait ConfigDiscoverer: Send + Sync {
    async fn discover(&self, workspace_cwd: &Path) -> DiscoveredConfigDigest;
}

pub struct WorkspaceTrustResolver {
    store: Arc<dyn TrustStore>,
    discoverer: Arc<dyn ConfigDiscoverer>,
    feature_enabled: bool,
}

impl WorkspaceTrustResolver {
    pub fn new(
        store: Arc<dyn TrustStore>,
        discoverer: Arc<dyn ConfigDiscoverer>,
        feature_enabled: bool,
    ) -> Self {
        Self {
            store,
            discoverer,
            feature_enabled,
        }
    }

    pub async fn evaluate(
        &self,
        principal_id: PrincipalId,
        workspace: WorkspaceIdentity,
        client_mode: ClientMode,
        is_broad_unrecordable_root: bool,
        now_unix: u64,
    ) -> WorkspaceTrustDecision {
        if !self.feature_enabled {
            return WorkspaceTrustDecision::Trusted {
                granted: ExecutableConfigSource::all(),
            };
        }

        let discovered = self.discoverer.discover(&workspace.canonical_path).await;
        let existing_receipt = self.store.get(&principal_id, &workspace).await;
        decide(&TrustEvaluationInput {
            principal_id,
            workspace,
            discovered,
            client_mode,
            feature_enabled: true,
            existing_receipt,
            is_broad_unrecordable_root,
            now_unix,
        })
    }

    pub async fn record_grant(&self, receipt: TrustReceipt) {
        self.store.put(receipt).await;
    }
}

/// Deterministic in-memory store used by integration tests and ephemeral hosts.
#[derive(Default)]
pub struct InMemoryTrustStore {
    receipts: RwLock<BTreeMap<String, TrustReceipt>>,
}

fn receipt_key(principal: &PrincipalId, workspace: &WorkspaceIdentity) -> String {
    format!(
        "{}\0{}\0{}",
        principal.0,
        workspace.canonical_path.display(),
        workspace.repo_fingerprint.as_deref().unwrap_or("")
    )
}

#[async_trait]
impl TrustStore for InMemoryTrustStore {
    async fn get(
        &self,
        principal: &PrincipalId,
        workspace: &WorkspaceIdentity,
    ) -> Option<TrustReceipt> {
        self.receipts
            .read()
            .await
            .get(&receipt_key(principal, workspace))
            .cloned()
    }

    async fn put(&self, receipt: TrustReceipt) {
        let key = receipt_key(&receipt.principal_id, &receipt.workspace);
        self.receipts.write().await.insert(key, receipt);
    }
}

/// Bounded, read-only discovery of known executable configuration paths.
#[derive(Debug, Clone)]
pub struct KnownConfigDiscoverer {
    max_files: usize,
    max_file_bytes: u64,
}

impl Default for KnownConfigDiscoverer {
    fn default() -> Self {
        Self {
            max_files: 256,
            max_file_bytes: 1024 * 1024,
        }
    }
}

impl KnownConfigDiscoverer {
    fn candidates(workspace: &Path) -> [(ExecutableConfigSource, PathBuf); 6] {
        [
            (
                ExecutableConfigSource::RepoHooks,
                workspace.join(".grok/hooks"),
            ),
            (
                ExecutableConfigSource::RepoMcpServer,
                workspace.join(".grok/mcp.json"),
            ),
            (
                ExecutableConfigSource::RepoPlugin,
                workspace.join(".aletheon/plugins"),
            ),
            (
                ExecutableConfigSource::EnvrcLoader,
                workspace.join(".envrc"),
            ),
            (
                ExecutableConfigSource::LspServer,
                workspace.join(".grok/lsp.json"),
            ),
            (
                ExecutableConfigSource::RepoAgentCommand,
                workspace.join("agents"),
            ),
        ]
    }

    fn collect_files(&self, candidate: &Path) -> Vec<PathBuf> {
        if candidate.is_file() {
            return vec![candidate.to_path_buf()];
        }
        if !candidate.is_dir() {
            return Vec::new();
        }

        let mut files = Vec::new();
        let mut pending = vec![candidate.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    files.push(path);
                    if files.len() >= self.max_files {
                        break;
                    }
                }
            }
            if files.len() >= self.max_files {
                break;
            }
        }
        files.sort();
        files
    }

    fn digest_source(&self, workspace: &Path, candidate: &Path) -> Option<String> {
        let files = self.collect_files(candidate);
        if files.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        for path in files {
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.len() > self.max_file_bytes {
                continue;
            }
            let Ok(content) = std::fs::read(&path) else {
                continue;
            };
            let relative = path.strip_prefix(workspace).unwrap_or(&path);
            hasher.update(relative.to_string_lossy().as_bytes());
            hasher.update([0]);
            hasher.update(content);
            hasher.update([0xff]);
        }
        Some(format!("sha256:{:x}", hasher.finalize()))
    }
}

#[async_trait]
impl ConfigDiscoverer for KnownConfigDiscoverer {
    async fn discover(&self, workspace_cwd: &Path) -> DiscoveredConfigDigest {
        let mut digest = BTreeMap::new();
        for (source, candidate) in Self::candidates(workspace_cwd) {
            if let Some(value) = self.digest_source(workspace_cwd, &candidate) {
                digest.insert(source, value);
            }
        }
        DiscoveredConfigDigest(digest)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn identity(path: &Path) -> WorkspaceIdentity {
        WorkspaceIdentity {
            canonical_path: path.to_path_buf(),
            repo_fingerprint: None,
        }
    }

    fn receipt(
        principal: &PrincipalId,
        workspace: &WorkspaceIdentity,
        digest: DiscoveredConfigDigest,
        updated_at_unix: u64,
    ) -> TrustReceipt {
        TrustReceipt {
            principal_id: principal.clone(),
            workspace: workspace.clone(),
            digest,
            granted: vec![ExecutableConfigSource::RepoHooks],
            created_at_unix: 1,
            updated_at_unix,
            expires_at_unix: None,
            granting_client: "test".into(),
        }
    }

    #[tokio::test]
    async fn memory_store_upsert_replaces_same_scope() {
        let store = InMemoryTrustStore::default();
        let principal = PrincipalId("alice".into());
        let workspace = identity(Path::new("/tmp/project"));
        store
            .put(receipt(
                &principal,
                &workspace,
                DiscoveredConfigDigest::default(),
                1,
            ))
            .await;
        store
            .put(receipt(
                &principal,
                &workspace,
                DiscoveredConfigDigest::default(),
                2,
            ))
            .await;

        assert_eq!(
            store
                .get(&principal, &workspace)
                .await
                .unwrap()
                .updated_at_unix,
            2
        );
        assert_eq!(store.receipts.read().await.len(), 1);
    }

    #[tokio::test]
    async fn resolver_prompts_then_accepts_recorded_grant() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".grok/hooks")).unwrap();
        std::fs::write(temp.path().join(".grok/hooks/pre.sh"), "echo safe").unwrap();
        let store = Arc::new(InMemoryTrustStore::default());
        let discoverer = Arc::new(KnownConfigDiscoverer::default());
        let resolver = WorkspaceTrustResolver::new(store.clone(), discoverer.clone(), true);
        let principal = PrincipalId("alice".into());
        let workspace = identity(temp.path());

        assert!(matches!(
            resolver
                .evaluate(
                    principal.clone(),
                    workspace.clone(),
                    ClientMode::Interactive,
                    false,
                    10,
                )
                .await,
            WorkspaceTrustDecision::PromptRequired { .. }
        ));
        let digest = discoverer.discover(temp.path()).await;
        resolver
            .record_grant(receipt(&principal, &workspace, digest, 10))
            .await;
        assert!(matches!(
            resolver
                .evaluate(principal, workspace, ClientMode::Interactive, false, 11,)
                .await,
            WorkspaceTrustDecision::Trusted { .. }
        ));
    }

    #[tokio::test]
    async fn discovery_is_stable_and_changes_with_content_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let hooks = temp.path().join(".grok/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre.sh");
        std::fs::write(&hook, "echo one").unwrap();
        let discoverer = KnownConfigDiscoverer::default();
        let before_entries = std::fs::read_dir(&hooks).unwrap().count();

        let first = discoverer.discover(temp.path()).await;
        let same = discoverer.discover(temp.path()).await;
        assert_eq!(first, same);
        assert_eq!(std::fs::read_dir(&hooks).unwrap().count(), before_entries);

        std::fs::write(hook, "echo two").unwrap();
        assert_ne!(first, discoverer.discover(temp.path()).await);
    }

    struct CountingDiscoverer(AtomicUsize);

    #[async_trait]
    impl ConfigDiscoverer for CountingDiscoverer {
        async fn discover(&self, _workspace_cwd: &Path) -> DiscoveredConfigDigest {
            self.0.fetch_add(1, Ordering::SeqCst);
            DiscoveredConfigDigest::default()
        }
    }

    #[tokio::test]
    async fn feature_off_is_a_strict_discovery_bypass() {
        let discoverer = Arc::new(CountingDiscoverer(AtomicUsize::new(0)));
        let resolver = WorkspaceTrustResolver::new(
            Arc::new(InMemoryTrustStore::default()),
            discoverer.clone(),
            false,
        );
        let decision = resolver
            .evaluate(
                PrincipalId("alice".into()),
                identity(Path::new("/tmp/project")),
                ClientMode::Headless,
                false,
                0,
            )
            .await;

        assert_eq!(discoverer.0.load(Ordering::SeqCst), 0);
        assert_eq!(
            decision,
            WorkspaceTrustDecision::Trusted {
                granted: ExecutableConfigSource::all()
            }
        );
    }
}
