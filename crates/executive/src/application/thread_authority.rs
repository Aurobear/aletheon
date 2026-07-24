//! Immutable per-principal thread settings.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use fabric::{ApprovalPolicy, PermissionProfileId, PrincipalId, ThreadId, WorkspacePolicy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ThreadAuthorityKey {
    principal: PrincipalId,
    thread: ThreadId,
}

impl ThreadAuthorityKey {
    pub fn new(principal: PrincipalId, thread: ThreadId) -> Self {
        Self { principal, thread }
    }

    fn file_name(&self) -> String {
        let name = format!("{}\0{}", self.principal.0, self.thread.0);
        format!(
            "{}.json",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadSettings {
    pub workspace: WorkspacePolicy,
    pub permission_profile: PermissionProfileId,
    pub approval_policy: ApprovalPolicy,
    pub model_policy: Option<String>,
}

impl ThreadSettings {
    pub fn from_context(context: &fabric::PrincipalContext, model_policy: Option<String>) -> Self {
        Self {
            workspace: context.workspace.clone(),
            permission_profile: context.permission_profile.clone(),
            approval_policy: context.approval_policy,
            model_policy,
        }
    }
}

pub struct ThreadAuthorityStore {
    root: Option<PathBuf>,
    records: Mutex<HashMap<ThreadAuthorityKey, ThreadSettings>>,
}

impl ThreadAuthorityStore {
    pub fn in_memory() -> Self {
        Self {
            root: None,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub fn persistent(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
            records: Mutex::new(HashMap::new()),
        }
    }

    pub fn bind_or_verify(
        &self,
        key: &ThreadAuthorityKey,
        settings: &ThreadSettings,
    ) -> Result<(), ThreadAuthorityError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| ThreadAuthorityError::Poisoned)?;
        if let Some(existing) = records.get(key) {
            return compare(key, existing, settings);
        }

        if let Some(root) = &self.root {
            fs::create_dir_all(root).map_err(|source| ThreadAuthorityError::Io {
                path: root.clone(),
                source,
            })?;
            let path = root.join(key.file_name());
            if path.exists() {
                let existing = read_settings(&path)?;
                compare(key, &existing, settings)?;
                records.insert(key.clone(), existing);
                return Ok(());
            }
            let bytes = serde_json::to_vec_pretty(settings)?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => file
                    .write_all(&bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|source| ThreadAuthorityError::Io {
                        path: path.clone(),
                        source,
                    })?,
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = read_settings(&path)?;
                    compare(key, &existing, settings)?;
                    records.insert(key.clone(), existing);
                    return Ok(());
                }
                Err(source) => {
                    return Err(ThreadAuthorityError::Io {
                        path: path.clone(),
                        source,
                    })
                }
            }
        }
        records.insert(key.clone(), settings.clone());
        Ok(())
    }

    /// Resolve host-bound settings for an existing thread.
    ///
    /// Callers must not reconstruct authority-bearing workspace settings from
    /// request input. Persistent stores lazily reload the immutable record so
    /// authority remains available after a daemon restart.
    pub fn get(
        &self,
        key: &ThreadAuthorityKey,
    ) -> Result<Option<ThreadSettings>, ThreadAuthorityError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| ThreadAuthorityError::Poisoned)?;
        if let Some(settings) = records.get(key) {
            return Ok(Some(settings.clone()));
        }
        let Some(root) = &self.root else {
            return Ok(None);
        };
        let path = root.join(key.file_name());
        if !path.exists() {
            return Ok(None);
        }
        let settings = read_settings(&path)?;
        records.insert(key.clone(), settings.clone());
        Ok(Some(settings))
    }
}

fn read_settings(path: &Path) -> Result<ThreadSettings, ThreadAuthorityError> {
    let bytes = fs::read(path).map_err(|source| ThreadAuthorityError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn compare(
    key: &ThreadAuthorityKey,
    existing: &ThreadSettings,
    requested: &ThreadSettings,
) -> Result<(), ThreadAuthorityError> {
    if existing == requested {
        Ok(())
    } else {
        Err(ThreadAuthorityError::Conflict {
            principal: key.principal.clone(),
            thread: key.thread.clone(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThreadAuthorityError {
    #[error("thread authority conflict for principal {principal:?} and thread {thread:?}")]
    Conflict {
        principal: PrincipalId,
        thread: ThreadId,
    },
    #[error("thread authority store lock is poisoned")]
    Poisoned,
    #[error("XDG_STATE_HOME or HOME is required for persistent thread authority")]
    StateRootUnavailable,
    #[error("thread authority I/O at '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("thread authority record is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_workspace_is_bound_once_per_authenticated_user() {
        let store = ThreadAuthorityStore::in_memory();
        let key =
            ThreadAuthorityKey::new(PrincipalId::local_uid(1001), ThreadId("thread-a".into()));
        let settings = |cwd: &str| ThreadSettings {
            workspace: WorkspacePolicy::from_resolved_roots(cwd.into(), vec![]).unwrap(),
            permission_profile: PermissionProfileId::workspace_write(),
            approval_policy: ApprovalPolicy::OnRequest,
            model_policy: None,
        };
        store
            .bind_or_verify(&key, &settings("/tmp/project"))
            .unwrap();
        assert!(matches!(
            store.bind_or_verify(&key, &settings("/etc")),
            Err(ThreadAuthorityError::Conflict { .. })
        ));
    }

    #[test]
    fn persistent_authority_can_be_resolved_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let key =
            ThreadAuthorityKey::new(PrincipalId::local_uid(1001), ThreadId("thread-a".into()));
        let context = fabric::PrincipalContext::new(
            PrincipalId::local_uid(1001),
            fabric::LocalOsPrincipal {
                uid: 1001,
                gid: 1001,
            },
            fabric::ConnectionId::new(),
            ThreadId("thread-a".into()),
            WorkspacePolicy::from_resolved_roots(root.path().to_path_buf(), vec![]).unwrap(),
            PermissionProfileId::workspace_write(),
            ApprovalPolicy::OnRequest,
        );
        let expected = ThreadSettings::from_context(&context, None);
        ThreadAuthorityStore::persistent(root.path().join("authority"))
            .bind_or_verify(&key, &expected)
            .unwrap();

        let reopened = ThreadAuthorityStore::persistent(root.path().join("authority"));
        assert_eq!(reopened.get(&key).unwrap(), Some(expected));
    }
}
