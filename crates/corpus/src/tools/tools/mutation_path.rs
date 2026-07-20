use std::path::{Component, Path, PathBuf};

use fabric::{ProtectedPathPolicy, WorkspacePolicy};

const PROTECTED_METADATA_COMPONENTS: &[&str] = &[".git", ".aletheon"];

/// Resolve a prospective mutation target against every canonical writable
/// root. Canonicalizing the nearest existing ancestor prevents a symlink from
/// redirecting a newly-created child outside the approved roots.
pub fn validate_mutation_path(
    workspace: &WorkspacePolicy,
    protected: &ProtectedPathPolicy,
    requested: &Path,
) -> Result<PathBuf, String> {
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.cwd().join(requested)
    };
    reject_protected(&candidate, protected)?;

    let (ancestor, suffix) = nearest_existing_ancestor(&candidate)?;
    let mut resolved = std::fs::canonicalize(&ancestor).map_err(|error| {
        format!(
            "invalid mutation ancestor '{}': {error}",
            ancestor.display()
        )
    })?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    reject_protected(&resolved, protected)?;

    if !workspace
        .writable_roots()
        .iter()
        .any(|root| resolved.starts_with(root))
    {
        let roots = workspace
            .writable_roots()
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "mutation path '{}' was denied by the configured sandbox/working-directory policy; the writable roots are [{}]. Host mount state was not checked, so do not change host mounts. Relaunch from the intended working directory or choose a path inside the approved working directory.",
            resolved.display(), roots
        ));
    }
    Ok(resolved)
}

fn nearest_existing_ancestor(
    candidate: &Path,
) -> Result<(PathBuf, Vec<std::ffi::OsString>), String> {
    let mut missing = Vec::new();
    let mut ancestor = candidate;
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| "mutation path has no existing ancestor".to_string())?;
        missing.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| "mutation path has no existing ancestor".to_string())?;
    }
    Ok((ancestor.to_path_buf(), missing))
}

fn reject_protected(path: &Path, protected: &ProtectedPathPolicy) -> Result<(), String> {
    for component in path.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        let value = value.to_string_lossy();
        if PROTECTED_METADATA_COMPONENTS.contains(&value.as_ref()) {
            return Err(format!("protected mutation path component: {value}"));
        }
    }
    if let Some(credential) = protected
        .credential_paths()
        .iter()
        .find(|credential| path.starts_with(credential))
    {
        return Err(format!(
            "protected configured credential path: {}",
            credential.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        add_dir: PathBuf,
        outside: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            let add_dir = temp.path().join("shared");
            let outside = temp.path().join("outside");
            for path in [&root, &add_dir, &outside] {
                std::fs::create_dir(path).unwrap();
            }
            Self {
                _temp: temp,
                root,
                add_dir,
                outside,
            }
        }

        fn workspace(&self) -> WorkspacePolicy {
            WorkspacePolicy::from_resolved_roots(
                self.root.canonicalize().unwrap(),
                vec![self.add_dir.canonicalize().unwrap()],
            )
            .unwrap()
        }
    }

    #[test]
    fn accepts_new_files_beneath_every_writable_root() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let protected = ProtectedPathPolicy::default();
        assert!(validate_mutation_path(&workspace, &protected, Path::new("docs/plan.md")).is_ok());
        assert!(
            validate_mutation_path(&workspace, &protected, &fixture.add_dir.join("ok.txt")).is_ok()
        );
    }

    #[test]
    fn rejects_parent_and_absolute_escape() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let protected = ProtectedPathPolicy::default();
        assert!(validate_mutation_path(&workspace, &protected, Path::new("../escape")).is_err());
        assert!(
            validate_mutation_path(&workspace, &protected, &fixture.outside.join("sibling"))
                .is_err()
        );
    }

    #[test]
    fn outside_path_diagnostic_identifies_policy_and_safe_recovery() {
        let fixture = Fixture::new();
        let error = validate_mutation_path(
            &fixture.workspace(),
            &ProtectedPathPolicy::default(),
            &fixture.outside.join("sibling"),
        )
        .unwrap_err();
        let lower = error.to_lowercase();
        assert!(error.contains("configured sandbox/working-directory policy"));
        assert!(error.contains(&fixture.root.display().to_string()));
        assert!(error.contains(&fixture.add_dir.display().to_string()));
        assert!(lower.contains("host mount state was not checked"));
        assert!(lower.contains("do not change host mounts"));
        assert!(lower.contains("relaunch from the intended working directory"));
        assert!(lower.contains("inside the approved working directory"));
        assert!(!lower.contains("sudo mount"));
        assert!(!lower.contains("mount -o"));
    }

    #[cfg(unix)]
    #[test]
    fn add_dir_metadata_and_symlink_escape_are_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let protected = ProtectedPathPolicy::default();
        assert!(validate_mutation_path(
            &workspace,
            &protected,
            &fixture.add_dir.join(".git/config")
        )
        .is_err());
        symlink(&fixture.outside, fixture.add_dir.join("escape")).unwrap();
        assert!(validate_mutation_path(
            &workspace,
            &protected,
            &fixture.add_dir.join("escape/file")
        )
        .is_err());
    }

    #[test]
    fn only_explicit_credential_paths_are_protected() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let credential = fixture.root.join("secrets/client.pem");
        let protected = ProtectedPathPolicy::new(vec![credential.clone()]).unwrap();
        assert!(validate_mutation_path(&workspace, &protected, &credential).is_err());
        assert!(validate_mutation_path(
            &workspace,
            &ProtectedPathPolicy::default(),
            Path::new("ordinary.key")
        )
        .is_ok());
    }
}
