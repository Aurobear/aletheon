use std::path::{Component, Path, PathBuf};

use ::contracts::ToolContext;
use platform::{FilesystemAccess, FilesystemHost, FilesystemScope, HostPath, SymlinkPolicy};

pub(crate) struct ScopedFilesystem {
    pub host: Box<dyn FilesystemHost>,
    pub path: HostPath,
}

pub(crate) fn open(
    context: &ToolContext,
    requested: &Path,
    access: FilesystemAccess,
) -> Result<ScopedFilesystem, String> {
    let workspace = context.effective_workspace_policy()?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.cwd().join(requested)
    };
    reject_sensitive_read(&candidate, workspace.protected_paths())?;

    let mut roots = workspace.writable_roots().to_vec();
    let mut readable_paths = Vec::new();
    if let Some(authority) = &context.approval_authority {
        if authority.granted_scope.allowed_paths.is_empty() {
            return Err("filesystem permit has an empty path scope".into());
        }
        let granted = authority
            .granted_scope
            .allowed_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        roots.retain(|root| granted.iter().any(|path| path == root));

        if !roots.iter().any(|root| candidate.starts_with(root)) {
            if access != FilesystemAccess::ReadOnly {
                return Err(format!(
                    "write path '{}' is outside the admitted workspace roots",
                    candidate.display()
                ));
            }
            let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
                format!(
                    "external read path '{}' cannot be resolved: {error}",
                    candidate.display()
                )
            })?;
            if !granted.iter().any(|path| path == &canonical) {
                return Err(format!(
                    "external read path '{}' is absent from the Kernel permit",
                    canonical.display()
                ));
            }
            readable_paths.push(canonical);
        }
    }

    if roots.is_empty() && readable_paths.is_empty() {
        return Err(
            "filesystem authority has no path admitted by both workspace and permit".into(),
        );
    }
    let mut traversal_roots = Vec::new();
    for root in roots {
        let traversal = writable_traversal_root(&root)?;
        if !traversal_roots.contains(&traversal) {
            traversal_roots.push(traversal);
        }
    }
    let host = platform::open_filesystem(FilesystemScope {
        roots: traversal_roots.into_iter().map(HostPath::new).collect(),
        readable_paths: readable_paths.into_iter().map(HostPath::new).collect(),
        access,
        symlink_policy: SymlinkPolicy::WithinRoot,
    })
    .map_err(|error| error.to_string())?;
    Ok(ScopedFilesystem {
        host,
        path: HostPath::new(candidate),
    })
}

fn writable_traversal_root(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            format!(
                "writable path has no existing directory ancestor: {}",
                path.display()
            )
        })?;
    }
    let directory = if ancestor.is_dir() {
        ancestor
    } else {
        ancestor
            .parent()
            .ok_or_else(|| format!("writable file has no parent directory: {}", path.display()))?
    };
    std::fs::canonicalize(directory).map_err(|error| {
        format!(
            "writable traversal root '{}' cannot be resolved: {error}",
            directory.display()
        )
    })
}

