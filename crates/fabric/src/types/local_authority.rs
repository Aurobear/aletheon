//! Authenticated local principal and workspace authority contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::{admission::PrincipalId, session::TurnId};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ThreadId(pub String);

impl From<&str> for ThreadId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalOsPrincipal {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionProfileId(pub String);

impl PermissionProfileId {
    pub fn workspace_write() -> Self {
        Self("workspace-write".into())
    }

    pub fn danger_full_access() -> Self {
        Self("danger-full-access".into())
    }

    pub fn permits_filesystem_root(&self) -> bool {
        self.0 == "danger-full-access"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Never,
    OnRequest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePolicy {
    cwd: PathBuf,
    writable_roots: Vec<PathBuf>,
    #[serde(default)]
    protected_paths: ProtectedPathPolicy,
}

impl WorkspacePolicy {
    pub fn from_resolved_roots(cwd: PathBuf, extra: Vec<PathBuf>) -> Result<Self, String> {
        if !cwd.is_absolute() {
            return Err(format!("cwd is not absolute: {}", cwd.display()));
        }

        let mut seen = HashSet::new();
        let mut writable_roots = Vec::new();
        for root in std::iter::once(cwd.clone()).chain(extra) {
            if !root.is_absolute() {
                return Err(format!("root is not absolute: {}", root.display()));
            }
            if seen.insert(root.clone()) {
                writable_roots.push(root);
            }
        }

        Ok(Self {
            cwd,
            writable_roots,
            protected_paths: ProtectedPathPolicy::default(),
        })
    }

    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    pub fn writable_roots(&self) -> &[PathBuf] {
        &self.writable_roots
    }

    /// Reduce this policy to an explicit subset of its existing write authority.
    ///
    /// The working directory is intentionally independent from the writable
    /// roots so a sandboxed process may start at the repository root while only
    /// a reviewed file or subdirectory is writable.
    pub fn narrow_writable_roots(mut self, roots: Vec<PathBuf>) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("narrowed workspace roots must not be empty".into());
        }

        let mut seen = HashSet::new();
        let mut narrowed = Vec::with_capacity(roots.len());
        for root in roots {
            if !root.is_absolute() {
                return Err(format!(
                    "narrowed workspace root is not absolute: {}",
                    root.display()
                ));
            }
            if !self
                .writable_roots
                .iter()
                .any(|authority| root.starts_with(authority))
            {
                return Err(format!(
                    "narrowed workspace root exceeds existing authority: {}",
                    root.display()
                ));
            }
            if seen.insert(root.clone()) {
                narrowed.push(root);
            }
        }
        self.writable_roots = narrowed;
        Ok(self)
    }

    pub fn with_protected_paths(mut self, protected_paths: ProtectedPathPolicy) -> Self {
        self.protected_paths = protected_paths;
        self
    }

    pub fn protected_paths(&self) -> &ProtectedPathPolicy {
        &self.protected_paths
    }
}

/// Paths that remain read-only even inside a writable workspace root.
///
/// Metadata names are intentionally limited to the design contract. Other
/// credential locations must be supplied explicitly as absolute paths.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtectedPathPolicy {
    credential_paths: Vec<PathBuf>,
}

impl ProtectedPathPolicy {
    pub fn new(credential_paths: Vec<PathBuf>) -> Result<Self, String> {
        let mut seen = HashSet::new();
        let mut materialized = Vec::new();
        for path in credential_paths {
            if !path.is_absolute() {
                return Err(format!(
                    "protected credential path is not absolute: {}",
                    path.display()
                ));
            }
            let path = materialize_protected_path(&path)?;
            if seen.insert(path.clone()) {
                materialized.push(path);
            }
        }
        Ok(Self {
            credential_paths: materialized,
        })
    }

    pub fn credential_paths(&self) -> &[PathBuf] {
        &self.credential_paths
    }
}

