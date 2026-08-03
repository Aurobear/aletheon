# Extension Runtime Activation Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete Aletheon's existing package lifecycle so enabled Skill, Hook, MCP Connector, Agent Profile, and Executable assets are validated and atomically published by the running daemon.

**Architecture:** A daemon-owned `ExtensionCoordinator` serializes mutations, compiles immutable candidate snapshots from Package Store activation records, probes them, and swaps one shared `ExtensionRuntimeView`. Offline inspect/validate remain local; all mutations use authenticated versioned RPC. Package files are always read from content-addressed store roots and are never copied into legacy directories.

**Tech Stack:** Rust, Tokio `Mutex`/`RwLock`, serde/schemars versioned RPC, existing PackageStore/SkillLoader/HookLoader/McpManager/AgentLoader, SHA-256 receipts, Unix-socket daemon.

**Approved design:** `docs/plans/2026-08-03-governed-commands-extension-runtime-design.md:140-336`

---

### Task 1: Define versioned Connector and extension RPC contracts

**Files:**
- Create: `crates/fabric/src/protocol/extension.rs`
- Modify: `crates/fabric/src/protocol/mod.rs`
- Modify: `crates/fabric/src/protocol/client.rs`
- Test: `crates/fabric/tests/extension_runtime_protocol.rs`

- [ ] **Step 1: Write failing schema and serialization tests**

```rust
use fabric::protocol::extension::*;

#[test]
fn connector_rejects_inline_secret_values() {
    let raw = serde_json::json!({
        "schema_version": 1,
        "id": "aurb.gbrain",
        "transport": {"kind":"streamable_http","url":"http://127.0.0.1:3131/mcp"},
        "bearer_token_env": "Bearer literal-token"
    });
    let parsed = serde_json::from_value::<McpConnectorManifestV1>(raw).unwrap();
    assert!(parsed.validate().is_err());
}

#[test]
fn extension_enable_serializes_versioned_rpc() {
    let request = fabric::protocol::client::ClientRpcRequest::ExtensionEnable(
        ExtensionEnableRequestV1 {
            schema_version: 1,
            package_id: "aurb.core".into(),
            approve_permissions: true,
        },
    ).to_json_rpc(Some(9)).unwrap();
    assert_eq!(request["method"], "extension.enable");
    assert_eq!(request["params"]["schema_version"], 1);
}
```

- [ ] **Step 2: Run and verify the tests fail to compile**

```bash
bash scripts/cargo-agent.sh test -p fabric --test extension_runtime_protocol -- --nocapture
```

Expected: compile failure because `protocol::extension` does not exist.

