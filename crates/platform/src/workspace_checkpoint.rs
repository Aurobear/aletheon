//! Fail-closed workspace checkpoint filesystem adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use application::workspace_checkpoint::{
    CaptureResult, CheckpointFileEntry, WorkspaceFilePort, WorkspaceIdentity,
};
use async_trait::async_trait;
use uuid::Uuid;

#[derive(Default)]
pub struct LocalWorkspaceFilePort;

#[async_trait]
impl WorkspaceFilePort for LocalWorkspaceFilePort {
    async fn capture(
        &self,
        workspace: &WorkspaceIdentity,
        writable_roots: &[PathBuf],
        limit: usize,
    ) -> Result<CaptureResult> {
        capture_files(workspace, writable_roots, limit)
    }

    async fn protect_current(
        &self,
        workspace: &WorkspaceIdentity,
    ) -> Result<Vec<CheckpointFileEntry>> {
        Ok(capture_files(
            workspace,
            std::slice::from_ref(&workspace.canonical_path),
            usize::MAX,
        )?
        .files)
    }

    async fn restore(
        &self,
        workspace: &WorkspaceIdentity,
        target: &[CheckpointFileEntry],
        rollback: &[CheckpointFileEntry],
    ) -> Result<()> {
        if let Err(error) = apply_snapshot(&workspace.canonical_path, target) {
            return match apply_snapshot(&workspace.canonical_path, rollback) {
                Ok(()) => Err(error.context("restore failed; current workspace rolled back")),
                Err(rollback_error) => Err(anyhow!(
                    "restore failed ({error:#}); rollback also failed ({rollback_error:#})"
                )),
            };
        }
        Ok(())
    }
}

fn capture_files(
    workspace: &WorkspaceIdentity,
    writable_roots: &[PathBuf],
    limit: usize,
) -> Result<CaptureResult> {
    let root = canonical_directory(&workspace.canonical_path)?;
    let mut files = BTreeMap::new();
    let mut truncated = false;
    for writable_root in writable_roots {
        let writable_root = canonical_directory(writable_root)?;
        anyhow::ensure!(
            writable_root.starts_with(&root),
            "writable root escapes checkpoint workspace"
        );
        walk_files(&root, &writable_root, limit, &mut files, &mut truncated)?;
        if truncated {
            break;
        }
    }
    Ok(CaptureResult {
        files: files.into_values().collect(),
        truncated,
    })
}

fn walk_files(
    workspace: &Path,
    directory: &Path,
    limit: usize,
    files: &mut BTreeMap<PathBuf, CheckpointFileEntry>,
    truncated: &mut bool,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("read checkpoint directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk_files(workspace, &path, limit, files, truncated)?;
            if *truncated {
                return Ok(());
            }
        } else if file_type.is_file() {
            if files.len() >= limit {
                *truncated = true;
                return Ok(());
            }
            let relative = path
                .strip_prefix(workspace)
                .context("checkpoint file escaped workspace")?
                .to_path_buf();
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("read checkpoint file {}", path.display()))?;
            files.insert(
                relative.clone(),
                CheckpointFileEntry {
                    path: relative,
                    content: Some(content),
                },
            );
        }
    }
    Ok(())
}

fn apply_snapshot(workspace: &Path, target: &[CheckpointFileEntry]) -> Result<()> {
    let workspace = canonical_directory(workspace)?;
    let target_paths = target
        .iter()
        .map(|entry| validated_target(&workspace, &entry.path).map(|_| entry.path.clone()))
        .collect::<Result<BTreeSet<_>>>()?;
    let current = capture_files(
        &WorkspaceIdentity {
            canonical_path: workspace.clone(),
            repo_fingerprint: None,
        },
        std::slice::from_ref(&workspace),
        usize::MAX,
    )?;
    for entry in current.files {
        if !target_paths.contains(&entry.path) {
            std::fs::remove_file(validated_target(&workspace, &entry.path)?)?;
        }
    }
    for entry in target {
        let destination = validated_target(&workspace, &entry.path)?;
        match &entry.content {
            None => {
                if destination.exists() {
                    std::fs::remove_file(destination)?;
                }
            }
            Some(content) => {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let temporary =
                    destination.with_extension(format!("aletheon-rewind-{}.tmp", Uuid::new_v4()));
                std::fs::write(&temporary, content)?;
                std::fs::rename(temporary, destination)?;
            }
        }
    }
    Ok(())
}

fn validated_target(workspace: &Path, relative: &Path) -> Result<PathBuf> {
    anyhow::ensure!(relative.is_relative(), "checkpoint path must be relative");
    anyhow::ensure!(
        !relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)),
        "checkpoint path contains parent traversal"
    );
    Ok(workspace.join(relative))
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize checkpoint root {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "checkpoint root is not a directory");
    Ok(canonical)
}
