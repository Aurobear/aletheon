//! Host-owned, version-bound single-Agent change transaction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::change_transaction::{
    ActiveCommandLease, ChangeTransactionId, ChangeTransactionPhase, ChangeTransactionSnapshot,
    ChangedRange, ValidationImpact, ValidationPlanOmission, ValidationPlanStep, ValidationRisk,
    VersionedValidationReceipt, WorkFailure, WorkFailureClass, WorkspaceVersion,
};
use fabric::repository::RepositoryContext;
use serde_json::json;
use tokio::sync::Mutex;

use crate::tools::artifact::ArtifactStore;

use super::workspace_version;
use super::{
    apply_patch::ApplyPatchTool, file_write::FileWriteTool, repo_inspect::RepoInspectTool,
    ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};

#[derive(Clone, Default)]
pub struct ChangeTransactionRegistry {
    transactions: Arc<Mutex<HashMap<ChangeTransactionId, ChangeTransactionSnapshot>>>,
    repository_contexts: Arc<Mutex<HashMap<ChangeTransactionId, RepositoryContext>>>,
    restore_points: Arc<Mutex<HashMap<ChangeTransactionId, BaselineRestore>>>,
}

#[derive(Clone)]
struct BaselineRestore {
    basis: fabric::change_transaction::WorkspaceVersionBasis,
    nodes: HashMap<String, RestoreNode>,
}

#[derive(Clone, PartialEq, Eq)]
enum RestoreNode {
    File { bytes: Vec<u8>, mode: u32 },
    Symlink(PathBuf),
    Missing,
}

impl ChangeTransactionRegistry {
    pub async fn begin(
        &self,
        owner_session_id: &str,
        root: &Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        self.begin_for_agent(owner_session_id, None, root).await
    }

    async fn begin_for_agent(
        &self,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        let baseline = workspace_version::capture(root)?;
        let restore = BaselineRestore::capture(Path::new(&baseline.root), &baseline)?;
        let snapshot = ChangeTransactionSnapshot {
            transaction_id: ChangeTransactionId::new(),
            owner_session_id: owner_session_id.into(),
            owner_agent,
            root: baseline.root.clone(),
            baseline: baseline.clone(),
            current: baseline,
            phase: ChangeTransactionPhase::Baseline,
            changed_paths: Vec::new(),
            changed_ranges: Vec::new(),
            diff_artifact_ref: None,
            validation_plan: Vec::new(),
            validation_omissions: Vec::new(),
            validation_impact: ValidationImpact::NonCode,
            validation_risk: ValidationRisk::Low,
            validation_receipts: Vec::new(),
            accepted_workspace_version: None,
            active_command: None,
            failure: None,
        };
        self.transactions
            .lock()
            .await
            .insert(snapshot.transaction_id, snapshot.clone());
        self.restore_points
            .lock()
            .await
            .insert(snapshot.transaction_id, restore);
        Ok(snapshot)
    }

    pub async fn begin_with_context(
        &self,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        context: RepositoryContext,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        let snapshot = self
            .begin_for_agent(owner_session_id, owner_agent, Path::new(&context.root))
            .await?;
        self.repository_contexts
            .lock()
            .await
            .insert(snapshot.transaction_id, context);
        Ok(snapshot)
    }

    pub async fn verify_current(
        &self,
        transaction_id: ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &Path,
    ) -> Result<ChangeTransactionSnapshot, WorkFailure> {
        let snapshot = self
            .transactions
            .lock()
            .await
            .get(&transaction_id)
            .cloned()
            .ok_or_else(|| invalid_request("unknown change transaction"))?;
        if snapshot.owner_session_id != owner_session_id {
            return Err(WorkFailure {
                class: WorkFailureClass::Permission,
                summary: "change transaction belongs to another authenticated session".into(),
                retryable: false,
            });
        }
        if snapshot.owner_agent != owner_agent {
            return Err(WorkFailure {
                class: WorkFailureClass::Permission,
                summary: "change transaction belongs to a different Agent runtime owner".into(),
                retryable: false,
            });
        }
        if let Some(active) = &snapshot.active_command {
            return Err(WorkFailure {
                class: WorkFailureClass::ConcurrentModification,
                summary: format!(
                    "managed command session {} still owns the transaction workspace lease",
                    active.session_id
                ),
                retryable: true,
            });
        }
        let root = root
            .canonicalize()
            .map_err(|error| invalid_request(&format!("invalid transaction root: {error}")))?;
        if snapshot.root != root.display().to_string() {
            return Err(invalid_request(
                "change transaction root does not match tool root",
            ));
        }
        let observed = workspace_version::capture(&root).map_err(|error| WorkFailure {
            class: WorkFailureClass::Environment,
            summary: format!("workspace version capture failed: {error}"),
            retryable: true,
        })?;
        if observed.digest != snapshot.current.digest {
            let failure = WorkFailure {
                class: WorkFailureClass::ConcurrentModification,
                summary: format!(
                    "workspace changed outside transaction: expected {}, observed {}",
                    snapshot.current.digest, observed.digest
                ),
                retryable: true,
            };
            self.mark_conflicted(transaction_id, failure.clone()).await;
            return Err(failure);
        }
        Ok(snapshot)
    }