fn reject_sensitive_read(
    candidate: &Path,
    protected: &::contracts::ProtectedPathPolicy,
) -> Result<(), String> {
    if protected
        .credential_paths()
        .iter()
        .any(|path| candidate.starts_with(path))
    {
        return Err(format!(
            "credential path '{}' is not available through generic file tools",
            candidate.display()
        ));
    }
    let sensitive_component = candidate.components().any(|component| {
        matches!(component, Component::Normal(value) if matches!(value.to_str(), Some(".ssh" | ".aws" | ".gnupg" | ".aletheon")))
    });
    let sensitive_system_file = matches!(
        candidate.to_str(),
        Some("/etc/shadow" | "/etc/gshadow" | "/etc/security/opasswd")
    );
    let proc_environment = candidate
        .components()
        .collect::<Vec<_>>()
        .windows(3)
        .any(|parts| {
            matches!(parts, [Component::RootDir, Component::Normal(proc), Component::Normal(pid)] if *proc == "proc" && (pid.to_string_lossy().chars().all(|ch| ch.is_ascii_digit()) || matches!(pid.to_str(), Some("self" | "thread-self"))))
        })
        && candidate.file_name().is_some_and(|name| name == "environ");
    let sensitive_file_name = candidate.file_name().is_some_and(|name| {
        matches!(
            name.to_str(),
            Some(".env" | "credentials.vault" | "id_rsa" | "id_ed25519")
        )
    });
    if sensitive_component || sensitive_system_file || proc_environment || sensitive_file_name {
        return Err(format!(
            "sensitive path '{}' requires a dedicated credential or system interface",
            candidate.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tools::{file_read::FileReadTool, file_write::FileWriteTool, Tool};
    use serde_json::json;
    use std::sync::Arc;

    fn context(root: &Path, allowed_paths: Vec<String>) -> ToolContext {
        let workspace =
            ::contracts::WorkspacePolicy::from_resolved_roots(root.to_path_buf(), vec![]).unwrap();
        ToolContext {
            agent: None,
            approval_authority: Some(::contracts::ToolApprovalAuthority {
                principal_id: ::contracts::PrincipalId("test".into()),
                connection_id: ::contracts::ConnectionId::new(),
                thread_id: ::contracts::ThreadId("test".into()),
                turn_id: ::contracts::TurnId::new(),
                call_id: "call".into(),
                workspace,
                granted_scope: ::contracts::CapabilityScope {
                    allowed_paths,
                    ..Default::default()
                },
                permission_mode: ::contracts::permission::HostPermissionMode::Safe,
            }),
            working_dir: root.to_path_buf(),
            session_id: "test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[test]
    fn generic_reads_reject_common_secret_locations() {
        let protected = ::contracts::ProtectedPathPolicy::default();
        for path in [
            "/home/user/.ssh/id_ed25519",
            "/etc/shadow",
            "/proc/1/environ",
            "/proc/self/environ",
            "/workspace/.env",
            "/workspace/.aletheon/credentials.vault",
        ] {
            assert!(reject_sensitive_read(Path::new(path), &protected).is_err());
        }
        assert!(reject_sensitive_read(Path::new("/etc/os-release"), &protected).is_ok());
    }

    #[test]
    fn governed_filesystem_fails_closed_on_empty_permit_paths() {
        let root = tempfile::tempdir().unwrap();
        let context = context(root.path(), vec![]);
        let error = open(&context, Path::new("file.txt"), FilesystemAccess::ReadOnly)
            .err()
            .expect("empty filesystem permit must fail closed");
        assert!(error.contains("empty path scope"));
    }

    #[tokio::test]
    async fn exact_external_read_does_not_grant_sibling_access() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let admitted = external.path().join("admitted.txt");
        let sibling = external.path().join("sibling.txt");
        std::fs::write(&admitted, b"allowed").unwrap();
        std::fs::write(&sibling, b"denied").unwrap();
        let context = context(
            workspace.path(),
            vec![
                workspace.path().to_string_lossy().into_owned(),
                admitted
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ],
        );
        let scoped = open(&context, &admitted, FilesystemAccess::ReadOnly).unwrap();
        assert_eq!(scoped.host.read(&scoped.path).await.unwrap(), b"allowed");
        let error = open(&context, &sibling, FilesystemAccess::ReadOnly)
            .err()
            .expect("sibling was not granted by Kernel");
        assert!(error.contains("absent from the Kernel permit"));
    }

    #[tokio::test]
    async fn file_tools_refuse_an_empty_governed_scope() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("file.txt");
        std::fs::write(&path, "existing").unwrap();
        let context = context(workspace.path(), vec![]);

        let read = FileReadTool.execute(json!({"path": path}), &context).await;
        let write = FileWriteTool
            .execute(json!({"path": path, "content": "changed"}), &context)
            .await;

        assert!(read.is_error);
        assert!(read.content.contains("empty path scope"));
        assert!(write.is_error);
        assert!(write.content.contains("empty path scope"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    }

    #[tokio::test]
    async fn exact_writable_file_scope_allows_target_and_rejects_sibling() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("target.txt");
        let sibling = workspace.path().join("sibling.txt");
        std::fs::write(&target, "original").unwrap();
        std::fs::write(&sibling, "sibling").unwrap();
        let policy = ::contracts::WorkspacePolicy::from_resolved_roots(
            workspace.path().to_path_buf(),
            Vec::new(),
        )
        .unwrap()
        .narrow_to_declared_paths(&["target.txt".into()])
        .unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: Some(::contracts::ToolApprovalAuthority {
                principal_id: ::contracts::PrincipalId("test".into()),
                connection_id: ::contracts::ConnectionId::new(),
                thread_id: ::contracts::ThreadId("test".into()),
                turn_id: ::contracts::TurnId::new(),
                call_id: "call".into(),
                workspace: policy,
                granted_scope: ::contracts::CapabilityScope {
                    allowed_paths: vec![target.to_string_lossy().into_owned()],
                    ..Default::default()
                },
                permission_mode: ::contracts::permission::HostPermissionMode::Safe,
            }),
            working_dir: workspace.path().to_path_buf(),
            session_id: "test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let accepted = FileWriteTool
            .execute(json!({"path": target, "content": "changed"}), &context)
            .await;
        let rejected = FileWriteTool
            .execute(json!({"path": sibling, "content": "escaped"}), &context)
            .await;

        assert!(!accepted.is_error, "{}", accepted.content);
        assert!(rejected.is_error);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "changed");
        assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "sibling");
    }

    #[tokio::test]
    async fn file_tools_read_and_write_inside_the_admitted_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = context(&root, vec![root.to_string_lossy().into_owned()]);
        let path = root.join("nested/file.txt");

        let write = FileWriteTool
            .execute(json!({"path": path, "content": "hello\nworld\n"}), &context)
            .await;
        assert!(!write.is_error, "{}", write.content);
        let delta = write
            .metadata
            .patch_delta
            .expect("successful file_write emits a typed delta");
        assert_eq!(delta.files_changed.len(), 1);
        assert_eq!(delta.files_changed[0].path, "nested/file.txt");
        assert_eq!(delta.files_changed[0].bytes_before, 0);
        assert_eq!(delta.files_changed[0].bytes_after, 12);

        let read = FileReadTool
            .execute(json!({"path": path, "offset": 1, "limit": 1}), &context)
            .await;
        assert!(!read.is_error, "{}", read.content);
        let receipt: serde_json::Value = serde_json::from_str(&read.content).unwrap();
        assert_eq!(receipt["files"][0]["content"], "    2\tworld");
        assert_eq!(receipt["files"][0]["line_range"]["start"], 2);
        assert_eq!(receipt["files"][0]["line_range"]["end"], 2);
        assert_eq!(receipt["files"][0]["sha256"].as_str().unwrap().len(), 64);
    }

    #[tokio::test]
    async fn file_read_batches_known_files_in_one_call() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = context(&root, vec![root.to_string_lossy().into_owned()]);
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        std::fs::write(root.join("README.md"), "# Demo\n").unwrap();

        let read = FileReadTool
            .execute(
                json!({"paths": ["Cargo.toml", "README.md"], "limit": 20}),
                &context,
            )
            .await;

        assert!(!read.is_error, "{}", read.content);
        let receipt: serde_json::Value = serde_json::from_str(&read.content).unwrap();
        assert_eq!(receipt["files"][0]["path"], "Cargo.toml");
        assert!(receipt["files"][0]["content"]
            .as_str()
            .unwrap()
            .contains("name = \"demo\""));
        assert_eq!(receipt["files"][1]["path"], "README.md");
        assert!(receipt["files"][1]["content"]
            .as_str()
            .unwrap()
            .contains("# Demo"));
        assert!(receipt["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["artifact_ref"]
                .as_str()
                .is_some_and(|reference| reference.starts_with("artifact://sha256/"))));
    }

    #[tokio::test]
    async fn file_read_batch_reports_missing_member_without_failing_successes() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = context(&root, vec![root.to_string_lossy().into_owned()]);
        std::fs::write(root.join("README.md"), "# Demo\n").unwrap();

        let read = FileReadTool
            .execute(
                json!({"paths": ["README.md", "package.json"], "limit": 20}),
                &context,
            )
            .await;

        assert!(!read.is_error, "{}", read.content);
        assert!(read.content.contains("README.md"));
        assert!(read.content.contains("# Demo"));
        assert!(read.content.contains("package.json"));
        assert!(read.content.contains("Failed to read package.json"));
    }

    #[tokio::test]
    async fn file_read_accepts_only_the_exact_external_path_in_the_permit() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let admitted = external.path().join("admitted.txt");
        let sibling = external.path().join("sibling.txt");
        std::fs::write(&admitted, "allowed").unwrap();
        std::fs::write(&sibling, "denied").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let admitted = admitted.canonicalize().unwrap();
        let context = context(
            &root,
            vec![
                root.to_string_lossy().into_owned(),
                admitted.to_string_lossy().into_owned(),
            ],
        );

        let allowed = FileReadTool
            .execute(json!({"path": admitted}), &context)
            .await;
        let denied = FileReadTool
            .execute(json!({"path": sibling}), &context)
            .await;

        assert!(!allowed.is_error, "{}", allowed.content);
        assert!(allowed.content.contains("allowed"));
        assert!(denied.is_error);
        assert!(denied.content.contains("absent from the Kernel permit"));
    }

    #[tokio::test]
    async fn generic_file_read_refuses_sensitive_paths_even_when_permitted() {
        let workspace = tempfile::tempdir().unwrap();
        let sensitive = workspace.path().join(".env");
        std::fs::write(&sensitive, "TOKEN=secret").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = context(&root, vec![root.to_string_lossy().into_owned()]);

        let result = FileReadTool
            .execute(json!({"path": sensitive}), &context)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("sensitive path"));
        assert!(!result.content.contains("TOKEN=secret"));
    }

    #[tokio::test]
    async fn file_write_refuses_external_paths_even_when_the_exact_path_is_permitted() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let target = external.path().join("target.txt");
        std::fs::write(&target, "unchanged").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let target = target.canonicalize().unwrap();
        let context = context(
            &root,
            vec![
                root.to_string_lossy().into_owned(),
                target.to_string_lossy().into_owned(),
            ],
        );

        let result = FileWriteTool
            .execute(json!({"path": target, "content": "changed"}), &context)
            .await;

        assert!(result.is_error);
        assert!(result
            .content
            .contains("outside the admitted workspace roots"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "unchanged");
    }
}
