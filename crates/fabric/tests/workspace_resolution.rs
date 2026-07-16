use std::path::{Path, PathBuf};

use fabric::{PermissionProfileId, WorkspaceResolveError, WorkspaceSelection};

#[test]
fn relative_add_dir_uses_final_cwd_and_deduplicates() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join("project/shared")).unwrap();

    let policy = WorkspaceSelection::new(
        Some(PathBuf::from("project")),
        vec![PathBuf::from("shared"), PathBuf::from("shared")],
    )
    .resolve(tree.path())
    .unwrap();

    assert_eq!(
        policy.cwd(),
        tree.path().join("project").canonicalize().unwrap()
    );
    assert_eq!(policy.writable_roots().len(), 2);
    assert_eq!(
        policy.writable_roots()[1],
        tree.path().join("project/shared").canonicalize().unwrap()
    );
}

#[test]
fn cwd_override_is_relative_to_process_cwd_and_all_roots_are_absolute() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join("project/extra")).unwrap();
    let policy = WorkspaceSelection::new(
        Some("project".into()),
        vec![tree.path().join("project/extra")],
    )
    .resolve(tree.path())
    .unwrap();

    assert!(policy.cwd().is_absolute());
    assert!(policy
        .writable_roots()
        .iter()
        .all(|path| path.is_absolute()));
}

#[test]
fn missing_path_preserves_the_exact_input_and_os_error() {
    let tree = tempfile::tempdir().unwrap();
    let missing = tree.path().join("missing");
    let error = WorkspaceSelection::new(Some(missing.clone()), vec![])
        .resolve(tree.path())
        .unwrap_err();

    match error {
        WorkspaceResolveError::Filesystem { path, source } => {
            assert_eq!(path, missing);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn files_are_rejected_as_directories() {
    let tree = tempfile::tempdir().unwrap();
    let file = tree.path().join("file");
    std::fs::write(&file, "not a directory").unwrap();
    let error = WorkspaceSelection::new(Some(file.clone()), vec![])
        .resolve(tree.path())
        .unwrap_err();
    assert!(matches!(
        error,
        WorkspaceResolveError::Filesystem { path, source }
            if path == file && source.kind() == std::io::ErrorKind::NotADirectory
    ));
}

#[test]
fn symlinks_are_canonicalized_before_deduplication() {
    use std::os::unix::fs::symlink;

    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir(tree.path().join("real")).unwrap();
    symlink(tree.path().join("real"), tree.path().join("alias")).unwrap();
    let policy = WorkspaceSelection::new(
        Some(tree.path().join("real")),
        vec![tree.path().join("alias")],
    )
    .resolve(tree.path())
    .unwrap();
    assert_eq!(policy.writable_roots().len(), 1);
}

#[test]
fn filesystem_root_requires_explicit_selection_and_permitting_profile() {
    let implicit = WorkspaceSelection::new(None, vec![])
        .resolve_with_profile(Path::new("/"), &PermissionProfileId::workspace_write());
    assert!(matches!(
        implicit,
        Err(WorkspaceResolveError::ImplicitFilesystemRoot)
    ));

    let explicit_denied = WorkspaceSelection::new(Some(PathBuf::from("/")), vec![])
        .resolve_with_profile(Path::new("/tmp"), &PermissionProfileId::workspace_write());
    assert!(matches!(
        explicit_denied,
        Err(WorkspaceResolveError::FilesystemRootDenied { .. })
    ));

    let explicit_allowed = WorkspaceSelection::new(Some(PathBuf::from("/")), vec![])
        .resolve_with_profile(
            Path::new("/tmp"),
            &PermissionProfileId::danger_full_access(),
        )
        .unwrap();
    assert_eq!(explicit_allowed.cwd(), Path::new("/"));
}

#[test]
fn filesystem_root_as_an_additional_root_also_requires_permission() {
    let error = WorkspaceSelection::new(Some(PathBuf::from("/tmp")), vec![PathBuf::from("/")])
        .resolve_with_profile(Path::new("/tmp"), &PermissionProfileId::workspace_write())
        .unwrap_err();
    assert!(matches!(
        error,
        WorkspaceResolveError::FilesystemRootDenied { .. }
    ));
}