    pub async fn reserve_command(
        &self,
        transaction_id: ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &Path,
        command_session_id: String,
        purpose: String,
    ) -> Result<ChangeTransactionSnapshot, WorkFailure> {
        let verified = self
            .verify_current(transaction_id, owner_session_id, owner_agent, root)
            .await?;
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| invalid_request("unknown change transaction"))?;
        if snapshot.active_command.is_some() {
            return Err(WorkFailure {
                class: WorkFailureClass::ConcurrentModification,
                summary: "another managed command already owns the transaction workspace lease"
                    .into(),
                retryable: true,
            });
        }
        if purpose == "validation" && snapshot.phase != ChangeTransactionPhase::DiffReviewed {
            return Err(invalid_request(
                "validation command lease requires a reviewed transaction diff",
            ));
        }
        if matches!(
            snapshot.phase,
            ChangeTransactionPhase::Accepted
                | ChangeTransactionPhase::RolledBack
                | ChangeTransactionPhase::Conflicted
        ) {
            return Err(invalid_request(
                "terminal or conflicted transaction cannot start a managed command",
            ));
        }
        snapshot.active_command = Some(ActiveCommandLease {
            session_id: command_session_id,
            purpose,
            workspace_version: verified.current.digest,
        });
        Ok(snapshot.clone())
    }

    pub async fn release_command(
        &self,
        transaction_id: ChangeTransactionId,
        command_session_id: &str,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
        require_command_lease(snapshot, command_session_id)?;
        snapshot.active_command = None;
        Ok(snapshot.clone())
    }

    pub async fn record_apply(
        &self,
        transaction_id: ChangeTransactionId,
        mut current: WorkspaceVersion,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        if current.basis == fabric::change_transaction::WorkspaceVersionBasis::BoundedTree {
            if let Some(restore) = self
                .restore_points
                .lock()
                .await
                .get(&transaction_id)
                .cloned()
            {
                current.changed_paths =
                    restore.changed_paths(Path::new(&current.root), &current)?;
            }
        }
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
        snapshot.changed_paths = current.changed_paths.clone();
        snapshot.current = current;
        let validation = self
            .repository_contexts
            .lock()
            .await
            .get(&transaction_id)
            .map(|context| derive_validation_plan(context, &snapshot.changed_paths))
            .unwrap_or_default();
        snapshot.validation_plan = validation.steps;
        snapshot.validation_omissions = validation.omissions;
        snapshot.validation_impact = validation.impact;
        snapshot.validation_risk = validation.risk;
        snapshot.phase = ChangeTransactionPhase::Applied;
        snapshot.diff_artifact_ref = None;
        snapshot.changed_ranges.clear();
        snapshot.validation_receipts.clear();
        snapshot.accepted_workspace_version = None;
        snapshot.active_command = None;
        snapshot.failure = None;
        Ok(snapshot.clone())
    }

    pub async fn record_shell_terminal(
        &self,
        transaction_id: ChangeTransactionId,
        command_session_id: &str,
        current: WorkspaceVersion,
        terminal_status: &str,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        {
            let transactions = self.transactions.lock().await;
            let stored = transactions
                .get(&transaction_id)
                .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
            require_command_lease(stored, command_session_id)?;
        }
        let mut snapshot = self.record_apply(transaction_id, current).await?;
        if terminal_status != "succeeded" {
            let mut transactions = self.transactions.lock().await;
            let stored = transactions
                .get_mut(&transaction_id)
                .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
            stored.phase = ChangeTransactionPhase::Repair;
            stored.failure = Some(WorkFailure {
                class: match terminal_status {
                    "timed_out" => WorkFailureClass::Timeout,
                    "cancelled" => WorkFailureClass::Environment,
                    _ => WorkFailureClass::Unknown,
                },
                summary: format!(
                    "workspace-changing managed command ended with {terminal_status}; inspect terminal artifact before recovery"
                ),
                retryable: true,
            });
            snapshot = stored.clone();
        }
        Ok(snapshot)
    }

    #[cfg(test)]
    pub(crate) async fn set_validation_plan(
        &self,
        transaction_id: ChangeTransactionId,
        plan: Vec<ValidationPlanStep>,
    ) {
        if let Some(snapshot) = self.transactions.lock().await.get_mut(&transaction_id) {
            snapshot.validation_plan = plan;
        }
    }

    pub async fn record_diff_review(
        &self,
        transaction_id: ChangeTransactionId,
        artifact_ref: String,
        changed_ranges: Vec<ChangedRange>,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
        if !matches!(
            snapshot.phase,
            ChangeTransactionPhase::Applied | ChangeTransactionPhase::Repair
        ) {
            anyhow::bail!("diff review requires an applied or repair transaction");
        }
        snapshot.diff_artifact_ref = Some(artifact_ref);
        snapshot.changed_ranges = changed_ranges;
        snapshot.phase = ChangeTransactionPhase::DiffReviewed;
        Ok(snapshot.clone())
    }

    pub async fn snapshot(
        &self,
        transaction_id: ChangeTransactionId,
    ) -> Option<ChangeTransactionSnapshot> {
        self.transactions.lock().await.get(&transaction_id).cloned()
    }

    async fn bounded_diff(
        &self,
        transaction_id: ChangeTransactionId,
        snapshot: &ChangeTransactionSnapshot,
    ) -> anyhow::Result<Vec<u8>> {
        let restore = self
            .restore_points
            .lock()
            .await
            .get(&transaction_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("transaction restore point is unavailable"))?;
        restore.render_diff(Path::new(&snapshot.root), &snapshot.changed_paths)
    }

    pub async fn accept(
        &self,
        transaction_id: ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &Path,
    ) -> Result<ChangeTransactionSnapshot, WorkFailure> {
        let verified = self
            .verify_current(transaction_id, owner_session_id, owner_agent, root)
            .await?;
        if verified.phase != ChangeTransactionPhase::Validated {
            return Err(invalid_request(
                "change acceptance requires successful validation of the reviewed version",
            ));
        }
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| invalid_request("unknown change transaction"))?;
        snapshot.accepted_workspace_version = Some(snapshot.current.digest.clone());
        snapshot.phase = ChangeTransactionPhase::Accepted;
        snapshot.failure = None;
        Ok(snapshot.clone())
    }

    pub async fn rollback(
        &self,
        transaction_id: ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &Path,
    ) -> Result<ChangeTransactionSnapshot, WorkFailure> {
        let verified = self
            .verify_current(transaction_id, owner_session_id, owner_agent, root)
            .await?;
        if matches!(
            verified.phase,
            ChangeTransactionPhase::Accepted | ChangeTransactionPhase::RolledBack
        ) {
            return Err(invalid_request(
                "accepted or already rolled-back transactions cannot be rolled back",
            ));
        }
        let restore = self
            .restore_points
            .lock()
            .await
            .get(&transaction_id)
            .cloned()
            .ok_or_else(|| invalid_request("transaction restore point is unavailable"))?;
        restore
            .restore(Path::new(&verified.root), &verified)
            .map_err(|error| WorkFailure {
                class: WorkFailureClass::Environment,
                summary: format!("transaction rollback failed: {error}"),
                retryable: true,
            })?;
        let observed =
            workspace_version::capture(Path::new(&verified.root)).map_err(|error| WorkFailure {
                class: WorkFailureClass::Environment,
                summary: format!("post-rollback workspace capture failed: {error}"),
                retryable: true,
            })?;
        if observed.digest != verified.baseline.digest {
            return Err(WorkFailure {
                class: WorkFailureClass::Unknown,
                summary: format!(
                    "rollback did not reproduce baseline: expected {}, observed {}",
                    verified.baseline.digest, observed.digest
                ),
                retryable: false,
            });
        }
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| invalid_request("unknown change transaction"))?;
        snapshot.current = observed;
        snapshot.changed_paths = snapshot.baseline.changed_paths.clone();
        snapshot.changed_ranges.clear();
        snapshot.phase = ChangeTransactionPhase::RolledBack;
        snapshot.diff_artifact_ref = None;
        snapshot.validation_plan.clear();
        snapshot.validation_omissions.clear();
        snapshot.validation_impact = ValidationImpact::NonCode;
        snapshot.validation_risk = ValidationRisk::Low;
        snapshot.validation_receipts.clear();
        snapshot.accepted_workspace_version = None;
        snapshot.active_command = None;
        snapshot.failure = None;
        Ok(snapshot.clone())
    }

    pub async fn record_validation(
        &self,
        transaction_id: ChangeTransactionId,
        validation_kind: String,
        command: String,
        workspace_version: String,
        observed_workspace: WorkspaceVersion,
        terminal_status: String,
        output_ref: Option<String>,
        command_session_id: &str,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        let mut transactions = self.transactions.lock().await;
        let snapshot = transactions
            .get_mut(&transaction_id)
            .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
        require_command_lease(snapshot, command_session_id)?;
        snapshot.active_command = None;
        if snapshot.phase != ChangeTransactionPhase::DiffReviewed {
            anyhow::bail!("validation requires a reviewed diff");
        }
        if snapshot.current.digest != workspace_version
            || observed_workspace.digest != workspace_version
        {
            snapshot.phase = ChangeTransactionPhase::Conflicted;
            snapshot.failure = Some(WorkFailure {
                class: WorkFailureClass::ConcurrentModification,
                summary: format!(
                    "validation workspace version mismatch: expected {workspace_version}, observed {}",
                    observed_workspace.digest
                ),
                retryable: true,
            });
            return Ok(snapshot.clone());
        }
        let Some(step) = snapshot
            .validation_plan
            .iter()
            .find(|step| step.command == command && step.validation_kind == validation_kind)
        else {
            snapshot.phase = ChangeTransactionPhase::Repair;
            snapshot.failure = Some(invalid_request(
                "validation command is not part of the host-derived validation plan",
            ));
            return Ok(snapshot.clone());
        };
        let planned_step_id = step.id.clone();
        snapshot
            .validation_receipts
            .push(VersionedValidationReceipt {
                validation_kind,
                command,
                workspace_version,
                terminal_status: terminal_status.clone(),
                output_ref,
            });
        let all_required_passed = snapshot
            .validation_plan
            .iter()
            .filter(|step| step.required)
            .all(|step| {
                snapshot.validation_receipts.iter().any(|receipt| {
                    receipt.command == step.command
                        && receipt.validation_kind == step.validation_kind
                        && receipt.workspace_version == snapshot.current.digest
                        && receipt.terminal_status == "succeeded"
                })
            });
        if terminal_status == "succeeded" && all_required_passed {
            snapshot.phase = ChangeTransactionPhase::Validated;
            snapshot.failure = None;
        } else if terminal_status == "succeeded" {
            snapshot.phase = ChangeTransactionPhase::DiffReviewed;
            snapshot.failure = None;
        } else {
            snapshot.phase = ChangeTransactionPhase::Repair;
            snapshot.failure = Some(WorkFailure {
                class: match terminal_status.as_str() {
                    "timed_out" => WorkFailureClass::Timeout,
                    "cancelled" | "truncated" => WorkFailureClass::Environment,
                    // A non-zero process exit alone does not prove whether the
                    // implementation, expectation, dependency, or environment
                    // is at fault. Preserve uncertainty until structured
                    // diagnostics provide stronger attribution.
                    _ => WorkFailureClass::Unknown,
                },
                summary: format!("validation step {planned_step_id} ended with {terminal_status}"),
                retryable: true,
            });
        }
        Ok(snapshot.clone())
    }

    async fn mark_conflicted(&self, transaction_id: ChangeTransactionId, failure: WorkFailure) {
        if let Some(snapshot) = self.transactions.lock().await.get_mut(&transaction_id) {
            snapshot.phase = ChangeTransactionPhase::Conflicted;
            snapshot.failure = Some(failure);
        }
    }
}

