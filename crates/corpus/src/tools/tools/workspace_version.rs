//! Deterministic version of the editable repository/workspace state.

use anyhow::{bail, Context, Result};
use fabric::change_transaction::{WorkspaceVersion, WorkspaceVersionBasis};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_FALLBACK_FILES: usize = 10_000;
const MAX_FALLBACK_BYTES: u64 = 128 * 1024 * 1024;

pub fn capture(root: &Path) -> Result<WorkspaceVersion> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", root.display()))?;
    capture_git(&root).or_else(|_| capture_tree(&root))
}

fn capture_git(root: &Path) -> Result<WorkspaceVersion> {
    let top = git_output(root, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(String::from_utf8(top)?.trim());
    let top = top.canonicalize()?;
    if !root.starts_with(&top) {
        bail!("workspace root is outside git worktree");
    }
    // Git deliberately hides ignored paths from both diff and untracked-file
    // inventory. When the admitted workspace itself is ignored, using Git as
    // the version basis would make writes invisible to review and rollback.
    // Fall back to the bounded tree snapshot for that scoped workspace.
    let ignored = std::process::Command::new("git")
        .args(["check-ignore", "--quiet", "--", "."])
        .current_dir(root)
        .status()
        .context("check whether workspace root is ignored")?;
    if ignored.success() {
        bail!("workspace root is ignored by git");
    }
    let head = String::from_utf8(git_output(root, &["rev-parse", "HEAD"])?)?
        .trim()
        .to_string();
    let diff = git_output(root, &["diff", "--binary", "HEAD", "--", "."])?;
    let untracked = git_output(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )?;
    let mut untracked_paths = untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    untracked_paths.sort();
    if untracked_paths.len() > MAX_FALLBACK_FILES {
        bail!("workspace has more than {MAX_FALLBACK_FILES} untracked files");
    }

    let mut hasher = Sha256::new();
    hasher.update(b"aletheon-workspace-version-v1\0git\0");
    hasher.update(head.as_bytes());
    hasher.update([0]);
    hasher.update(&diff);
    let mut changed_paths = untracked_paths.clone();
    let mut untracked_bytes = 0_u64;
    for relative in &untracked_paths {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspect untracked workspace file {relative}"))?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)?;
            hasher.update(b"symlink\0");
            hasher.update(target.as_os_str().as_encoded_bytes());
            continue;
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read untracked workspace file {relative}"))?;
        untracked_bytes = untracked_bytes.saturating_add(bytes.len() as u64);
        if untracked_bytes > MAX_FALLBACK_BYTES {
            bail!("untracked workspace content exceeds {MAX_FALLBACK_BYTES} bytes");
        }
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let names = git_output(root, &["diff", "--name-only", "HEAD", "--", "."])?;
    changed_paths.extend(
        String::from_utf8(names)?
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string),
    );
    changed_paths.sort();
    changed_paths.dedup();
    Ok(WorkspaceVersion {
        digest: format!("{:x}", hasher.finalize()),
        basis: WorkspaceVersionBasis::GitWorktree,
        root: root.display().to_string(),
        head: Some(head),
        changed_paths,
    })
}

fn capture_tree(root: &Path) -> Result<WorkspaceVersion> {
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path() != root && !entry.file_type().is_dir())
        .collect::<Vec<_>>();
    if entries.len() > MAX_FALLBACK_FILES {
        bail!("workspace tree exceeds {MAX_FALLBACK_FILES} files");
    }
    entries.sort_by_key(|entry| entry.path().to_path_buf());
    let mut total = 0_u64;
    let mut hasher = Sha256::new();
    hasher.update(b"aletheon-workspace-version-v1\0tree\0");
    let mut changed_paths = Vec::with_capacity(entries.len());
    for entry in entries {
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .into_owned();
        changed_paths.push(relative.clone());
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        if entry.file_type().is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            hasher.update(b"symlink\0");
            hasher.update(target.as_os_str().as_encoded_bytes());
            continue;
        }
        let bytes = std::fs::read(entry.path())?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_FALLBACK_BYTES {
            bail!("workspace tree exceeds {MAX_FALLBACK_BYTES} bytes");
        }
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(WorkspaceVersion {
        digest: format!("{:x}", hasher.finalize()),
        basis: WorkspaceVersionBasis::BoundedTree,
        root: root.display().to_string(),
        head: None,
        changed_paths,
    })
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_tree_version_changes_with_content_not_mtime() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), "one").unwrap();
        let first = capture(temp.path()).unwrap();
        let same = capture(temp.path()).unwrap();
        assert_eq!(first.digest, same.digest);
        std::fs::write(temp.path().join("a.txt"), "two").unwrap();
        let changed = capture(temp.path()).unwrap();
        assert_ne!(first.digest, changed.digest);
        assert_eq!(changed.basis, WorkspaceVersionBasis::BoundedTree);
    }

    #[cfg(unix)]
    #[test]
    fn git_version_hashes_untracked_symlink_target_without_following_it() {
        let repo = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success());
        std::fs::write(repo.path().join("tracked"), "baseline").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "tracked"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "baseline",
            ])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success());
        std::os::unix::fs::symlink("/definitely/not/read", repo.path().join("link")).unwrap();
        let version = capture(repo.path()).unwrap();
        assert_eq!(version.basis, WorkspaceVersionBasis::GitWorktree);
        assert_eq!(version.changed_paths, vec!["link"]);
    }

    #[test]
    fn ignored_nested_workspace_uses_bounded_tree_versioning() {
        let repo = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success());
        std::fs::write(repo.path().join(".gitignore"), ".scenario-runs/\n").unwrap();
        let workspace = repo.path().join(".scenario-runs/case");
        std::fs::create_dir_all(&workspace).unwrap();

        let before = capture(&workspace).unwrap();
        std::fs::write(workspace.join("artifact.txt"), "evidence").unwrap();
        let after = capture(&workspace).unwrap();

        assert_eq!(before.basis, WorkspaceVersionBasis::BoundedTree);
        assert_eq!(after.basis, WorkspaceVersionBasis::BoundedTree);
        assert_ne!(before.digest, after.digest);
        assert_eq!(after.changed_paths, vec!["artifact.txt"]);
    }
}
