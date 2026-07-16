//! Authenticated local principal and workspace authority contracts.

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};
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

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
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
        })
    }

    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    pub fn writable_roots(&self) -> &[PathBuf] {
        &self.writable_roots
    }
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
        }
    }
}