fn require_command_lease(
    snapshot: &ChangeTransactionSnapshot,
    command_session_id: &str,
) -> anyhow::Result<()> {
    match &snapshot.active_command {
        Some(lease) if lease.session_id == command_session_id => Ok(()),
        Some(lease) => anyhow::bail!(
            "transaction command lease belongs to session {}",
            lease.session_id
        ),
        None => anyhow::bail!("transaction has no active managed-command lease"),
    }
}

impl BaselineRestore {
    fn capture(root: &Path, baseline: &WorkspaceVersion) -> anyhow::Result<Self> {
        let paths = match baseline.basis {
            fabric::change_transaction::WorkspaceVersionBasis::GitWorktree => {
                baseline.changed_paths.clone()
            }
            fabric::change_transaction::WorkspaceVersionBasis::BoundedTree => {
                baseline.changed_paths.clone()
            }
        };
        let mut nodes = HashMap::new();
        for relative in paths {
            nodes.insert(relative.clone(), read_restore_node(&root.join(&relative))?);
        }
        Ok(Self {
            basis: baseline.basis,
            nodes,
        })
    }

    fn restore(&self, root: &Path, snapshot: &ChangeTransactionSnapshot) -> anyhow::Result<()> {
        let mut paths = snapshot.current.changed_paths.clone();
        paths.extend(snapshot.baseline.changed_paths.iter().cloned());
        paths.sort();
        paths.dedup();
        for relative in paths {
            ensure_relative_workspace_path(&relative)?;
            let node = if let Some(node) = self.nodes.get(&relative) {
                node.clone()
            } else if self.basis == fabric::change_transaction::WorkspaceVersionBasis::GitWorktree {
                git_head_node(root, &relative)?
            } else {
                RestoreNode::Missing
            };
            restore_node(&root.join(&relative), node)?;
        }
        Ok(())
    }

    fn changed_paths(
        &self,
        root: &Path,
        current: &WorkspaceVersion,
    ) -> anyhow::Result<Vec<String>> {
        let mut paths = self.nodes.keys().cloned().collect::<Vec<_>>();
        paths.extend(current.changed_paths.iter().cloned());
        paths.sort();
        paths.dedup();
        let mut changed = Vec::new();
        for relative in paths {
            ensure_relative_workspace_path(&relative)?;
            let before = self
                .nodes
                .get(&relative)
                .cloned()
                .unwrap_or(RestoreNode::Missing);
            let after = read_restore_node(&root.join(&relative))?;
            if before != after {
                changed.push(relative);
            }
        }
        Ok(changed)
    }

    fn render_diff(&self, root: &Path, changed_paths: &[String]) -> anyhow::Result<Vec<u8>> {
        let mut output = Vec::new();
        for relative in changed_paths {
            ensure_relative_workspace_path(relative)?;
            let before = self
                .nodes
                .get(relative)
                .cloned()
                .unwrap_or(RestoreNode::Missing);
            let after = read_restore_node(&root.join(relative))?;
            output.extend_from_slice(format!("--- a/{relative}\n+++ b/{relative}\n").as_bytes());
            render_node_evidence(&mut output, "before", &before);
            render_node_evidence(&mut output, "after", &after);
        }
        Ok(output)
    }
}

fn render_node_evidence(output: &mut Vec<u8>, label: &str, node: &RestoreNode) {
    use sha2::{Digest, Sha256};
    match node {
        RestoreNode::Missing => output.extend_from_slice(format!("{label}: missing\n").as_bytes()),
        RestoreNode::Symlink(target) => output
            .extend_from_slice(format!("{label}: symlink -> {}\n", target.display()).as_bytes()),
        RestoreNode::File { bytes, mode } => {
            output.extend_from_slice(
                format!(
                    "{label}: file mode={mode:o} bytes={} sha256={:x}\n",
                    bytes.len(),
                    Sha256::digest(bytes)
                )
                .as_bytes(),
            );
            if bytes.len() <= 64 * 1024 {
                if let Ok(text) = std::str::from_utf8(bytes) {
                    output.extend_from_slice(format!("{label}-content:\n{text}\n").as_bytes());
                }
            }
        }
    }
}

fn ensure_relative_workspace_path(relative: &str) -> anyhow::Result<()> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        anyhow::bail!("unsafe restore path {relative}");
    }
    Ok(())
}

fn read_restore_node(path: &Path) -> anyhow::Result<RestoreNode> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestoreNode::Missing)
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Ok(RestoreNode::Symlink(std::fs::read_link(path)?));
    }
    if metadata.is_file() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        #[cfg(unix)]
        let mode = metadata.permissions().mode();
        #[cfg(not(unix))]
        let mode = 0;
        return Ok(RestoreNode::File {
            bytes: std::fs::read(path)?,
            mode,
        });
    }
    anyhow::bail!("unsupported restore node {}", path.display())
}

fn git_head_node(root: &Path, relative: &str) -> anyhow::Result<RestoreNode> {
    ensure_relative_workspace_path(relative)?;
    let spec = format!("HEAD:{relative}");
    let content = std::process::Command::new("git")
        .args(["show", &spec])
        .current_dir(root)
        .output()?;
    if !content.status.success() {
        return Ok(RestoreNode::Missing);
    }
    let tree = std::process::Command::new("git")
        .args(["ls-tree", "HEAD", "--", relative])
        .current_dir(root)
        .output()?;
    let tree_text = String::from_utf8_lossy(&tree.stdout);
    let mode = tree_text.split_whitespace().next().unwrap_or("100644");
    if mode == "120000" {
        return Ok(RestoreNode::Symlink(PathBuf::from(String::from_utf8(
            content.stdout,
        )?)));
    }
    let permissions = if mode == "100755" { 0o755 } else { 0o644 };
    Ok(RestoreNode::File {
        bytes: content.stdout,
        mode: permissions,
    })
}

fn restore_node(path: &Path, node: RestoreNode) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    match node {
        RestoreNode::Missing => {}
        RestoreNode::File { bytes, mode } => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
            }
        }
        RestoreNode::Symlink(target) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, path)?;
            #[cfg(not(unix))]
            anyhow::bail!("symlink rollback is unsupported on this platform");
        }
    }
    Ok(())
}

struct ValidationProjection {
    steps: Vec<ValidationPlanStep>,
    omissions: Vec<ValidationPlanOmission>,
    impact: ValidationImpact,
    risk: ValidationRisk,
}

impl Default for ValidationProjection {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            omissions: Vec::new(),
            impact: ValidationImpact::NonCode,
            risk: ValidationRisk::Low,
        }
    }
}