fn materialize_protected_path(path: &Path) -> Result<PathBuf, String> {
    let mut missing = Vec::new();
    let mut ancestor = path;
    while !ancestor.exists() {
        let name = ancestor.file_name().ok_or_else(|| {
            format!(
                "protected credential path has no existing ancestor: {}",
                path.display()
            )
        })?;
        missing.push(name.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| {
            format!(
                "protected credential path has no existing ancestor: {}",
                path.display()
            )
        })?;
    }
    let mut resolved = std::fs::canonicalize(ancestor).map_err(|error| {
        format!(
            "cannot resolve protected credential path '{}': {error}",
            path.display()
        )
    })?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

/// Raw workspace options supplied by one client invocation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceSelection {
    cwd: Option<PathBuf>,
    add_dirs: Vec<PathBuf>,
}

impl WorkspaceSelection {
    pub fn new(cwd: Option<PathBuf>, add_dirs: Vec<PathBuf>) -> Self {
        Self { cwd, add_dirs }
    }

    pub fn resolve(self, process_cwd: &Path) -> Result<WorkspacePolicy, WorkspaceResolveError> {
        self.resolve_with_profile(process_cwd, &PermissionProfileId::workspace_write())
    }

    pub fn resolve_with_profile(
        self,
        process_cwd: &Path,
        profile: &PermissionProfileId,
    ) -> Result<WorkspacePolicy, WorkspaceResolveError> {
        let explicitly_selected = self.cwd.is_some();
        let requested = self.cwd.unwrap_or_else(|| process_cwd.to_path_buf());
        let cwd_input = if requested.is_absolute() {
            requested
        } else {
            process_cwd.join(requested)
        };
        let cwd = canonical_directory(&cwd_input)?;
        if cwd == Path::new("/") && !explicitly_selected {
            return Err(WorkspaceResolveError::ImplicitFilesystemRoot);
        }
        ensure_filesystem_root_allowed(&cwd, profile)?;

        let mut roots = Vec::with_capacity(self.add_dirs.len());
        for raw in self.add_dirs {
            let input = if raw.is_absolute() {
                raw
            } else {
                cwd.join(raw)
            };
            let root = canonical_directory(&input)?;
            ensure_filesystem_root_allowed(&root, profile)?;
            roots.push(root);
        }

        WorkspacePolicy::from_resolved_roots(cwd, roots).map_err(WorkspaceResolveError::Policy)
    }
}

fn canonical_directory(input: &Path) -> Result<PathBuf, WorkspaceResolveError> {
    let canonical =
        std::fs::canonicalize(input).map_err(|source| WorkspaceResolveError::Filesystem {
            path: input.to_path_buf(),
            source,
        })?;
    if !canonical.is_dir() {
        return Err(WorkspaceResolveError::Filesystem {
            path: input.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotADirectory, "path is not a directory"),
        });
    }
    Ok(canonical)
}

fn ensure_filesystem_root_allowed(
    root: &Path,
    profile: &PermissionProfileId,
) -> Result<(), WorkspaceResolveError> {
    if root == Path::new("/") && !profile.permits_filesystem_root() {
        return Err(WorkspaceResolveError::FilesystemRootDenied {
            profile: profile.clone(),
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceResolveError {
    #[error("workspace path '{}': {source}", path.display())]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the filesystem root cannot be inferred as a workspace")]
    ImplicitFilesystemRoot,
    #[error("permission profile '{}' does not permit the filesystem root", profile.0)]
    FilesystemRootDenied { profile: PermissionProfileId },
    #[error("invalid resolved workspace policy: {0}")]
    Policy(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrincipalContext {
    pub principal_id: PrincipalId,
    pub os_principal: LocalOsPrincipal,
    pub connection_id: ConnectionId,
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub workspace: WorkspacePolicy,
    pub permission_profile: PermissionProfileId,
    pub approval_policy: ApprovalPolicy,
    /// Host-minted G1 decision for repository command hooks.
    #[serde(default)]
    pub repo_hooks_trusted: bool,
}

impl PrincipalContext {
    pub fn new(
        principal_id: PrincipalId,
        os_principal: LocalOsPrincipal,
        connection_id: ConnectionId,
        thread_id: ThreadId,
        workspace: WorkspacePolicy,
        permission_profile: PermissionProfileId,
        approval_policy: ApprovalPolicy,
    ) -> Self {
        Self {
            principal_id,
            os_principal,
            connection_id,
            thread_id,
            turn_id: None,
            workspace,
            permission_profile,
            approval_policy,
            repo_hooks_trusted: false,
        }
    }
}
