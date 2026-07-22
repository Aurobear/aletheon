//! Deployment storage and quota construction for daemon bootstrap.

use std::collections::HashMap;

pub(super) fn deployment_storage_quota(
    deployment: &cognit::config::DeploymentConfig,
) -> anyhow::Result<crate::r#impl::storage_quota::StorageQuota> {
    use crate::r#impl::storage_quota::{StorageClass, StorageLimit, StorageQuota, StorageRoot};

    let quotas = &deployment.quotas;
    let paths = &deployment.paths;
    let roots = HashMap::from([
        (
            StorageClass::Total,
            StorageRoot {
                path: paths.state_root.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.total_data_soft_bytes,
                    hard_bytes: quotas.total_data_bytes,
                    hard_items: quotas.total_data_items,
                },
            },
        ),
        (
            StorageClass::Artifacts,
            StorageRoot {
                path: paths.artifacts.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.artifacts_soft_bytes,
                    hard_bytes: quotas.artifacts_bytes,
                    hard_items: quotas.artifacts_items,
                },
            },
        ),
        (
            StorageClass::Worktrees,
            StorageRoot {
                path: paths.worktrees.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.worktrees_soft_bytes,
                    hard_bytes: quotas.worktrees_bytes,
                    hard_items: quotas.worktrees_items,
                },
            },
        ),
        (
            StorageClass::Audit,
            StorageRoot {
                path: paths.audit.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.audit_soft_bytes,
                    hard_bytes: quotas.audit_bytes,
                    hard_items: quotas.audit_items,
                },
            },
        ),
        (
            StorageClass::Sessions,
            StorageRoot {
                path: paths.sessions.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.sessions_soft_bytes,
                    hard_bytes: quotas.sessions_bytes,
                    hard_items: quotas.sessions_items,
                },
            },
        ),
        (
            StorageClass::Google,
            StorageRoot {
                path: paths.state.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.google_soft_bytes,
                    hard_bytes: quotas.google_bytes,
                    hard_items: quotas.google_items,
                },
            },
        ),
        (
            StorageClass::GbrainSpool,
            StorageRoot {
                path: paths.mnemosyne.clone(),
                limit: StorageLimit {
                    soft_bytes: quotas.supplemental_spool_soft_bytes,
                    hard_bytes: quotas.supplemental_spool_bytes,
                    hard_items: quotas.supplemental_spool_items,
                },
            },
        ),
    ]);
    StorageQuota::new(roots).map_err(Into::into)
}