fn derive_validation_plan(
    context: &RepositoryContext,
    changed_paths: &[String],
) -> ValidationProjection {
    let mut plan = Vec::new();
    let deployment_required = context.deployment_policy.as_ref().is_some_and(|policy| {
        policy.requires_installed_runtime
            && (policy.affected_path_prefixes.is_empty()
                || changed_paths.iter().any(|path| {
                    policy
                        .affected_path_prefixes
                        .iter()
                        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
                }))
    });
    for spec in &context.validation_commands {
        if spec.kind == "deploy" && !deployment_required {
            continue;
        }
        if spec.command.is_empty()
            || spec.command.contains('<')
            || spec.command.contains('>')
            || spec.command.contains('$')
        {
            continue;
        }
        if plan.iter().any(|step: &ValidationPlanStep| {
            step.command == spec.command && step.validation_kind == spec.kind
        }) {
            continue;
        }
        plan.push(ValidationPlanStep {
            id: format!("repository-rule-{}-{}", spec.source_line, plan.len() + 1),
            validation_kind: spec.kind.clone(),
            command: spec.command.clone(),
            reason: "required by repository instruction evidence".into(),
            source: format!("{}:{}", spec.source_path, spec.source_line),
            required: true,
        });
    }
    if deployment_required {
        if let Some(policy) = &context.deployment_policy {
            if let Some(command) = policy
                .command
                .as_ref()
                .filter(|command| !command.is_empty())
            {
                if !plan.iter().any(|step| step.command == *command) {
                    plan.push(ValidationPlanStep {
                        id: "installed-runtime-acceptance".into(),
                        validation_kind: "deploy".into(),
                        command: command.clone(),
                        reason: "typed repository deployment policy applies to the changed paths"
                            .into(),
                        source: policy.source_path.clone(),
                        required: true,
                    });
                }
            }
        }
    }
    let rust_crate_dirs = changed_paths
        .iter()
        .filter_map(|path| {
            let mut parts = path.split('/');
            (parts.next() == Some("crates"))
                .then(|| parts.next())
                .flatten()
        })
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let mut impact = if rust_crate_dirs.is_empty() {
        ValidationImpact::NonCode
    } else {
        ValidationImpact::PackageLocal
    };
    let manifest_changed = changed_paths
        .iter()
        .any(|path| path == "Cargo.toml" || path.ends_with("/Cargo.toml"));
    let mut dependency_checks = std::collections::BTreeSet::new();
    if context
        .manifests
        .iter()
        .any(|manifest| manifest.kind == "cargo")
    {
        let wrapper = Path::new(&context.root)
            .join("scripts/cargo-agent.sh")
            .is_file();
        let prefix = if wrapper {
            "bash scripts/cargo-agent.sh"
        } else {
            "cargo"
        };
        let packages = cargo_workspace_packages(Path::new(&context.root));
        for crate_dir in rust_crate_dirs {
            let crate_name = cargo_package_name(Path::new(&context.root), &crate_dir)
                .unwrap_or_else(|| crate_dir.clone());
            for (kind, suffix, reason) in [
                (
                    "check",
                    format!("check -p {crate_name}"),
                    format!("changed Rust package `{crate_name}` requires a target check"),
                ),
                (
                    "test",
                    format!("test -p {crate_name} --lib"),
                    format!("changed Rust package `{crate_name}` requires focused unit tests"),
                ),
            ] {
                let command = format!("{prefix} {suffix}");
                if !plan
                    .iter()
                    .any(|step| step.validation_kind == kind && step.command == command)
                {
                    plan.push(ValidationPlanStep {
                        id: format!("cargo-{kind}-{crate_name}"),
                        validation_kind: kind.into(),
                        command,
                        reason,
                        source: "manifest:Cargo.toml".into(),
                        required: true,
                    });
                }
            }
            let integration_dir = Path::new(&context.root)
                .join("crates")
                .join(&crate_dir)
                .join("tests");
            if integration_dir.is_dir()
                || changed_paths
                    .iter()
                    .any(|path| path.starts_with(&format!("crates/{crate_dir}/tests/")))
            {
                let command = format!("{prefix} test -p {crate_name} --tests");
                plan.push(ValidationPlanStep {
                    id: format!("cargo-integration-{crate_name}"),
                    validation_kind: "test".into(),
                    command,
                    reason: format!(
                        "changed Rust package `{crate_name}` has relevant integration-test targets"
                    ),
                    source: format!("manifest:crates/{crate_dir}/Cargo.toml"),
                    required: true,
                });
            }
            for (dependent, dependencies) in &packages {
                if dependent != &crate_name && dependencies.contains(&crate_name) {
                    dependency_checks.insert(dependent.clone());
                }
            }
        }
        for dependent in dependency_checks {
            impact = ValidationImpact::WorkspaceDependency;
            plan.push(ValidationPlanStep {
                id: format!("cargo-dependent-check-{dependent}"),
                validation_kind: "check".into(),
                command: format!("{prefix} check -p {dependent}"),
                reason: format!(
                    "direct workspace dependent `{dependent}` must compile against the changed package"
                ),
                source: "workspace dependency graph".into(),
                required: true,
            });
        }
    }
    plan.sort_by_key(|step| match step.validation_kind.as_str() {
        "format" => 0,
        "check" => 1,
        "test" => 2,
        "lint" => 3,
        "build" => 4,
        "deploy" => 5,
        _ => 6,
    });
    let mut omissions = Vec::new();
    if !plan.iter().any(|step| step.validation_kind == "format") {
        omissions.push(ValidationPlanOmission {
            validation_kind: "format".into(),
            reason: "no applicable repository format command was found".into(),
        });
    }
    if !plan
        .iter()
        .any(|step| step.validation_kind == "test" && step.command.contains("--tests"))
    {
        omissions.push(ValidationPlanOmission {
            validation_kind: "integration_test".into(),
            reason: "no changed package exposed a relevant integration-test target".into(),
        });
    }
    if !deployment_required {
        omissions.push(ValidationPlanOmission {
            validation_kind: "installed_runtime".into(),
            reason: "typed repository metadata does not require installed-runtime acceptance"
                .into(),
        });
    }
    if !plan.iter().any(|step| step.validation_kind == "build") {
        omissions.push(ValidationPlanOmission {
            validation_kind: "workspace_build".into(),
            reason: "targeted package checks cover the derived dependency impact; no workspace-wide build was selected".into(),
        });
    }
    let risk = if deployment_required {
        ValidationRisk::DeploymentCritical
    } else if manifest_changed || impact == ValidationImpact::WorkspaceDependency {
        ValidationRisk::High
    } else if impact == ValidationImpact::PackageLocal {
        ValidationRisk::Moderate
    } else {
        ValidationRisk::Low
    };
    ValidationProjection {
        steps: plan,
        omissions,
        impact,
        risk,
    }
}

fn cargo_package_name(root: &Path, crate_dir: &str) -> Option<String> {
    let content =
        std::fs::read_to_string(root.join("crates").join(crate_dir).join("Cargo.toml")).ok()?;
    content
        .parse::<toml::Value>()
        .ok()?
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

fn cargo_workspace_packages(root: &Path) -> Vec<(String, std::collections::BTreeSet<String>)> {
    let Ok(entries) = std::fs::read_dir(root.join("crates")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let manifest = std::fs::read_to_string(entry.path().join("Cargo.toml")).ok()?;
            let value = manifest.parse::<toml::Value>().ok()?;
            let package = value.get("package")?.get("name")?.as_str()?.to_string();
            let mut dependencies = std::collections::BTreeSet::new();
            for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(table) = value.get(table_name).and_then(toml::Value::as_table) {
                    dependencies.extend(table.keys().cloned());
                    dependencies.extend(table.values().filter_map(|dependency| {
                        dependency
                            .as_table()
                            .and_then(|table| table.get("package"))
                            .and_then(toml::Value::as_str)
                            .map(str::to_string)
                    }));
                }
            }
            Some((package, dependencies))
        })
        .collect()
}

#[derive(Clone)]
pub struct TransactionalFileWriteTool {
    registry: ChangeTransactionRegistry,
}

