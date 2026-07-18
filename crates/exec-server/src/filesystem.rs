//! Workspace-confined filesystem operations for exec-server.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::protocol::*;

const MAX_READ_BYTES: u64 = 100 * 1024 * 1024;

/// Canonical roots supplied by the trusted daemon at child creation time.
pub struct WorkspaceRoots {
    roots: Vec<PathBuf>,
}

impl WorkspaceRoots {
    pub fn from_env() -> std::io::Result<Self> {
        let encoded = std::env::var("ALETHEON_EXEC_SERVER_WORKSPACE_ROOTS").map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ALETHEON_EXEC_SERVER_WORKSPACE_ROOTS must be set",
            )
        })?;
        let configured: Vec<PathBuf> = serde_json::from_str(&encoded).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid exec-server workspace roots: {error}"),
            )
        })?;
        Self::new(configured)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    }

    fn new(configured: Vec<PathBuf>) -> Result<Self, String> {
        if configured.is_empty() {
            return Err("exec-server requires at least one workspace root".into());
        }
        let roots = configured
            .into_iter()
            .map(|root| {
                root.canonicalize().map_err(|error| {
                    format!("workspace root {} is unavailable: {error}", root.display())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if roots.iter().any(|root| !root.is_dir()) {
            return Err("exec-server workspace roots must be directories".into());
        }
        Ok(Self { roots })
    }

    fn existing_path(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("path {} is unavailable: {error}", path.display()))?;
        self.ensure_contained(canonical)
    }

    fn write_path(&self, path: &Path) -> Result<PathBuf, String> {
        if path.exists() {
            return self.existing_path(path);
        }
        let name = path
            .file_name()
            .ok_or_else(|| "write path must name a file".to_string())?;
        let parent = path
            .parent()
            .ok_or_else(|| "write path must have a parent".to_string())?
            .canonicalize()
            .map_err(|error| format!("write parent is unavailable: {error}"))?;
        self.ensure_contained(parent.join(name))
    }

    fn ensure_contained(&self, canonical: PathBuf) -> Result<PathBuf, String> {
        if self.roots.iter().any(|root| canonical.starts_with(root)) {
            Ok(canonical)
        } else {
            Err("path is outside configured workspace roots".into())
        }
    }
}

#[derive(Deserialize)]
struct ReadParams {
    path: PathBuf,
}

#[derive(Deserialize)]
struct WriteParams {
    path: PathBuf,
    content: String,
}

/// Handle the currently supported fs RPC methods. All path decisions are made
/// server-side against canonical roots and fail closed.
pub fn handle_fs(
    method: &str,
    params: &serde_json::Value,
    workspace: &WorkspaceRoots,
) -> Option<Response> {
    let result = match method {
        "fs/read" => read(params, workspace),
        "fs/write" => write(params, workspace),
        method if method.starts_with("fs/") => Err((
            METHOD_NOT_FOUND,
            format!("Filesystem method not implemented: {method}"),
        )),
        _ => return None,
    };
    Some(match result {
        Ok(value) => Response::ok(serde_json::Value::Null, value),
        Err((code, message)) => Response::err(serde_json::Value::Null, code, message),
    })
}

fn read(
    params: &serde_json::Value,
    workspace: &WorkspaceRoots,
) -> Result<serde_json::Value, (i32, String)> {
    let params: ReadParams = serde_json::from_value(params.clone())
        .map_err(|error| (INVALID_PARAMS, format!("Invalid fs/read params: {error}")))?;
    let path = workspace
        .existing_path(&params.path)
        .map_err(|message| (FS_ACCESS_DENIED, message))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| (INTERNAL_ERROR, format!("read metadata failed: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_READ_BYTES {
        return Err((
            FS_ACCESS_DENIED,
            "file is not readable or exceeds 100MB".into(),
        ));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| (INTERNAL_ERROR, format!("read failed: {error}")))?;
    Ok(serde_json::json!({"size": content.len(), "content": content}))
}

fn write(
    params: &serde_json::Value,
    workspace: &WorkspaceRoots,
) -> Result<serde_json::Value, (i32, String)> {
    let params: WriteParams = serde_json::from_value(params.clone())
        .map_err(|error| (INVALID_PARAMS, format!("Invalid fs/write params: {error}")))?;
    let path = workspace
        .write_path(&params.path)
        .map_err(|message| (FS_ACCESS_DENIED, message))?;
    std::fs::write(&path, params.content.as_bytes())
        .map_err(|error| (INTERNAL_ERROR, format!("write failed: {error}")))?;
    Ok(serde_json::json!({"bytes_written": params.content.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_write_reject_paths_outside_roots_and_symlink_escapes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "nope").unwrap();
        let roots = WorkspaceRoots::new(vec![root.clone()]).unwrap();

        assert!(matches!(
            read(&serde_json::json!({"path": outside.join("secret")}), &roots),
            Err((FS_ACCESS_DENIED, _))
        ));
        assert!(matches!(
            write(
                &serde_json::json!({"path": outside.join("new"), "content": "nope"}),
                &roots
            ),
            Err((FS_ACCESS_DENIED, _))
        ));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("secret"), root.join("escape")).unwrap();
            assert!(matches!(
                read(&serde_json::json!({"path": root.join("escape")}), &roots),
                Err((FS_ACCESS_DENIED, _))
            ));
        }
    }

    #[test]
    fn permitted_workspace_read_and_write_succeed() {
        let temp = tempfile::tempdir().unwrap();
        let roots = WorkspaceRoots::new(vec![temp.path().to_path_buf()]).unwrap();
        let path = temp.path().join("file.txt");
        write(
            &serde_json::json!({"path": path, "content": "hello"}),
            &roots,
        )
        .unwrap();
        let result = read(&serde_json::json!({"path": path}), &roots).unwrap();
        assert_eq!(result["content"], "hello");
        assert_eq!(result["size"], 5);
    }
}
