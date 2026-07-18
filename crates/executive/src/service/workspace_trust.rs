//! Executive orchestration for G1 workspace trust decisions.
//!
//! Discovery is deliberately read-only: it hashes a bounded set of known
//! repository-provided executable configuration files without parsing or
//! executing any of them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::ipc::bus::kernel_bus::CanonicalEventBus;
use fabric::workspace_trust::{
    ClientMode, DiscoveredConfigDigest, ExecutableConfigSource, TrustEvaluationInput, TrustReceipt,
    WorkspaceIdentity, WorkspaceTrustDecision, decide,
};
use fabric::{PrincipalId, SchemaId};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};

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
    event_bus: Option<Arc<CanonicalEventBus>>,
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
            event_bus: None,
        }
    }

    pub fn with_event_bus(mut self, event_bus: Arc<CanonicalEventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
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
        let granting_client = existing_receipt
            .as_ref()
            .map(|receipt| receipt.granting_client.clone());
        let decision = decide(&TrustEvaluationInput {
            principal_id: principal_id.clone(),
            workspace: workspace.clone(),
            discovered,
            client_mode,
            feature_enabled: true,
            existing_receipt,
            is_broad_unrecordable_root,
            now_unix,
        });
        if let Some(event_bus) = &self.event_bus {
            let (decision_name, sources) = decision_event_fields(&decision);
            let _ = event_bus
                .publish_event(
                    SchemaId::from("aletheon.event.workspace_trust_decided/v1"),
                    "executive:workspace-trust",
                    serde_json::json!({
                        "principal_id": principal_id.0,
                        "workspace": workspace.canonical_path,
                        "decision": decision_name,
                        "sources": sources,
                        "granting_client": granting_client,
                    }),
                )
                .await;
        }
        decision
    }

    pub async fn record_grant(&self, receipt: TrustReceipt) {
        self.store.put(receipt).await;
    }
}

fn decision_event_fields(
    decision: &WorkspaceTrustDecision,
) -> (&'static str, Vec<ExecutableConfigSource>) {
    match decision {
        WorkspaceTrustDecision::Trusted { granted } => ("trusted", granted.clone()),
        WorkspaceTrustDecision::Restricted { blocked } => ("restricted", blocked.clone()),
        WorkspaceTrustDecision::PromptRequired { findings } => {
            ("prompt_required", findings.clone())
        }
    }
}

/// Query whether a specific repository executable source may be loaded.
/// Normal workspace file access is intentionally outside this decision.
pub fn source_is_granted(
    decision: &WorkspaceTrustDecision,
    source: ExecutableConfigSource,
) -> bool {
    matches!(
        decision,
        WorkspaceTrustDecision::Trusted { granted } if granted.contains(&source)
    )
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

/// Durable JSON trust store with atomic replacement.
///
/// Read or decode failures fail closed through the `TrustStore` contract by
/// returning no receipt. Writes are serialized and replace the file only after
/// the complete new state has been flushed.
pub struct FileTrustStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl FileTrustStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    async fn read_all(&self) -> BTreeMap<String, TrustReceipt> {
        let Ok(bytes) = tokio::fs::read(&self.path).await else {
            return BTreeMap::new();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }
}

#[async_trait]
impl TrustStore for FileTrustStore {
    async fn get(
        &self,
        principal: &PrincipalId,
        workspace: &WorkspaceIdentity,
    ) -> Option<TrustReceipt> {
        self.read_all()
            .await
            .remove(&receipt_key(principal, workspace))
    }

    async fn put(&self, receipt: TrustReceipt) {
        let _guard = self.write_lock.lock().await;
        let mut receipts = self.read_all().await;
        receipts.insert(
            receipt_key(&receipt.principal_id, &receipt.workspace),
            receipt,
        );
        let Ok(encoded) = serde_json::to_vec_pretty(&receipts) else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            if tokio::fs::create_dir_all(parent).await.is_err() {
                return;
            }
        }
        let temporary = self.path.with_extension("tmp");
        if tokio::fs::write(&temporary, encoded).await.is_ok() {
            let _ = tokio::fs::rename(temporary, &self.path).await;
        }
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
    async fn file_store_survives_reopen_and_upserts_scope() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("trust/receipts.json");
        let principal = PrincipalId("alice".into());
        let workspace = identity(temp.path());
        {
            let store = FileTrustStore::new(path.clone());
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
        }

        let reopened = FileTrustStore::new(path);
        assert_eq!(
            reopened
                .get(&principal, &workspace)
                .await
                .unwrap()
                .updated_at_unix,
            2
        );
    }

    #[tokio::test]
    async fn corrupt_file_store_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("receipts.json");
        std::fs::write(&path, b"not-json").unwrap();
        let store = FileTrustStore::new(path);

        assert!(
            store
                .get(&PrincipalId("alice".into()), &identity(temp.path()))
                .await
                .is_none()
        );
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

    #[tokio::test]
    async fn evaluation_publishes_scoped_decision_event() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".envrc"), "export SAFE=1").unwrap();
        let bus = Arc::new(CanonicalEventBus::new(16));
        let mut events =
            bus.subscribe_channel(SchemaId::from("aletheon.event.workspace_trust_decided/v1"));
        let resolver = WorkspaceTrustResolver::new(
            Arc::new(InMemoryTrustStore::default()),
            Arc::new(KnownConfigDiscoverer::default()),
            true,
        )
        .with_event_bus(bus);

        let decision = resolver
            .evaluate(
                PrincipalId("alice".into()),
                identity(temp.path()),
                ClientMode::Headless,
                false,
                1,
            )
            .await;
        assert!(matches!(
            decision,
            WorkspaceTrustDecision::Restricted { .. }
        ));
        let event = events.recv().await.unwrap();
        assert_eq!(event.payload["principal_id"], "alice");
        assert_eq!(event.payload["decision"], "restricted");
        assert_eq!(
            event.payload["workspace"],
            temp.path().to_string_lossy().as_ref()
        );
    }

    #[tokio::test]
    async fn headless_untrusted_repo_blocks_loader_but_keeps_normal_files_usable() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".envrc"), "run-untrusted-command").unwrap();
        let resolver = WorkspaceTrustResolver::new(
            Arc::new(InMemoryTrustStore::default()),
            Arc::new(KnownConfigDiscoverer::default()),
            true,
        );

        let decision = resolver
            .evaluate(
                PrincipalId("headless-user".into()),
                identity(temp.path()),
                ClientMode::Headless,
                false,
                1,
            )
            .await;
        assert!(!source_is_granted(
            &decision,
            ExecutableConfigSource::EnvrcLoader
        ));

        let ordinary = temp.path().join("ordinary.txt");
        std::fs::write(&ordinary, "still writable").unwrap();
        assert_eq!(std::fs::read_to_string(ordinary).unwrap(), "still writable");
    }
}