impl TransactionalFileWriteTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for TransactionalFileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }
    fn description(&self) -> &str {
        "Create or overwrite a file inside a host-owned version-bound change transaction."
    }
    fn input_schema(&self) -> serde_json::Value {
        let mut schema = FileWriteTool.input_schema();
        schema["properties"]["transaction_id"] = json!({"type":"string","description":"Host-minted transaction_id returned by repo_inspect"});
        schema["required"] = json!(["path", "content", "transaction_id"]);
        schema
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let id = match transaction_id(&input) {
            Ok(id) => id,
            Err(error) => return transaction_error(invalid_request(&error), ctx, start),
        };
        let before = match self
            .registry
            .verify_current(id, &ctx.session_id, ctx.agent, &ctx.working_dir)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => return transaction_error(error, ctx, start),
        };
        let mut result = FileWriteTool.execute(input, ctx).await;
        if result.is_error {
            return result;
        }
        let current = match workspace_version::capture(&ctx.working_dir) {
            Ok(version) => version,
            Err(error) => {
                return transaction_error(
                    WorkFailure {
                        class: WorkFailureClass::Environment,
                        summary: error.to_string(),
                        retryable: true,
                    },
                    ctx,
                    start,
                )
            }
        };
        let snapshot = match self.registry.record_apply(id, current).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return transaction_error(invalid_request(&error.to_string()), ctx, start)
            }
        };
        result.content = json!({
            "kind": "file_write_receipt",
            "transaction_id": id.0.to_string(),
            "baseline_workspace_version": before.baseline.digest,
            "resulting_workspace_version": snapshot.current.digest,
            "transaction_phase": snapshot.phase,
            "validation_plan": snapshot.validation_plan,
            "validation_omissions": snapshot.validation_omissions,
            "validation_impact": snapshot.validation_impact,
            "validation_risk": snapshot.validation_risk,
            "write_receipt": result.content,
        })
        .to_string();
        result
    }
}

#[derive(Clone)]
pub struct ChangeAcceptTool {
    registry: ChangeTransactionRegistry,
}

