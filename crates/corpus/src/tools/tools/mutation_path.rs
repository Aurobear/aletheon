use std::path::{Component, Path, PathBuf};

const PROTECTED_COMPONENTS: &[&str] = &[".git", ".aletheon", ".ssh"];

/// Resolve a prospective mutation target and confine it to the canonical
/// working directory. The nearest existing ancestor is canonicalized so a
/// symlink cannot redirect a newly-created child outside the project.
pub fn validate_mutation_path(working_dir: &Path, requested: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(working_dir)
        .map_err(|error| format!("invalid working directory: {error}"))?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };

    reject_protected(&candidate)?;

    let mut missing = Vec::new();
    let mut ancestor = candidate.as_path();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| "mutation path has no existing ancestor".to_string())?;
        missing.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| "mutation path escapes its working directory".to_string())?;
    }

    let mut resolved = std::fs::canonicalize(ancestor)
        .map_err(|error| format!("invalid mutation path: {error}"))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    if !resolved.starts_with(&root) {
        return Err(format!(
            "mutation path '{}' was denied by the configured sandbox/working-directory policy; the approved working directory is '{}'. Host mount state was not checked, so do not change host mounts. Relaunch from the intended working directory or choose a path inside the approved working directory.",
            resolved.display(),
            root.display()
        ));
    }
    Ok(resolved)
}

fn reject_protected(path: &Path) -> Result<(), String> {
    for component in path.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        let value = value.to_string_lossy();
        if PROTECTED_COMPONENTS.contains(&value.as_ref())
            || value == ".env"
            || value.starts_with(".env.")
            || value.starts_with("client_secret_")
            || value.ends_with(".pem")
            || value.ends_with(".key")
        {
            return Err(format!("protected mutation path component: {value}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        (temp, root)
    }

    #[test]
    fn accepts_new_file_beneath_workspace() {
        let (_temp, root) = fixture();
        let path = validate_mutation_path(&root, Path::new("docs/plan.md")).unwrap();
        assert!(path.starts_with(std::fs::canonicalize(root).unwrap()));
    }

    #[test]
    fn rejects_parent_and_absolute_escape() {
        let (temp, root) = fixture();
        assert!(validate_mutation_path(&root, Path::new("../escape")).is_err());
        assert!(validate_mutation_path(&root, &temp.path().join("sibling")).is_err());
    }

    #[test]
    fn outside_path_diagnostic_identifies_policy_and_safe_recovery() {
        let (temp, root) = fixture();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let error = validate_mutation_path(&root, &temp.path().join("sibling")).unwrap_err();
        let lower = error.to_lowercase();

        assert!(error.contains("configured sandbox/working-directory policy"));
        assert!(error.contains(&canonical_root.display().to_string()));
        assert!(lower.contains("host mount state was not checked"));
        assert!(lower.contains("do not change host mounts"));
        assert!(lower.contains("relaunch from the intended working directory"));
        assert!(lower.contains("inside the approved working directory"));
        assert!(!lower.contains("sudo mount"));
        assert!(!lower.contains("mount -o"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_and_protected_metadata() {
        use std::os::unix::fs::symlink;
        let (temp, root) = fixture();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("link")).unwrap();
        assert!(validate_mutation_path(&root, Path::new("link/file")).is_err());
        assert!(validate_mutation_path(&root, Path::new(".git/config")).is_err());
        assert!(validate_mutation_path(&root, Path::new("client_secret_oauth.json")).is_err());
    }
}
