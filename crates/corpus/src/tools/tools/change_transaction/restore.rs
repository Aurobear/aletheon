//! Workspace baseline capture, comparison, and rollback primitives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ::contracts::change_transaction::{ChangeTransactionSnapshot, WorkspaceVersion};

#[derive(Clone)]
pub(super) struct BaselineRestore {
    basis: ::contracts::change_transaction::WorkspaceVersionBasis,
    nodes: HashMap<String, RestoreNode>,
}

#[derive(Clone, PartialEq, Eq)]
enum RestoreNode {
    File { bytes: Vec<u8>, mode: u32 },
    Symlink(PathBuf),
    Missing,
}

impl BaselineRestore {
    pub(super) fn capture(root: &Path, baseline: &WorkspaceVersion) -> anyhow::Result<Self> {
        let paths = match baseline.basis {
            ::contracts::change_transaction::WorkspaceVersionBasis::GitWorktree => {
                baseline.changed_paths.clone()
            }
            ::contracts::change_transaction::WorkspaceVersionBasis::BoundedTree => {
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

    pub(super) fn restore(
        &self,
        root: &Path,
        snapshot: &ChangeTransactionSnapshot,
    ) -> anyhow::Result<()> {
        let mut paths = snapshot.current.changed_paths.clone();
        paths.extend(snapshot.baseline.changed_paths.iter().cloned());
        paths.sort();
        paths.dedup();
        for relative in paths {
            ensure_relative_workspace_path(&relative)?;
            let node = if let Some(node) = self.nodes.get(&relative) {
                node.clone()
            } else if self.basis
                == ::contracts::change_transaction::WorkspaceVersionBasis::GitWorktree
            {
                git_head_node(root, &relative)?
            } else {
                RestoreNode::Missing
            };
            restore_node(&root.join(&relative), node)?;
        }
        Ok(())
    }

    pub(super) fn changed_paths(
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

    pub(super) fn render_diff(
        &self,
        root: &Path,
        changed_paths: &[String],
    ) -> anyhow::Result<Vec<u8>> {
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