impl ChangeAcceptTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ChangeAcceptTool {
    fn name(&self) -> &str {
        "change_accept"
    }
    fn description(&self) -> &str {
        "Finalize a transaction only when its reviewed and validated workspace version is still current."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"transaction_id":{"type":"string"}},"required":["transaction_id"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let id = match transaction_id(&input) {
            Ok(id) => id,
            Err(error) => return transaction_error(invalid_request(&error), ctx, start),
        };
        match self
            .registry
            .accept(id, &ctx.session_id, ctx.agent, &ctx.working_dir)
            .await
        {
            Ok(snapshot) => ToolResult {
                content: json!({
                    "kind":"change_acceptance_receipt",
                    "transaction_id": id.0.to_string(),
                    "workspace_version": snapshot.accepted_workspace_version,
                    "transaction_phase": snapshot.phase,
                })
                .to_string(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(error) => transaction_error(error, ctx, start),
        }
    }
}

#[derive(Clone)]
pub struct ChangeRollbackTool {
    registry: ChangeTransactionRegistry,
}

impl ChangeRollbackTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ChangeRollbackTool {
    fn name(&self) -> &str {
        "change_rollback"
    }
    fn description(&self) -> &str {
        "Restore the exact host-captured transaction baseline after rechecking that no external writer changed the workspace."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"transaction_id":{"type":"string"}},"required":["transaction_id"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let id = match transaction_id(&input) {
            Ok(id) => id,
            Err(error) => return transaction_error(invalid_request(&error), ctx, start),
        };
        match self
            .registry
            .rollback(id, &ctx.session_id, ctx.agent, &ctx.working_dir)
            .await
        {
            Ok(snapshot) => ToolResult {
                content: json!({
                    "kind":"change_rollback_receipt",
                    "transaction_id":id.0.to_string(),
                    "workspace_version":snapshot.current.digest,
                    "baseline_workspace_version":snapshot.baseline.digest,
                    "transaction_phase":snapshot.phase,
                })
                .to_string(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(error) => transaction_error(error, ctx, start),
        }
    }
}

fn invalid_request(summary: &str) -> WorkFailure {
    WorkFailure {
        class: WorkFailureClass::InvalidToolRequest,
        summary: summary.into(),
        retryable: true,
    }
}

fn transaction_id(input: &serde_json::Value) -> Result<ChangeTransactionId, String> {
    let raw = input
        .get("transaction_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "transaction_id from repo_inspect is required".to_string())?;
    uuid::Uuid::parse_str(raw)
        .map(ChangeTransactionId)
        .map_err(|error| format!("invalid transaction_id: {error}"))
}

fn transaction_error(error: WorkFailure, ctx: &ToolContext, start: fabric::MonoTime) -> ToolResult {
    ToolResult {
        content: json!({
            "kind": "change_transaction_error",
            "recovery": error.recovery(),
            "failure": error
        })
        .to_string(),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

#[derive(Clone)]
pub struct TransactionalRepoInspectTool {
    registry: ChangeTransactionRegistry,
}

impl TransactionalRepoInspectTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for TransactionalRepoInspectTool {
    fn name(&self) -> &str {
        "repo_inspect"
    }
    fn description(&self) -> &str {
        "Inspect repository evidence and begin a host-owned version-bound change transaction."
    }
    fn input_schema(&self) -> serde_json::Value {
        RepoInspectTool.input_schema()
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let mut result = RepoInspectTool.execute(input, ctx).await;
        if result.is_error {
            return result;
        }
        let mut payload: serde_json::Value = match serde_json::from_str(&result.content) {
            Ok(payload) => payload,
            Err(error) => {
                return transaction_error(
                    invalid_request(&format!("invalid repo_inspect receipt: {error}")),
                    ctx,
                    ctx.clock.mono_now(),
                )
            }
        };
        let repository_context = match serde_json::from_value::<RepositoryContext>(payload.clone())
        {
            Ok(context) => context,
            Err(error) => {
                return transaction_error(
                    invalid_request(&format!(
                        "repo_inspect receipt is not a RepositoryContext: {error}"
                    )),
                    ctx,
                    ctx.clock.mono_now(),
                )
            }
        };
        match self
            .registry
            .begin_with_context(&ctx.session_id, ctx.agent, repository_context)
            .await
        {
            Ok(snapshot) => {
                payload["change_transaction"] = serde_json::to_value(snapshot).unwrap_or_default();
                result.content = serde_json::to_string_pretty(&payload).unwrap_or_default();
                result
            }
            Err(error) => transaction_error(
                WorkFailure {
                    class: WorkFailureClass::Environment,
                    summary: error.to_string(),
                    retryable: true,
                },
                ctx,
                ctx.clock.mono_now(),
            ),
        }
    }
}

#[derive(Clone)]
pub struct TransactionalApplyPatchTool {
    registry: ChangeTransactionRegistry,
}

impl TransactionalApplyPatchTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for TransactionalApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }
    fn description(&self) -> &str {
        "Apply a scoped patch inside the transaction_id returned by repo_inspect; rejects stale or foreign workspace versions."
    }
    fn input_schema(&self) -> serde_json::Value {
        let mut schema = ApplyPatchTool.input_schema();
        schema["properties"]["transaction_id"] = json!({"type":"string", "description":"Host-minted transaction_id returned by repo_inspect"});
        schema["required"] = json!(["transaction_id"]);
        schema
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let id = match transaction_id(&input) {
            Ok(id) => id,
            Err(error) => return transaction_error(invalid_request(&error), ctx, start),
        };
        let root = input
            .get("base_dir")
            .and_then(|value| value.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let root = if root.is_absolute() {
            root
        } else {
            ctx.working_dir.join(root)
        };
        let before = match self
            .registry
            .verify_current(id, &ctx.session_id, ctx.agent, &root)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => return transaction_error(error, ctx, start),
        };
        let mut result = ApplyPatchTool.execute(input, ctx).await;
        if result.is_error
            || serde_json::from_str::<serde_json::Value>(&result.content)
                .ok()
                .and_then(|value| {
                    value
                        .get("terminal_status")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                != Some("succeeded")
        {
            return result;
        }
        let current = match workspace_version::capture(&root) {
            Ok(current) => current,
            Err(error) => {
                return transaction_error(
                    WorkFailure {
                        class: WorkFailureClass::Environment,
                        summary: error.to_string(),
                        retryable: true,
                    },
                    ctx,
                    start,
                )
            }
        };
        let snapshot = match self.registry.record_apply(id, current).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return transaction_error(invalid_request(&error.to_string()), ctx, start)
            }
        };
        if let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&result.content) {
            payload["transaction_id"] = json!(id.0.to_string());
            payload["baseline_workspace_version"] = json!(before.baseline.digest);
            payload["resulting_workspace_version"] = json!(snapshot.current.digest);
            payload["transaction_phase"] = json!(snapshot.phase);
            payload["validation_plan"] = json!(snapshot.validation_plan);
            payload["validation_omissions"] = json!(snapshot.validation_omissions);
            payload["validation_impact"] = json!(snapshot.validation_impact);
            payload["validation_risk"] = json!(snapshot.validation_risk);
            result.content = serde_json::to_string_pretty(&payload).unwrap_or_default();
        }
        result
    }
}

#[derive(Clone)]
pub struct TransactionalGitDiffTool {
    registry: ChangeTransactionRegistry,
}

impl TransactionalGitDiffTool {
    pub fn new(registry: ChangeTransactionRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for TransactionalGitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }
    fn description(&self) -> &str {
        "Review the complete baseline-to-current diff for a version-bound Git or bounded-tree change transaction and preserve it as an artifact."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"transaction_id":{"type":"string"},"path":{"type":"string"}},"required":["transaction_id"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let id = match transaction_id(&input) {
            Ok(id) => id,
            Err(error) => return transaction_error(invalid_request(&error), ctx, start),
        };
        let root = input
            .get("path")
            .and_then(|value| value.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let root = if root.is_absolute() {
            root
        } else {
            ctx.working_dir.join(root)
        };
        let snapshot = match self
            .registry
            .verify_current(id, &ctx.session_id, ctx.agent, &root)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => return transaction_error(error, ctx, start),
        };
        if !matches!(
            snapshot.phase,
            ChangeTransactionPhase::Applied | ChangeTransactionPhase::Repair
        ) {
            return transaction_error(
                invalid_request("git_diff requires an applied transaction"),
                ctx,
                start,
            );
        }
        let mut output = if snapshot.current.basis
            == fabric::change_transaction::WorkspaceVersionBasis::BoundedTree
        {
            match self.registry.bounded_diff(id, &snapshot).await {
                Ok(output) => output,
                Err(error) => {
                    return transaction_error(
                        WorkFailure {
                            class: WorkFailureClass::Environment,
                            summary: error.to_string(),
                            retryable: true,
                        },
                        ctx,
                        start,
                    )
                }
            }
        } else {
            Vec::new()
        };
        if snapshot.current.basis == fabric::change_transaction::WorkspaceVersionBasis::GitWorktree
        {
            let command_output = std::process::Command::new("git")
                .args(["diff", "--binary", "HEAD", "--", "."])
                .current_dir(&root)
                .output();
            output = match command_output {
                Ok(output) if output.status.success() => output.stdout,
                Ok(output) => {
                    return transaction_error(
                        WorkFailure {
                            class: WorkFailureClass::Environment,
                            summary: String::from_utf8_lossy(&output.stderr).into_owned(),
                            retryable: true,
                        },
                        ctx,
                        start,
                    )
                }
                Err(error) => {
                    return transaction_error(
                        WorkFailure {
                            class: WorkFailureClass::Environment,
                            summary: error.to_string(),
                            retryable: true,
                        },
                        ctx,
                        start,
                    )
                }
            };
            let untracked = std::process::Command::new("git")
                .args([
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--",
                    ".",
                ])
                .current_dir(&root)
                .output();
            let untracked = match untracked {
                Ok(result) if result.status.success() => result.stdout,
                Ok(result) => {
                    return transaction_error(
                        WorkFailure {
                            class: WorkFailureClass::Environment,
                            summary: String::from_utf8_lossy(&result.stderr).into_owned(),
                            retryable: true,
                        },
                        ctx,
                        start,
                    )
                }
                Err(error) => {
                    return transaction_error(
                        WorkFailure {
                            class: WorkFailureClass::Environment,
                            summary: error.to_string(),
                            retryable: true,
                        },
                        ctx,
                        start,
                    )
                }
            };
            for relative in untracked
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                let relative = String::from_utf8_lossy(relative);
                let untracked_diff = std::process::Command::new("git")
                    .args([
                        "diff",
                        "--no-index",
                        "--binary",
                        "--",
                        "/dev/null",
                        relative.as_ref(),
                    ])
                    .current_dir(&root)
                    .output();
                match untracked_diff {
                    Ok(result) if matches!(result.status.code(), Some(0 | 1)) => {
                        output.extend_from_slice(&result.stdout);
                    }
                    Ok(result) => {
                        return transaction_error(
                            WorkFailure {
                                class: WorkFailureClass::Environment,
                                summary: String::from_utf8_lossy(&result.stderr).into_owned(),
                                retryable: true,
                            },
                            ctx,
                            start,
                        )
                    }
                    Err(error) => {
                        return transaction_error(
                            WorkFailure {
                                class: WorkFailureClass::Environment,
                                summary: error.to_string(),
                                retryable: true,
                            },
                            ctx,
                            start,
                        )
                    }
                }
            }
        }
        let store = ArtifactStore::new(
            super::output::OutputConfig::default()
                .overflow_dir
                .join("artifacts"),
        );
        let artifact = match store.store(&output, "text/x-diff") {
            Ok(artifact) => artifact,
            Err(error) => {
                return transaction_error(
                    WorkFailure {
                        class: WorkFailureClass::Environment,
                        summary: error.to_string(),
                        retryable: true,
                    },
                    ctx,
                    start,
                )
            }
        };
        let changed_ranges =
            changed_ranges_from_diff(&output, &updated_paths_for_ranges(&snapshot, &root));
        let updated = match self
            .registry
            .record_diff_review(id, artifact.uri(), changed_ranges)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return transaction_error(invalid_request(&error.to_string()), ctx, start)
            }
        };
        ToolResult {
            content: json!({
                "kind":"change_diff_receipt",
                "transaction_id": id.0.to_string(),
                "workspace_version": updated.current.digest,
                "diff_sha256": artifact.sha256,
                "diff_artifact_ref": artifact.uri(),
                "bytes": output.len(),
                "empty": output.is_empty(),
                "transaction_phase": updated.phase,
                "changed_ranges": updated.changed_ranges,
                "validation_plan": updated.validation_plan,
                "validation_omissions": updated.validation_omissions,
                "validation_impact": updated.validation_impact,
                "validation_risk": updated.validation_risk,
                "preview": String::from_utf8_lossy(&output[..output.len().min(24 * 1024)]),
                "truncated": output.len() > 24 * 1024,
            })
            .to_string(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: output.len() > 24 * 1024,
                patch_delta: None,
            },
        }
    }
}

fn updated_paths_for_ranges(snapshot: &ChangeTransactionSnapshot, _root: &Path) -> Vec<String> {
    snapshot.changed_paths.clone()
}

fn changed_ranges_from_diff(output: &[u8], fallback_paths: &[String]) -> Vec<ChangedRange> {
    let text = String::from_utf8_lossy(output);
    let mut current_path = None::<String>;
    let mut previous_path = None::<String>;
    let mut ranges = Vec::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("--- a/") {
            previous_path = Some(path.to_string());
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_path = Some(path.to_string());
            continue;
        }
        if line == "+++ /dev/null" {
            current_path = previous_path.clone();
            continue;
        }
        let Some(header) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some(added) = header.split_whitespace().find(|part| part.starts_with('+')) else {
            continue;
        };
        let coordinates = added.trim_start_matches('+');
        let mut parts = coordinates.split(',');
        let Some(start) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let count = parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        if let Some(path) = current_path.clone() {
            ranges.push(ChangedRange {
                path,
                start_line: start,
                end_line: start.saturating_add(count.saturating_sub(1)),
            });
        }
    }
    if ranges.is_empty() {
        ranges.extend(fallback_paths.iter().cloned().map(|path| ChangedRange {
            path,
            start_line: 1,
            end_line: 1,
        }));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::repository::{
        DeploymentPolicy, ManifestRef, RepositoryFileEvidence, ValidationSpec, VcsSnapshot,
    };

    fn init_repo() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "test"],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(temp.path().join("file.txt"), "before\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["commit", "-q", "-m", "baseline"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        temp
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: root.to_path_buf(),
            session_id: "transaction-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    async fn begin(
        registry: &ChangeTransactionRegistry,
        context: &ToolContext,
    ) -> ChangeTransactionSnapshot {
        registry
            .begin(&context.session_id, &context.working_dir)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn apply_and_diff_bind_to_one_workspace_version() {
        let repo = init_repo();
        let registry = ChangeTransactionRegistry::default();
        let context = context(repo.path());
        let transaction = begin(&registry, &context).await;
        let patch =
            "*** Begin Patch\nUpdate File: file.txt\n>>>\n@@ -1 +1 @@\n-before\n+after\n>>>\n*** End Patch";

        let applied = TransactionalApplyPatchTool::new(registry.clone())
            .execute(
                json!({"transaction_id": transaction.transaction_id.0, "patch": patch}),
                &context,
            )
            .await;
        assert!(!applied.is_error, "{}", applied.content);
        let applied_receipt: serde_json::Value = serde_json::from_str(&applied.content).unwrap();
        assert_ne!(
            applied_receipt["baseline_workspace_version"],
            applied_receipt["resulting_workspace_version"]
        );

        let reviewed = TransactionalGitDiffTool::new(registry.clone())
            .execute(
                json!({"transaction_id": transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!reviewed.is_error, "{}", reviewed.content);
        let reviewed_receipt: serde_json::Value = serde_json::from_str(&reviewed.content).unwrap();
        assert_eq!(
            reviewed_receipt["workspace_version"],
            applied_receipt["resulting_workspace_version"]
        );
        assert!(reviewed_receipt["diff_artifact_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("artifact://sha256/")));
        assert_eq!(reviewed_receipt["changed_ranges"][0]["path"], "file.txt");
        assert_eq!(reviewed_receipt["changed_ranges"][0]["start_line"], 1);
        assert_eq!(
            registry
                .snapshot(transaction.transaction_id)
                .await
                .unwrap()
                .phase,
            ChangeTransactionPhase::DiffReviewed
        );
    }

    #[tokio::test]
    async fn concurrent_modification_fails_before_patch_application() {
        let repo = init_repo();
        let registry = ChangeTransactionRegistry::default();
        let context = context(repo.path());
        let transaction = begin(&registry, &context).await;
        std::fs::write(repo.path().join("file.txt"), "concurrent\n").unwrap();
        let patch =
            "*** Begin Patch\nUpdate File: file.txt\n>>>\n@@ -1 +1 @@\n-before\n+agent\n>>>\n*** End Patch";

        let result = TransactionalApplyPatchTool::new(registry.clone())
            .execute(
                json!({"transaction_id": transaction.transaction_id.0, "patch": patch}),
                &context,
            )
            .await;

        assert!(result.is_error);
        let receipt: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(receipt["failure"]["class"], "concurrent_modification");
        assert_eq!(
            registry
                .snapshot(transaction.transaction_id)
                .await
                .unwrap()
                .phase,
            ChangeTransactionPhase::Conflicted
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("file.txt")).unwrap(),
            "concurrent\n"
        );
    }

    #[tokio::test]
    async fn sibling_agent_in_same_session_cannot_use_transaction() {
        let repo = init_repo();
        let registry = ChangeTransactionRegistry::default();
        let owner = fabric::AgentToolContext {
            caller_root_agent_id: fabric::AgentId::new(),
            parent_agent_id: fabric::AgentId::new(),
            parent_process_id: fabric::ProcessId::new(),
        };
        let sibling = fabric::AgentToolContext {
            caller_root_agent_id: owner.caller_root_agent_id,
            parent_agent_id: fabric::AgentId::new(),
            parent_process_id: fabric::ProcessId::new(),
        };
        let transaction = registry
            .begin_for_agent("shared-session", Some(owner), repo.path())
            .await
            .unwrap();
        let failure = registry
            .verify_current(
                transaction.transaction_id,
                "shared-session",
                Some(sibling),
                repo.path(),
            )
            .await
            .unwrap_err();
        assert_eq!(failure.class, WorkFailureClass::Permission);
    }

    #[tokio::test]
    async fn file_write_diff_validation_and_acceptance_share_exact_version() {
        let repo = init_repo();
        let registry = ChangeTransactionRegistry::default();
        let context = context(repo.path());
        let transaction = begin(&registry, &context).await;

        let written = TransactionalFileWriteTool::new(registry.clone())
            .execute(
                json!({
                    "transaction_id": transaction.transaction_id.0,
                    "path": "new.txt",
                    "content": "new evidence\n"
                }),
                &context,
            )
            .await;
        assert!(!written.is_error, "{}", written.content);
        let write_receipt: serde_json::Value = serde_json::from_str(&written.content).unwrap();
        let version = write_receipt["resulting_workspace_version"]
            .as_str()
            .unwrap()
            .to_string();

        let reviewed = TransactionalGitDiffTool::new(registry.clone())
            .execute(
                json!({"transaction_id": transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!reviewed.is_error, "{}", reviewed.content);
        let reviewed: serde_json::Value = serde_json::from_str(&reviewed.content).unwrap();
        assert!(!reviewed["empty"].as_bool().unwrap());
        assert_eq!(reviewed["workspace_version"], version);

        registry
            .set_validation_plan(
                transaction.transaction_id,
                vec![ValidationPlanStep {
                    id: "focused".into(),
                    validation_kind: "test".into(),
                    command: "true".into(),
                    reason: "fixture".into(),
                    source: "test".into(),
                    required: true,
                }],
            )
            .await;
        let observed = workspace_version::capture(repo.path()).unwrap();
        registry
            .reserve_command(
                transaction.transaction_id,
                &context.session_id,
                context.agent,
                repo.path(),
                "validation-test".into(),
                "validation".into(),
            )
            .await
            .unwrap();
        let validated = registry
            .record_validation(
                transaction.transaction_id,
                "test".into(),
                "true".into(),
                version.clone(),
                observed,
                "succeeded".into(),
                Some("artifact://sha256/validation".into()),
                "validation-test",
            )
            .await
            .unwrap();
        assert_eq!(validated.phase, ChangeTransactionPhase::Validated);

        let accepted = ChangeAcceptTool::new(registry)
            .execute(
                json!({"transaction_id": transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!accepted.is_error, "{}", accepted.content);
        let accepted: serde_json::Value = serde_json::from_str(&accepted.content).unwrap();
        assert_eq!(accepted["workspace_version"], version);
        assert_eq!(accepted["transaction_phase"], "accepted");
    }

    #[tokio::test]
    async fn acceptance_rechecks_workspace_after_validation() {
        let repo = init_repo();
        let registry = ChangeTransactionRegistry::default();
        let context = context(repo.path());
        let transaction = begin(&registry, &context).await;
        let current = workspace_version::capture(repo.path()).unwrap();
        registry
            .record_apply(transaction.transaction_id, current.clone())
            .await
            .unwrap();
        registry
            .record_diff_review(
                transaction.transaction_id,
                "artifact://sha256/diff".into(),
                Vec::new(),
            )
            .await
            .unwrap();
        registry
            .set_validation_plan(
                transaction.transaction_id,
                vec![ValidationPlanStep {
                    id: "focused".into(),
                    validation_kind: "test".into(),
                    command: "true".into(),
                    reason: "fixture".into(),
                    source: "test".into(),
                    required: true,
                }],
            )
            .await;
        registry
            .reserve_command(
                transaction.transaction_id,
                &context.session_id,
                context.agent,
                repo.path(),
                "validation-test".into(),
                "validation".into(),
            )
            .await
            .unwrap();
        registry
            .record_validation(
                transaction.transaction_id,
                "test".into(),
                "true".into(),
                current.digest.clone(),
                current,
                "succeeded".into(),
                None,
                "validation-test",
            )
            .await
            .unwrap();
        std::fs::write(repo.path().join("outside.txt"), "changed after tests").unwrap();

        let result = ChangeAcceptTool::new(registry.clone())
            .execute(
                json!({"transaction_id": transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(result.is_error);
        assert_eq!(
            registry
                .snapshot(transaction.transaction_id)
                .await
                .unwrap()
                .phase,
            ChangeTransactionPhase::Conflicted
        );
    }

    #[test]
    fn validation_planner_orders_repo_rules_adds_focused_targets_and_gates_deploy() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("scripts")).unwrap();
        std::fs::write(temp.path().join("scripts/cargo-agent.sh"), "#!/bin/sh\n").unwrap();
        let file = RepositoryFileEvidence {
            path: "Cargo.toml".into(),
            sha256: "manifest".into(),
            size_bytes: 0,
            artifact_ref: "artifact://sha256/manifest".into(),
            preview: String::new(),
            preview_truncated: false,
        };
        let context = RepositoryContext {
            root: temp.path().display().to_string(),
            version: "context".into(),
            instructions: Vec::new(),
            manifests: vec![ManifestRef {
                file,
                kind: "cargo".into(),
            }],
            entry_files: Vec::new(),
            missing_candidates: Vec::new(),
            vcs_state: VcsSnapshot::default(),
            validation_commands: vec![
                ValidationSpec {
                    kind: "format".into(),
                    command: "bash scripts/cargo-agent.sh fmt --all -- --check".into(),
                    source_path: "AGENTS.md".into(),
                    source_line: 10,
                },
                ValidationSpec {
                    kind: "deploy".into(),
                    command: "sudo bash scripts/deploy.sh".into(),
                    source_path: "AGENTS.md".into(),
                    source_line: 20,
                },
            ],
            protected_paths: Vec::new(),
            deployment_policy: None,
        };

        let projection = derive_validation_plan(&context, &["crates/corpus/src/lib.rs".into()]);
        let plan = projection.steps;
        assert_eq!(
            plan.iter()
                .map(|step| step.validation_kind.as_str())
                .collect::<Vec<_>>(),
            vec!["format", "check", "test"]
        );
        assert_eq!(
            plan[1].command,
            "bash scripts/cargo-agent.sh check -p corpus"
        );
        assert!(!plan.iter().any(|step| step.validation_kind == "deploy"));
        assert_eq!(projection.impact, ValidationImpact::PackageLocal);
        assert_eq!(projection.risk, ValidationRisk::Moderate);
        assert!(projection
            .omissions
            .iter()
            .any(|omission| omission.validation_kind == "installed_runtime"));

        let mut installed = context;
        installed.deployment_policy = Some(DeploymentPolicy {
            source_path: "typed-policy".into(),
            requires_installed_runtime: true,
            command: Some("sudo bash scripts/deploy.sh".into()),
            affected_path_prefixes: Vec::new(),
        });
        let installed_projection =
            derive_validation_plan(&installed, &["crates/corpus/src/lib.rs".into()]);
        assert!(installed_projection
            .steps
            .iter()
            .any(|step| step.validation_kind == "deploy"));
        assert_eq!(
            installed_projection.risk,
            ValidationRisk::DeploymentCritical
        );
    }

    #[test]
    fn validation_planner_expands_only_direct_workspace_dependency_impact() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("scripts")).unwrap();
        std::fs::write(temp.path().join("scripts/cargo-agent.sh"), "#!/bin/sh\n").unwrap();
        for (name, manifest) in [
            (
                "core_pkg",
                "[package]\nname='core-package'\nversion='0.1.0'\n",
            ),
            (
                "dependent",
                "[package]\nname='dependent'\nversion='0.1.0'\n[dependencies]\ncore_alias={package='core-package',path='../core_pkg'}\n",
            ),
            (
                "unrelated",
                "[package]\nname='unrelated'\nversion='0.1.0'\n",
            ),
        ] {
            let directory = temp.path().join("crates").join(name);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Cargo.toml"), manifest).unwrap();
        }
        let context = RepositoryContext {
            root: temp.path().display().to_string(),
            version: "context".into(),
            instructions: Vec::new(),
            manifests: vec![ManifestRef {
                file: RepositoryFileEvidence {
                    path: "Cargo.toml".into(),
                    sha256: "manifest".into(),
                    size_bytes: 0,
                    artifact_ref: "artifact://sha256/manifest".into(),
                    preview: String::new(),
                    preview_truncated: false,
                },
                kind: "cargo".into(),
            }],
            entry_files: Vec::new(),
            missing_candidates: Vec::new(),
            vcs_state: VcsSnapshot::default(),
            validation_commands: Vec::new(),
            protected_paths: Vec::new(),
            deployment_policy: None,
        };
        let projection = derive_validation_plan(&context, &["crates/core_pkg/src/lib.rs".into()]);
        assert_eq!(projection.impact, ValidationImpact::WorkspaceDependency);
        assert_eq!(projection.risk, ValidationRisk::High);
        let commands = projection
            .steps
            .iter()
            .map(|step| step.command.as_str())
            .collect::<Vec<_>>();
        assert!(commands.contains(&"bash scripts/cargo-agent.sh check -p core-package"));
        assert!(commands.contains(&"bash scripts/cargo-agent.sh test -p core-package --lib"));
        assert!(commands.contains(&"bash scripts/cargo-agent.sh check -p dependent"));
        assert!(!commands.iter().any(|command| command.contains("unrelated")));
    }

    #[tokio::test]
    async fn rollback_restores_preexisting_dirty_baseline_and_removes_new_files() {
        let repo = init_repo();
        std::fs::write(repo.path().join("file.txt"), "dirty baseline\n").unwrap();
        std::fs::write(repo.path().join("preexisting.txt"), "keep me\n").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let context = context(repo.path());
        let transaction = begin(&registry, &context).await;

        for (path, content) in [
            ("file.txt", "transaction edit\n"),
            ("created.txt", "remove me\n"),
        ] {
            let result = TransactionalFileWriteTool::new(registry.clone())
                .execute(
                    json!({
                        "transaction_id":transaction.transaction_id.0,
                        "path":path,
                        "content":content
                    }),
                    &context,
                )
                .await;
            assert!(!result.is_error, "{}", result.content);
        }

        let rolled_back = ChangeRollbackTool::new(registry.clone())
            .execute(
                json!({"transaction_id":transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!rolled_back.is_error, "{}", rolled_back.content);
        assert_eq!(
            std::fs::read_to_string(repo.path().join("file.txt")).unwrap(),
            "dirty baseline\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("preexisting.txt")).unwrap(),
            "keep me\n"
        );
        assert!(!repo.path().join("created.txt").exists());
        let snapshot = registry.snapshot(transaction.transaction_id).await.unwrap();
        assert_eq!(snapshot.phase, ChangeTransactionPhase::RolledBack);
        assert_eq!(snapshot.current.digest, transaction.baseline.digest);
    }

    #[tokio::test]
    async fn bounded_tree_change_produces_content_backed_diff_and_rolls_back() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("plain.txt"), "before\n").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let context = context(temp.path());
        let transaction = begin(&registry, &context).await;
        let written = TransactionalFileWriteTool::new(registry.clone())
            .execute(
                json!({
                    "transaction_id":transaction.transaction_id.0,
                    "path":"plain.txt",
                    "content":"after\n"
                }),
                &context,
            )
            .await;
        assert!(!written.is_error, "{}", written.content);
        let reviewed = TransactionalGitDiffTool::new(registry.clone())
            .execute(
                json!({"transaction_id":transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!reviewed.is_error, "{}", reviewed.content);
        let receipt: serde_json::Value = serde_json::from_str(&reviewed.content).unwrap();
        assert!(receipt["preview"]
            .as_str()
            .unwrap()
            .contains("before-content"));
        assert!(receipt["preview"]
            .as_str()
            .unwrap()
            .contains("after-content"));
        let rolled_back = ChangeRollbackTool::new(registry)
            .execute(
                json!({"transaction_id":transaction.transaction_id.0}),
                &context,
            )
            .await;
        assert!(!rolled_back.is_error, "{}", rolled_back.content);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("plain.txt")).unwrap(),
            "before\n"
        );
    }
}