- [ ] **Step 3: Add the exact public types**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionEnableRequestV1 {
    pub schema_version: u16,
    pub package_id: String,
    pub approve_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionPackagePathRequestV1 {
    pub schema_version: u16,
    pub path: PathBuf,
    pub trust_workspace: bool,
    pub approve_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionPackageIdRequestV1 {
    pub schema_version: u16,
    pub package_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionMutationReceiptV1 {
    pub schema_version: u16,
    pub operation: String,
    pub actor: String,
    pub package_id: String,
    pub package_version: Option<String>,
    pub package_hash: Option<String>,
    pub previous_snapshot_digest: String,
    pub snapshot_digest: String,
    pub permission_approved: bool,
    pub health: String,
    pub evidence_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpConnectorTransportV1 {
    Stdio { command: String, args: Vec<String> },
    StreamableHttp { url: String },
    Sse { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct McpConnectorManifestV1 {
    pub schema_version: u16,
    pub id: String,
    pub transport: McpConnectorTransportV1,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub allowed_resources: Vec<String>,
}
```

`validate()` requires schema version 1, a nonempty ID, timeout `1..=120_000`,
package-relative stdio commands under `payload/`, HTTP(S) URLs for network
transports, and bearer-token environment names matching
`[A-Z][A-Z0-9_]*`. Values containing whitespace, `Bearer `, `=`, or URL
credentials are rejected.

Add `ClientRpcRequest` variants and mappings for:

```text
extension.install    ExtensionPackagePathRequestV1
extension.enable     ExtensionEnableRequestV1
extension.disable    ExtensionPackageIdRequestV1
extension.upgrade    ExtensionPackagePathRequestV1
extension.rollback   ExtensionPackageIdRequestV1
extension.remove     ExtensionPackageIdRequestV1
extension.purge      ExtensionPackageIdRequestV1
extension.list       no params
extension.show       ExtensionPackageIdRequestV1
extension.doctor     ExtensionPackageIdRequestV1
```

- [ ] **Step 4: Run tests and commit**

```bash
bash scripts/cargo-agent.sh test -p fabric --test extension_runtime_protocol -- --nocapture
```

Expected: pass.

```bash
git add crates/fabric/src/protocol/extension.rs crates/fabric/src/protocol/mod.rs crates/fabric/src/protocol/client.rs crates/fabric/tests/extension_runtime_protocol.rs
git commit -F - <<'MSG'
feat(protocol): define governed extension mutations

Extension lifecycle mutations need authenticated daemon contracts and package
connectors need a secret-safe versioned manifest.

- add extension mutation request and receipt schemas
- define validated MCP connector transport contracts
- bind extension lifecycle methods to typed client requests
MSG
```

### Task 2: Resolve package assets without activating them

**Files:**
- Create: `crates/corpus/src/extension/resolver.rs`
- Modify: `crates/corpus/src/extension/mod.rs`
- Modify: `crates/corpus/src/skill/loader.rs:160-166`
- Modify: `crates/corpus/src/hook/loader.rs:86-144`
- Modify: `crates/corpus/src/hook/registry.rs:70-120,260-285`
- Test: `crates/corpus/tests/package_asset_resolver.rs`

- [ ] **Step 1: Write a mixed-package fixture test**

```rust
#[test]
fn resolves_assets_in_stable_kind_and_id_order() {
    let fixture = PackageFixture::new()
        .skill("skill.review", "assets/skills/review/SKILL.md")
        .hook("hook.audit", "assets/hooks/audit.toml")
        .connector("connector.gbrain", "assets/connectors/gbrain.json")
        .profile("agent.reviewer", "assets/agents/reviewer.md")
        .finish();
    let resolved = PackageAssetResolver::new(fixture.store()).resolve_enabled().unwrap();
    assert_eq!(resolved.asset_ids(), [
        "agent.reviewer", "connector.gbrain", "hook.audit", "skill.review"
    ]);
    assert!(resolved.iter().all(|asset| asset.path().starts_with(fixture.package_root())));
}
```

- [ ] **Step 2: Run and confirm the test fails to compile**

```bash
bash scripts/cargo-agent.sh test -p corpus --test package_asset_resolver -- --nocapture
```

Expected: compile failure because `PackageAssetResolver` is absent.

- [ ] **Step 3: Add path-based parsers and resolver types**

Expose these existing parsers without duplicating parsing logic:

```rust
pub fn load_skill_dir(dir: &Path) -> anyhow::Result<(LoadedSkill, SkillPlugin)>;
pub fn load_hook_path(path: &Path, package_root: Option<&Path>) -> anyhow::Result<HookConfig>;
```

For package Hooks, `load_hook_path` resolves a relative script against the
package root, canonicalizes both paths, and rejects scripts outside the root.
Legacy absolute Hook scripts remain accepted only when `package_root` is
`None`.

Add `timeout_ms: Option<u64>` to `HookConfig` and `RegisteredHook`. Parse
`timeout_ms` from package Hook manifests, require `1..=120_000`, and use
`hook.timeout_ms.map(Duration::from_millis).unwrap_or(self.execution_timeout)`
for process execution. Existing built-in/config Hook literals set it to `None`.

Create:

```rust
pub struct ResolvedPackageAsset {
    pub package_id: String,
    pub package_version: String,
    pub package_hash: String,
    pub asset: fabric::types::extension_package::AssetRef,
    pub absolute_path: PathBuf,
}

pub struct ResolvedPackageSet {
    pub assets: Vec<ResolvedPackageAsset>,
    pub activation_records: Vec<ActivationRecord>,
}

pub struct PackageAssetResolver {
    store: PackageStore,
}
```

`resolve_enabled()` reads enabled activations, requires their selected hash and
installed projection to exist, joins declared paths below the canonical package
root, verifies each file, and sorts by `(kind, id, package_id, hash)`.

- [ ] **Step 4: Run Corpus tests and commit**

```bash
bash scripts/cargo-agent.sh test -p corpus --test package_asset_resolver -- --nocapture
bash scripts/cargo-agent.sh test -p corpus --lib extension:: -- --nocapture
```

Expected: pass.

```bash
git add crates/corpus/src/extension/resolver.rs crates/corpus/src/extension/mod.rs crates/corpus/src/skill/loader.rs crates/corpus/src/hook/loader.rs crates/corpus/src/hook/registry.rs crates/corpus/tests/package_asset_resolver.rs
git commit -F - <<'MSG'
feat(corpus): resolve enabled package assets

Installed package metadata must be converted into canonical, contained asset
paths before any runtime registry sees it.

- resolve enabled package projections deterministically
- expose shared Skill and Hook path parsers
- reject missing, escaping, and conflicting asset paths
MSG
```

### Task 3: Compile immutable extension snapshots

**Files:**
- Create: `crates/executive/src/application/extension_snapshot.rs`
- Create: `crates/executive/tests/extension_snapshot.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Modify: `crates/executive/Cargo.toml`

- [ ] **Step 1: Write digest and conflict tests**

```rust
#[test]
fn snapshot_digest_is_order_independent() {
    let a = fixture_snapshot(["skill.z", "hook.a"]);
    let b = fixture_snapshot(["hook.a", "skill.z"]);
    assert_eq!(a.digest, b.digest);
}

#[test]
fn duplicate_public_names_fail_closed() {
    let error = compile_fixture([
        asset("one", "skill.review", "review"),
        asset("two", "skill.other", "review"),
    ]).unwrap_err();
    assert!(error.to_string().contains("duplicate public skill name 'review'"));
}
```

- [ ] **Step 2: Run and confirm compilation fails**

```bash
bash scripts/cargo-agent.sh test -p executive --test extension_snapshot -- --nocapture
```

Expected: compile failure because the snapshot module is absent.

- [ ] **Step 3: Implement the candidate snapshot**

```rust
#[derive(Clone)]
pub struct ExtensionRuntimeSnapshot {
    pub digest: String,
    pub skills: Arc<Vec<corpus::skill::loader::LoadedSkill>>,
    pub skill_plugins: Arc<Vec<corpus::skill::plugin::SkillPlugin>>,
    pub hooks: Arc<Vec<corpus::hook::loader::HookConfig>>,
    pub connectors: Arc<Vec<fabric::protocol::extension::McpConnectorManifestV1>>,
    pub agent_profile_paths: Arc<Vec<PathBuf>>,
    pub executable_assets: Arc<Vec<corpus::extension::resolver::ResolvedPackageAsset>>,
    pub package_digests: Arc<BTreeMap<String, String>>,
}

#[derive(Clone)]
pub struct ExtensionRuntimeView {
    current: Arc<RwLock<Arc<ExtensionRuntimeSnapshot>>>,
}

impl ExtensionRuntimeView {
    pub async fn load(&self) -> Arc<ExtensionRuntimeSnapshot> {
        self.current.read().await.clone()
    }
    pub async fn publish(&self, snapshot: ExtensionRuntimeSnapshot) {
        *self.current.write().await = Arc::new(snapshot);
    }
}
```

`ExtensionSnapshotCompiler::compile` parses all assets, namespaces non-public
IDs with their package ID, rejects duplicate names and config-owned MCP IDs,
serializes a canonical projection with sorted keys, and hashes it using SHA-256.
It performs no registry mutation.

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test extension_snapshot -- --nocapture
```

Expected: pass.

```bash
git add crates/executive/src/application/extension_snapshot.rs crates/executive/src/application/mod.rs crates/executive/tests/extension_snapshot.rs crates/executive/Cargo.toml
git commit -F - <<'MSG'
feat(extensions): compile immutable runtime snapshots

All package asset kinds need one canonical candidate representation so runtime
readers cannot observe partially activated extensions.

- compile Skills, Hooks, Connectors, Profiles, and runtimes together
- reject naming and administrator-config conflicts
- compute a stable content and capability digest
MSG
```

### Task 4: Add the transactional ExtensionCoordinator

**Files:**
- Create: `crates/executive/src/application/extension_coordinator.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Modify: `crates/executive/src/application/extension_install.rs`
- Modify: `crates/executive/src/application/extension_manage.rs`
- Test: `crates/executive/tests/extension_coordinator.rs`

- [ ] **Step 1: Write atomic-failure and serialization tests**

```rust
#[tokio::test]
async fn failed_upgrade_keeps_old_snapshot() {
    let fixture = CoordinatorFixture::with_enabled_skill("pkg", "1.0.0").await;
    let before = fixture.view.load().await.digest.clone();
    let result = fixture.coordinator.upgrade(
        actor(), fixture.invalid_upgrade(), false, false
    ).await;
    assert!(result.is_err());
    assert_eq!(fixture.view.load().await.digest, before);
}

#[tokio::test]
async fn mutations_return_snapshot_bound_receipts() {
    let fixture = CoordinatorFixture::empty().await;
    let receipt = fixture.coordinator.install(actor(), fixture.package(), false).await.unwrap();
    assert_eq!(receipt.operation, "install");
    assert!(!receipt.snapshot_digest.is_empty());
    assert_eq!(receipt.schema_version, 1);
}
```

- [ ] **Step 2: Run and confirm compilation fails**

```bash
bash scripts/cargo-agent.sh test -p executive --test extension_coordinator -- --nocapture
```

Expected: compile failure because `ExtensionCoordinator` is absent.

- [ ] **Step 3: Implement the coordinator boundary**

```rust
pub struct ExtensionCoordinator {
    mutations: tokio::sync::Mutex<()>,
    install: ExtensionInstallService,
    manage: ExtensionManageService,
    compiler: ExtensionSnapshotCompiler,
    runtime: ExtensionRuntimePublisher,
    view: ExtensionRuntimeView,
    clock: Arc<dyn fabric::Clock>,
}
```

Every mutation acquires `mutations`, records the old digest, performs the
Package Store transaction, compiles and probes a candidate, then publishes it.
If compilation/probe fails after an activation change, restore the previous
activation record before releasing the lock. Produce
`ExtensionMutationReceiptV1` for success and append an evidence event for both
success and failure. `install` stores only and recompiles to prove the active
digest did not change.

Define an `ExtensionRuntimePublisher` trait:

```rust
#[async_trait]
pub trait ExtensionRuntimePublisher: Send + Sync {
    async fn probe(&self, candidate: &ExtensionRuntimeSnapshot) -> anyhow::Result<()>;
    async fn publish(
        &self,
        previous: Arc<ExtensionRuntimeSnapshot>,
        candidate: Arc<ExtensionRuntimeSnapshot>,
    ) -> anyhow::Result<()>;
}
```

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test extension_coordinator -- --nocapture
```

Expected: pass.

```bash
git add crates/executive/src/application/extension_coordinator.rs crates/executive/src/application/mod.rs crates/executive/src/application/extension_install.rs crates/executive/src/application/extension_manage.rs crates/executive/tests/extension_coordinator.rs
git commit -F - <<'MSG'
feat(extensions): coordinate atomic package activation

Direct lifecycle calls persist state without proving that the running daemon can
publish the resulting assets.

- serialize extension mutations at one application boundary
- compile and probe candidates before publication
- restore prior activation and snapshot on failure
- emit snapshot-bound mutation receipts
MSG
```

### Task 5: Publish Skill and Hook assets dynamically

**Files:**
- Create: `crates/executive/src/host/daemon/bootstrap/extension_publisher.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs:496-535`
- Modify: `crates/executive/src/application/admin_service.rs:413-426,595-810`
- Modify: `crates/corpus/src/hook/registry.rs`
- Modify: `crates/corpus/src/tools/tools/registry.rs`
- Test: `crates/executive/tests/package_skill_hook_runtime.rs`

- [ ] **Step 1: Write live enable/disable tests**

```rust
#[tokio::test]
async fn enabled_package_skill_and_hook_are_visible_without_restart() {
    let daemon = RuntimeFixture::start().await;
    daemon.install_and_enable(skill_hook_package()).await;
    assert!(daemon.skills().await.iter().any(|skill| skill.id == "aurb:review"));
    daemon.emit_post_turn().await;
    assert_eq!(daemon.hook_receipts("aurb:audit").await.len(), 1);
    daemon.disable("aurb.core").await;
    assert!(!daemon.skills().await.iter().any(|skill| skill.id == "aurb:review"));
}
```

- [ ] **Step 2: Run and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_skill_hook_runtime -- --nocapture
```

Expected: compile failure until the publisher is present.

- [ ] **Step 3: Implement replaceable package-owned registrations**

Add owner-aware registry operations:

```rust
pub fn replace_package_tools(&mut self, owner: &str, tools: Vec<Arc<dyn Tool>>) -> Result<()>;
pub fn replace_package_hooks(&mut self, owner: &str, hooks: Vec<RegisteredHook>);
pub fn remove_package_hooks(&mut self, owner: &str);
```

Built-ins and legacy entries have distinct owners and cannot be removed by a
package. `DaemonExtensionRuntimePublisher` builds all new Skill tools and Hook
registrations first, then swaps owner-indexed maps under their existing locks.
`AdminService::list_skills` reads `ExtensionRuntimeView`, so `/skills` changes
immediately after publication.

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_skill_hook_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p corpus --lib -- --nocapture
```

Expected: pass.

```bash
git add crates/executive/src/host/daemon/bootstrap/extension_publisher.rs crates/executive/src/host/daemon/bootstrap/request.rs crates/executive/src/application/admin_service.rs crates/corpus/src/hook/registry.rs crates/corpus/src/tools/tools/registry.rs crates/executive/tests/package_skill_hook_runtime.rs
git commit -F - <<'MSG'
feat(extensions): activate packaged Skills and Hooks

Package Skill and Hook assets currently remain inert even after enablement.

- publish package-owned Skill tools and Hook registrations atomically
- protect built-in and legacy registry owners
- update Skill inventory immediately on enable and disable
MSG
```

### Task 6: Publish MCP Connector assets

**Files:**
- Create: `crates/executive/src/host/daemon/bootstrap/extension_connectors.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/extension_publisher.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs:395-449,537-565`
- Modify: `crates/corpus/src/tools/mcp/config.rs`
- Modify: `crates/corpus/src/tools/mcp/manager.rs`
- Test: `crates/executive/tests/package_mcp_runtime.rs`

- [ ] **Step 1: Add real test-server acceptance**

```rust
#[tokio::test]
async fn packaged_mcp_connects_and_is_removed_on_disable() {
    let server = McpFixture::start().await;
    let daemon = RuntimeFixture::start_with_secret("AURB_TEST_TOKEN", server.token()).await;
    daemon.install_and_enable(connector_package(server.url())).await;
    assert!(daemon.tools().await.contains(&"aurb_search".to_string()));
    assert_eq!(daemon.call_tool("aurb_search", json!({"q":"x"})).await["ok"], true);
    daemon.disable("aurb.core").await;
    assert!(!daemon.tools().await.contains(&"aurb_search".to_string()));
}
```

- [ ] **Step 2: Run and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_mcp_runtime -- --nocapture
```

Expected: compile failure until connector projection exists.

- [ ] **Step 3: Resolve connectors into MCP configuration**

`extension_connectors.rs` maps manifest transports to `McpServerConfig`, resolves
only named host secrets, prefixes registration ownership with the package ID,
and creates a candidate `McpManager`. `probe` must connect all candidate
package servers and enumerate filtered tools/resources before publication.
Administrator-configured MCP IDs are reserved. Publication unregisters the old
package-owned tool IDs, registers the new wrappers, and swaps the retained
package-manager set. A failure restores old registrations.

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_mcp_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p corpus --lib tools::mcp -- --nocapture
```

Expected: pass.

```bash
git add crates/executive/src/host/daemon/bootstrap/extension_connectors.rs crates/executive/src/host/daemon/bootstrap/extension_publisher.rs crates/executive/src/host/daemon/bootstrap/request.rs crates/corpus/src/tools/mcp/config.rs crates/corpus/src/tools/mcp/manager.rs crates/executive/tests/package_mcp_runtime.rs
git commit -F - <<'MSG'
feat(extensions): activate packaged MCP connectors

Connector assets need secret-safe projection into the existing MCP manager and
must not override administrator configuration.

- resolve connector manifests into candidate MCP clients
- probe tools and resources before snapshot publication
- register and remove package-owned MCP capabilities atomically
MSG
```

### Task 7: Publish Agent Profiles and unify executable runtimes

**Files:**
- Modify: `crates/executive/src/host/daemon/bootstrap/runtime.rs:61-140`
- Modify: `crates/executive/src/host/daemon/bootstrap/agents.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/extensions.rs:19-325`
- Modify: `crates/executive/src/host/daemon/bootstrap/extension_publisher.rs`
- Modify: `crates/executive/src/adapters/runtime/native_cognit.rs:50-100`
- Test: `crates/executive/tests/package_profile_runtime.rs`

- [ ] **Step 1: Add dependency and rollback tests**

```rust
#[tokio::test]
async fn profile_requires_capabilities_from_same_candidate_snapshot() {
    let daemon = RuntimeFixture::start().await;
    let error = daemon.enable(profile_with_missing_tool()).await.unwrap_err();
    assert!(error.to_string().contains("unknown tool 'aurb_search'"));
    assert!(!daemon.profiles().await.contains(&"aurb:reviewer".to_string()));
}

#[tokio::test]
async fn executable_probe_failure_keeps_previous_snapshot() {
    let daemon = RuntimeFixture::with_known_good_runtime().await;
    let before = daemon.snapshot_digest().await;
    assert!(daemon.upgrade(failing_runtime_package()).await.is_err());
    assert_eq!(daemon.snapshot_digest().await, before);
}
```

- [ ] **Step 2: Run and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_profile_runtime -- --nocapture
```

Expected: compile failure until package profile publication exists.

- [ ] **Step 3: Make profile and runtime registries snapshot-aware**

Expose a path-list input on `load_agent_profiles` rather than copying files:

```rust
pub(super) async fn load_agent_profiles_from_paths(
    profile_paths: &[PathBuf],
    inference: Arc<dyn InferencePort>,
    default_llm: Arc<dyn LlmProvider>,
    definitions: &[ToolDefinition],
    profile_definitions: &[ToolDefinition],
    config: &ExecutiveConfig,
    profiles_config: &AgentProfilesConfig,
) -> anyhow::Result<ProfileLoadResult>;
```

The publisher validates profile dependencies against candidate tool definitions,
then replaces package-owned profile entries. Move existing executable runtime
preparation behind `ExtensionRuntimePublisher::probe` and publication behind
`publish`, preserving sandbox, permission approval, quarantine, and
previous-known-good behavior.

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test package_profile_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test evolution_integration -- --nocapture
```

Expected: pass.

```bash
git add crates/executive/src/host/daemon/bootstrap/runtime.rs crates/executive/src/host/daemon/bootstrap/agents.rs crates/executive/src/host/daemon/bootstrap/extensions.rs crates/executive/src/host/daemon/bootstrap/extension_publisher.rs crates/executive/src/adapters/runtime/native_cognit.rs crates/executive/tests/package_profile_runtime.rs
git commit -F - <<'MSG'
feat(extensions): activate Profiles and runtimes together

Agent Profiles must resolve against the same capability snapshot as packaged
Skills, MCP tools, and executable runtimes.

- load package Profiles directly from content-addressed paths
- validate Profile dependencies against candidate capabilities
- unify executable probe and rollback with snapshot publication
MSG
```

### Task 8: Route lifecycle CLI mutations through the daemon

**Files:**
- Create: `crates/aletheon/src/extension_cli.rs`
- Modify: `crates/aletheon/src/main.rs:403-517,620-625`
- Modify: `crates/executive/src/host/daemon/handler/ports.rs`
- Create: `crates/executive/src/host/daemon/handler/rpc/rpc_extension.rs`
- Modify: `crates/executive/src/host/daemon/handler/rpc.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs:1507-1533`
- Test: `crates/aletheon/tests/extension_cli_rpc.rs`
- Test: `crates/executive/tests/extension_rpc.rs`

- [ ] **Step 1: Write RPC-routing tests**

```rust
#[tokio::test]
async fn enable_cli_uses_official_socket_not_direct_store() {
    let server = RpcCapture::start(json!({"schema_version":1,"operation":"enable",
        "actor":"local-owner","package_id":"aurb.core","package_version":"1.0.0",
        "package_hash":"aa","previous_snapshot_digest":"old","snapshot_digest":"new",
        "permission_approved":true,"health":"healthy","evidence_references":[]})).await;
    run_cli(["extension", "enable", "aurb.core", "--approve-permissions"], server.socket()).await.unwrap();
    assert_eq!(server.last_method().await, "extension.enable");
}
```

- [ ] **Step 2: Confirm failure**

```bash
bash scripts/cargo-agent.sh test -p aletheon --test extension_cli_rpc -- --nocapture
```

Expected: failure because the CLI opens the Package Store locally.

- [ ] **Step 3: Add coordinator port and handlers**

Add `extensions: Arc<ExtensionCoordinator>` to `HandlerPorts`. Implement
`rpc_extension.rs` handlers that deserialize the exact Fabric request, reject
schema versions other than 1, derive actor identity from authenticated
`ConnectionContext.principal_id`, and return the coordinator receipt. Register
all ten methods from Task 1 in `rpc.rs`.

Move extension CLI transport into `extension_cli.rs`. `inspect` and `validate`
remain local and read-only; list/show/doctor and all mutations call the official
user socket. Remove direct mutation service construction from `main.rs`.

- [ ] **Step 4: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p aletheon --test extension_cli_rpc -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test extension_rpc -- --nocapture
```

Expected: pass.

```bash
git add crates/aletheon/src/extension_cli.rs crates/aletheon/src/main.rs crates/executive/src/host/daemon/handler/ports.rs crates/executive/src/host/daemon/handler/rpc/rpc_extension.rs crates/executive/src/host/daemon/handler/rpc.rs crates/executive/src/host/daemon/bootstrap/request.rs crates/aletheon/tests/extension_cli_rpc.rs crates/executive/tests/extension_rpc.rs
git commit -F - <<'MSG'
feat(extensions): route lifecycle through daemon authority

A standalone CLI must not mutate package activation behind the running daemon's
runtime snapshot.

- authenticate extension lifecycle RPCs on the official socket
- derive actors from host connection identity
- keep only offline inspect and validate operations local
MSG
```

### Task 9: Recover deterministic snapshots on restart

**Files:**
- Modify: `crates/executive/src/host/daemon/bootstrap/extensions.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/application/health.rs`
- Test: `crates/executive/tests/extension_restart_recovery.rs`

- [ ] **Step 1: Add restart/quarantine test**

```rust
#[tokio::test]
async fn restart_rolls_back_bad_package_without_losing_healthy_packages() {
    let fixture = RestartFixture::with_healthy_and_broken_packages().await;
    let first = fixture.start_daemon().await;
    assert!(first.skills().contains(&"healthy:review".into()));
    assert!(!first.skills().contains(&"broken:review".into()));
    assert_eq!(first.health()["extensions"]["quarantined_count"], 1);
    let digest = first.snapshot_digest();
    drop(first);
    assert_eq!(fixture.start_daemon().await.snapshot_digest(), digest);
}
```

- [ ] **Step 2: Implement bootstrap reconciliation**

At bootstrap, compile packages independently in stable package-ID order. For a
failed current version, try `previous_known_good`; if that fails, disable and
quarantine only that package. Compile and publish the remaining packages.
Expose active package count, asset counts by kind, snapshot digest,
quarantined IDs, and rollback count in health/status.

- [ ] **Step 3: Run and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test extension_restart_recovery -- --nocapture
```

Expected: pass twice consecutively with the same snapshot digest.

```bash
git add crates/executive/src/host/daemon/bootstrap/extensions.rs crates/executive/src/host/daemon/bootstrap/request.rs crates/executive/src/application/health.rs crates/executive/tests/extension_restart_recovery.rs
git commit -F - <<'MSG'
feat(extensions): recover active snapshots on restart

A broken package must not prevent daemon startup or remove unrelated healthy
extensions.

- rollback or quarantine failing packages independently
- reconstruct deterministic runtime snapshots on restart
- expose active, quarantined, and digest health evidence
MSG
```

### Task 10: Run extension runtime acceptance

**Files:**
- Create: `tests/suites/operations/extension_runtime_test.sh`
- Modify: `scripts/lib/aletheon/test.sh:11-20`

- [ ] **Step 1: Add isolated package lifecycle acceptance**

The script builds deterministic fixtures for all five asset kinds, starts the
repository test harness under an isolated home/state root, and asserts:

```bash
aletheon extension validate "$package"
aletheon extension install "$package"
aletheon extension enable test.full --approve-permissions
aletheon extension doctor test.full
aletheon extension disable test.full
aletheon extension enable test.full --approve-permissions
aletheon extension upgrade "$upgrade" --approve-permissions
aletheon extension rollback test.full
```

It records receipts and checks that Skill, Hook, MCP, Profile, and executable
inventories change together and failed upgrades retain the previous digest.

- [ ] **Step 2: Run deterministic validation**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p fabric --test extension_runtime_protocol -- --nocapture
bash scripts/cargo-agent.sh test -p corpus --test package_asset_resolver -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test extension_coordinator -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test package_skill_hook_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test package_mcp_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test package_profile_runtime -- --nocapture
bash scripts/cargo-agent.sh test -p executive --test extension_restart_recovery -- --nocapture
bash tests/suites/operations/extension_runtime_test.sh
```

Expected: all pass.

- [ ] **Step 3: Commit acceptance suite**

```bash
git add tests/suites/operations/extension_runtime_test.sh scripts/lib/aletheon/test.sh
git commit -F - <<'MSG'
test(extensions): verify governed runtime activation

The package lifecycle is complete only when every asset kind changes through
one authoritative daemon snapshot.

- exercise install, enable, disable, upgrade, and rollback
- verify all package asset kinds and snapshot digests
- prove failed candidates preserve previous-known-good runtime
MSG
```
